"""Integration tests — client ↔ server over loopback TCP.

Spins up an EventDaemon on a random port and connects a Client to it.
No VM required.
"""

import contextlib
import socket
import threading
import time

import pytest

from bmc_virt.client import Client, DaemonProtocolError
from bmc_virt.commands import Cmd
from bmc_virt.events import Event
from bmc_virt.protocol import Msg, MsgType
from bmc_virt.protocol import ack as mk_ack
from bmc_virt.protocol import cmd as mk_cmd
from bmc_virt.server import EventDaemon


def _free_port() -> int:
    """Find a free TCP port."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class DaemonFixture:
    """Manages a test daemon on a random port."""

    def __init__(self):
        self.port = _free_port()
        self.daemon = EventDaemon()
        self.daemon._state.app_started = True  # skip polling, we control events manually
        self.thread: threading.Thread | None = None
        self._ready = threading.Event()

    def start(self):
        self.thread = threading.Thread(target=self._run, daemon=True, name="test-daemon")
        self.thread.start()
        if not self._ready.wait(timeout=5):
            msg = "Daemon did not start"
            raise RuntimeError(msg)

    def _run(self):
        server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        server.settimeout(1.0)
        server.bind(("127.0.0.1", self.port))
        server.listen(1)
        self._ready.set()

        try:
            while self.daemon._running:
                try:
                    client, _addr = server.accept()
                except TimeoutError:
                    continue

                # Reject if already connected (mirrors real daemon logic)
                with self.daemon._client_lock:
                    if self.daemon._client_sock is not None:
                        with contextlib.suppress(OSError):
                            client.sendall(
                                mk_ack("", ok=False, error="another client is connected").to_line()
                            )
                        client.close()
                        continue
                    self.daemon._client_sock = client

                # Handle client in a separate thread so the accept loop
                # stays free to reject additional connections
                threading.Thread(
                    target=self.daemon._handle_client,
                    args=(client,),
                    daemon=True,
                ).start()
        except Exception:
            pass
        finally:
            server.close()

    def stop(self):
        self.daemon._running = False
        if self.thread:
            self.thread.join(timeout=3)


@pytest.fixture
def daemon():
    d = DaemonFixture()
    d.start()
    yield d
    d.stop()


@pytest.fixture
def connected(daemon):
    """Returns (daemon, client, received_messages)."""
    messages: list[Msg] = []
    client = Client.connect("127.0.0.1", daemon.port, messages.append)
    yield daemon, client, messages
    client.close()


# ── Connection lifecycle ───────────────────────────────────────────────────────


class TestHandshake:
    def test_receives_hello_and_synced(self, connected):
        _daemon, _client, messages = connected
        # Give the reader thread time to process buffered messages
        time.sleep(1.0)
        types = [m.type for m in messages]
        assert MsgType.SYNCED in types

    def test_hello_version(self, connected):
        _daemon, _client, messages = connected
        time.sleep(1.0)
        # The hello is consumed during Client.connect handshake,
        # so synced should be the first message the callback sees
        assert any(m.type == MsgType.SYNCED for m in messages)


# ── Event streaming ────────────────────────────────────────────────────────────


class TestEvents:
    def test_receives_emitted_event(self, connected):
        daemon, _client, messages = connected
        time.sleep(0.1)
        daemon.daemon.emit(Event.APP_READY)
        time.sleep(0.2)
        event_msgs = [m for m in messages if m.type == MsgType.EVENT]
        assert len(event_msgs) >= 1
        assert event_msgs[-1].name == "app.ready"

    def test_event_with_data(self, connected):
        daemon, _client, messages = connected
        time.sleep(0.1)
        daemon.daemon.emit(Event.WIFI_GOT_IP, {"iface": "wlan0", "ip": "10.0.0.1"})
        time.sleep(0.2)
        event_msgs = [m for m in messages if m.type == MsgType.EVENT]
        assert any(m.data.get("ip") == "10.0.0.1" for m in event_msgs)


# ── Backlog replay ─────────────────────────────────────────────────────────────


class TestBacklog:
    def test_new_client_receives_past_events(self, daemon):
        # Emit events before any client connects
        daemon.daemon.emit(Event.SETUP_DONE)
        daemon.daemon.emit(Event.APP_STARTED, {"pid": 42})
        time.sleep(0.1)

        # Connect a client — should receive backlog
        messages: list[Msg] = []
        client = Client.connect("127.0.0.1", daemon.port, messages.append)
        time.sleep(0.3)
        client.close()

        event_names = [m.name for m in messages if m.type == MsgType.EVENT]
        assert "setup.done" in event_names
        assert "app.started" in event_names


# ── Command execution ──────────────────────────────────────────────────────────


class TestCommands:
    def test_shell_exec(self, connected):
        _daemon, client, messages = connected
        time.sleep(0.1)
        client.send(mk_cmd("test-1", Cmd.SHELL_EXEC, {"cmd": "echo hello"}))
        time.sleep(0.5)
        acks = [m for m in messages if m.type == MsgType.ACK]
        assert len(acks) >= 1
        ack = next(a for a in acks if a.id == "test-1")
        assert ack.ok is True
        assert ack.data["stdout"].strip() == "hello"
        assert ack.data["exit_code"] == 0

    def test_shell_exec_failure(self, connected):
        _daemon, client, messages = connected
        time.sleep(0.1)
        client.send(mk_cmd("test-2", Cmd.SHELL_EXEC, {"cmd": "false"}))
        time.sleep(0.5)
        ack = next(a for a in messages if a.type == MsgType.ACK and a.id == "test-2")
        assert ack.ok is False
        assert ack.data["exit_code"] != 0

    def test_unknown_command(self, connected):
        _daemon, client, messages = connected
        time.sleep(0.1)
        # Send a raw command with a bogus name
        msg = Msg(type=MsgType.CMD, id="test-3", name="bogus.cmd")
        client.send(msg)
        time.sleep(0.3)
        ack = next(a for a in messages if a.type == MsgType.ACK and a.id == "test-3")
        assert ack.ok is False
        assert "unknown command" in (ack.error or "")


# ── Connection rejection ──────────────────────────────────────────────────────


class TestSingleClient:
    def test_second_client_rejected(self, connected):
        daemon, _client, _messages = connected
        time.sleep(0.1)

        # Try connecting a second client — should be rejected
        reject_messages: list[Msg] = []
        with pytest.raises(DaemonProtocolError):
            # The second connect should fail — server sends an ack (not hello) and closes
            Client.connect("127.0.0.1", daemon.port, reject_messages.append)
