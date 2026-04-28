"""High-level VM handle — the main entry point for test scripts."""

from __future__ import annotations

import threading
import time
import uuid
from datetime import UTC, datetime
from typing import TYPE_CHECKING, Any

from rich.live import Live
from rich.text import Text

from bmc_virt import ui
from bmc_virt.client import Client, DaemonConnectionError
from bmc_virt.commands import Ack, Cmd
from bmc_virt.events import Event, ReceivedEvent
from bmc_virt.metrics import MetricsCollector as _MetricsCollector
from bmc_virt.protocol import Msg, MsgType
from bmc_virt.protocol import cmd as _cmd_factory
from bmc_virt.rr import RrHandle
from bmc_virt.screenshot import capture as _capture_frame
from bmc_virt.ssh import Ssh

if TYPE_CHECKING:
    import subprocess
    from collections.abc import Callable, Iterator
    from pathlib import Path

    from bmc_virt.metrics import MetricsCollector

# ── Default connection parameters ──────────────────────────────────────────────

DEFAULT_HOST = "localhost"
DEFAULT_EVENT_PORT = 5920


class WaitTimeoutError(Exception):
    """Raised when a wait_for / exec times out."""


class ServiceHandle:
    """Bound handle for a named init.d service — vm.service("name").restart()."""

    def __init__(self, vm: VM, name: str) -> None:
        self._vm = vm
        self._name = name

    def restart(self, *, timeout: float = 30, verbose: bool = False) -> Ack:
        """Restart the service and return the ack."""
        return self._vm._send_cmd(
            Cmd.SERVICE_RESTART, timeout=timeout, verbose=verbose, name=self._name
        )


class _MetricsFactory:
    """Factory for creating MetricsCollector instances — accessed via vm.metrics."""

    def __init__(self, vm: VM) -> None:
        self._vm = vm

    def start(
        self,
        label: str = "",
        interval: float | None = None,
        *,
        processes: list[str] | None = None,
    ) -> MetricsCollector:
        """Start a new metrics collector.

        Without interval: purely imperative, each capture() polls once.
        With interval (seconds): background thread polls automatically.
        Pass ``processes=[...]`` to also sample per-process VmRSS/RssShmem.
        """
        return _MetricsCollector(self._vm, label, interval=interval, processes=processes)


class VM:
    """Connection to a running BMC virtual machine.

    Provides event waiting, command execution, SSH, and file transfer.
    """

    def __init__(self, client: Client, ssh: Ssh) -> None:
        self._client = client
        self._ssh = ssh
        self._history: list[ReceivedEvent] = []
        self._history_lock = threading.Lock()
        self._new_event = threading.Condition()
        self._synced = threading.Event()
        self._pending_acks: dict[str, threading.Event] = {}
        self._ack_results: dict[str, Msg] = {}

    @classmethod
    def connect(
        cls,
        *,
        host: str = DEFAULT_HOST,
        event_port: int = DEFAULT_EVENT_PORT,
        ssh_port: int = 2222,
        retry_interval: float = 0.5,
        timeout: float = 120,
    ) -> VM:
        """Connect to the VM's event daemon, retrying until available."""
        ssh = Ssh(host=host, port=ssh_port)
        start = time.monotonic()
        deadline = start + timeout
        last_exc: Exception | None = None

        with Live(console=ui.console, transient=True) as live:
            while time.monotonic() < deadline:
                elapsed = time.monotonic() - start
                remaining = timeout - elapsed
                bar_width = 30
                frac = elapsed / timeout
                filled = int(bar_width * frac)
                bar = "█" * filled + "░" * (bar_width - filled)
                status = Text.assemble(
                    ("  ", ""),
                    (f"{bar} ", "dim"),
                    (f"{elapsed:.0f}s", "bold"),
                    (f" / {timeout:.0f}s ", "dim"),
                    (f"({remaining:.0f}s left) ", "dim yellow"),
                    (f"{host}:{event_port}", "cyan"),
                )
                live.update(status)

                try:
                    vm = cls.__new__(cls)
                    vm._ssh = ssh
                    vm._history = []
                    vm._history_lock = threading.Lock()
                    vm._new_event = threading.Condition()
                    vm._synced = threading.Event()
                    vm._pending_acks = {}
                    vm._ack_results = {}
                    vm._client = Client.connect(host, event_port, vm._on_message)

                    # Wait for the daemon to finish replaying the backlog.
                    # Until `synced` arrives, vm.history is incomplete and
                    # wait_for() would race the reader thread.
                    sync_remaining = deadline - time.monotonic()
                    if sync_remaining <= 0 or not vm._synced.wait(timeout=sync_remaining):
                        vm._client.close()
                        msg = f"Connected but did not receive synced within {timeout}s"
                        raise WaitTimeoutError(msg)

                    return vm
                except DaemonConnectionError as exc:
                    last_exc = exc
                    time.sleep(retry_interval)

        msg = f"Could not connect to event daemon at {host}:{event_port} within {timeout}s"
        raise WaitTimeoutError(msg) from last_exc

    # ── Events ─────────────────────────────────────────────────────────────────

    def wait_for(
        self,
        event: Event,
        *,
        timeout: float = 30,
        where: Callable[[ReceivedEvent], bool] | None = None,
    ) -> ReceivedEvent:
        """Wait for an event, checking backlog first.

        Returns immediately if the event already occurred in history.
        """
        # Check history first
        with self._history_lock:
            for e in self._history:
                if e.name == event and (where is None or where(e)):
                    return e

        # Wait for live event
        deadline = time.monotonic() + timeout
        with self._new_event:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise WaitTimeoutError(f"Timed out waiting for {event}")

                # Re-check history under lock (new events may have arrived)
                with self._history_lock:
                    for e in self._history:
                        if e.name == event and (where is None or where(e)):
                            return e

                self._new_event.wait(timeout=min(remaining, 0.5))

    def wait_next(
        self,
        event: Event,
        *,
        timeout: float = 30,
        where: Callable[[ReceivedEvent], bool] | None = None,
    ) -> ReceivedEvent:
        """Wait for the next live occurrence, ignoring backlog."""
        marker = len(self._history)
        deadline = time.monotonic() + timeout
        with self._new_event:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise WaitTimeoutError(f"Timed out waiting for next {event}")

                with self._history_lock:
                    for e in self._history[marker:]:
                        if e.name == event and (where is None or where(e)):
                            return e

                self._new_event.wait(timeout=min(remaining, 0.5))

    def events(self) -> Iterator[ReceivedEvent]:
        """Iterate over live events as they arrive (blocking)."""
        seen = 0
        while True:
            with self._new_event:
                while seen >= len(self._history):
                    self._new_event.wait(timeout=1.0)
                with self._history_lock:
                    batch = self._history[seen:]
                    seen = len(self._history)
            yield from batch

    def stream(self, duration: float = 5) -> None:
        """Print live events for a fixed duration, then return."""
        stop = threading.Event()

        def _print_events() -> None:
            for evt in self.events():
                if stop.is_set():
                    break
                ui.event(evt)

        t = threading.Thread(target=_print_events, daemon=True)
        t.start()
        # Show a spinner with remaining time while streaming
        with ui.out.status(f"[dim]streaming for {duration:.0f}s...[/dim]", spinner="dots"):
            time.sleep(duration)
        stop.set()
        t.join(timeout=1)

    @property
    def history(self) -> list[ReceivedEvent]:
        """All events received so far (snapshot)."""
        with self._history_lock:
            return list(self._history)

    # ── Shell execution ────────────────────────────────────────────────────────

    def exec(self, cmd: str, *, timeout: float = 30, verbose: bool = False) -> Ack:
        """Execute a shell command on the VM via the event daemon."""
        return self._send_cmd(Cmd.SHELL_EXEC, timeout=timeout, verbose=verbose, cmd=cmd)

    # ── Service management ─────────────────────────────────────────────────────

    def service(self, name: str) -> ServiceHandle:
        """Get a handle for a named init.d service."""
        return ServiceHandle(self, name)

    # ── Screenshot ──────────────────────────────────────────────────────────────

    def screenshot(self, path: str | Path, *, relay_port: int = 5910) -> Path:
        """Capture a screenshot from the relay daemon and save as PNG."""
        frame = _capture_frame(host=self._ssh._host, port=relay_port)
        return frame.save_png(path)

    # ── rr debugger ─────────────────────────────────────────────────────────────

    @property
    def rr(self) -> RrHandle:
        """Access rr time-travel debugger."""
        return RrHandle(self)

    # ── Metrics ────────────────────────────────────────────────────────────────

    @property
    def metrics(self) -> _MetricsFactory:
        """Access metrics collection."""
        return _MetricsFactory(self)

    # ── SSH / file transfer ────────────────────────────────────────────────────

    def ssh(
        self, command: str, *, timeout: float = 30, check: bool = True
    ) -> subprocess.CompletedProcess[str]:
        """Run a command on the VM via SSH."""
        return self._ssh.run(command, timeout=timeout, check=check)

    def pull(self, *, src: str, dst: str | Path) -> None:
        """Copy a file from the VM to the host."""
        self._ssh.pull(remote=src, local=dst)

    def push(self, *, src: str | Path, dst: str) -> None:
        """Copy a file from the host to the VM."""
        self._ssh.push(local=src, remote=dst)

    # ── Lifecycle ──────────────────────────────────────────────────────────────

    def close(self) -> None:
        """Disconnect from the event daemon."""
        self._client.close()

    def __enter__(self) -> VM:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    # ── Internal ───────────────────────────────────────────────────────────────

    def _send_cmd(self, command: Cmd, *, timeout: float, verbose: bool, **data: Any) -> Ack:
        """Send a command to the daemon and wait for its ack."""
        if verbose:
            ui.cmd_sent(command, **data)

        request_id = uuid.uuid4().hex[:8]
        done = threading.Event()
        self._pending_acks[request_id] = done

        spinner = ui.out.status("[dim]running...[/dim]", spinner="dots") if verbose else None
        try:
            if spinner:
                spinner.start()
            self._client.send(_cmd_factory(request_id, command, data or None))
            if not done.wait(timeout=timeout):
                raise WaitTimeoutError(f"Timed out waiting for ack of {command} (id={request_id})")
            msg = self._ack_results.pop(request_id)
            result = Ack(
                id=request_id,
                ok=msg.ok or False,
                data=msg.data,
                error=msg.error,
            )
        finally:
            if spinner:
                spinner.stop()
            self._pending_acks.pop(request_id, None)

        if verbose:
            ui.ack_result(result)
        return result

    def _on_message(self, msg: Msg) -> None:
        """Callback from the Client reader thread."""
        if msg.type == MsgType.EVENT:
            evt = ReceivedEvent(
                name=Event(msg.name),
                ts=datetime.fromisoformat(msg.ts) if msg.ts else datetime.now(UTC),
                data=msg.data,
            )
            with self._history_lock:
                self._history.append(evt)
            with self._new_event:
                self._new_event.notify_all()

        elif msg.type == MsgType.SYNCED:
            self._synced.set()

        elif msg.type == MsgType.ACK and msg.id:
            done = self._pending_acks.get(msg.id)
            if done:
                self._ack_results[msg.id] = msg
                done.set()
