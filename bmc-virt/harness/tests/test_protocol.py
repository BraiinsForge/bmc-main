"""Unit tests for the JSONL wire protocol — no VM required."""

import json

from bmc_virt.commands import Cmd
from bmc_virt.events import Event
from bmc_virt.protocol import (
    PROTOCOL_VERSION,
    Msg,
    MsgType,
)
from bmc_virt.protocol import ack as mk_ack
from bmc_virt.protocol import cmd as mk_cmd
from bmc_virt.protocol import event as mk_event
from bmc_virt.protocol import hello as mk_hello
from bmc_virt.protocol import shutdown as mk_shutdown
from bmc_virt.protocol import synced as mk_synced

# ── Message construction ───────────────────────────────────────────────────────


class TestHello:
    def test_fields(self):
        msg = mk_hello()
        assert msg.type == MsgType.HELLO
        assert msg.version == PROTOCOL_VERSION
        assert msg.ts is not None

    def test_roundtrip(self):
        msg = mk_hello()
        line = msg.to_line()
        parsed = Msg.from_line(line)
        assert parsed.type == MsgType.HELLO
        assert parsed.version == PROTOCOL_VERSION


class TestSynced:
    def test_fields(self):
        msg = mk_synced()
        assert msg.type == MsgType.SYNCED
        assert msg.ts is not None


class TestEvent:
    def test_without_data(self):
        msg = mk_event(Event.APP_READY)
        assert msg.type == MsgType.EVENT
        assert msg.name == "app.ready"
        assert msg.data == {}

    def test_with_data(self):
        msg = mk_event(Event.APP_STARTED, {"pid": 1234})
        assert msg.data == {"pid": 1234}

    def test_roundtrip(self):
        msg = mk_event(Event.WIFI_GOT_IP, {"iface": "wlan0", "ip": "192.168.1.100"})
        parsed = Msg.from_line(msg.to_line())
        assert parsed.name == "wifi.got_ip"
        assert parsed.data["ip"] == "192.168.1.100"


class TestCmd:
    def test_fields(self):
        msg = mk_cmd("req-1", Cmd.SHELL_EXEC, {"cmd": "echo hello"})
        assert msg.type == MsgType.CMD
        assert msg.id == "req-1"
        assert msg.name == "shell.exec"
        assert msg.data == {"cmd": "echo hello"}

    def test_roundtrip(self):
        msg = mk_cmd("req-2", Cmd.SERVICE_RESTART, {"name": "bmc-openwrt"})
        parsed = Msg.from_line(msg.to_line())
        assert parsed.id == "req-2"
        assert parsed.name == "service.restart"
        assert parsed.data["name"] == "bmc-openwrt"


class TestAck:
    def test_success(self):
        msg = mk_ack("req-1", ok=True, data={"exit_code": 0, "stdout": "hello\n"})
        assert msg.ok is True
        assert msg.error is None

    def test_failure(self):
        msg = mk_ack("req-1", ok=False, error="command timed out")
        assert msg.ok is False
        assert msg.error == "command timed out"

    def test_roundtrip(self):
        msg = mk_ack("req-3", ok=True, data={"exit_code": 0})
        parsed = Msg.from_line(msg.to_line())
        assert parsed.ok is True
        assert parsed.id == "req-3"
        assert parsed.data["exit_code"] == 0


class TestShutdown:
    def test_fields(self):
        msg = mk_shutdown("daemon stopping")
        assert msg.type == MsgType.SHUTDOWN
        assert msg.reason == "daemon stopping"


# ── Wire format ────────────────────────────────────────────────────────────────


class TestWireFormat:
    def test_newline_terminated(self):
        msg = mk_hello()
        line = msg.to_line()
        assert line.endswith(b"\n")

    def test_single_line(self):
        msg = mk_event(Event.APP_READY, {"some": "data"})
        line = msg.to_line()
        assert line.count(b"\n") == 1

    def test_valid_json(self):
        msg = mk_event(Event.RELAY_LISTENING)
        line = msg.to_line()
        parsed = json.loads(line)
        assert parsed["type"] == "event"
        assert parsed["name"] == "relay.listening"

    def test_compact_encoding(self):
        """Wire format uses compact JSON (no extra spaces)."""
        msg = mk_event(Event.APP_READY)
        line = msg.to_line().decode()
        assert ": " not in line
        assert ", " not in line

    def test_omits_none_fields(self):
        """None-valued fields are not included in the wire format."""
        msg = mk_event(Event.APP_READY)
        parsed = json.loads(msg.to_line())
        assert "id" not in parsed
        assert "version" not in parsed
        assert "error" not in parsed
        assert "reason" not in parsed

    def test_omits_empty_data(self):
        """Empty data dict is not included in the wire format."""
        msg = mk_hello()
        parsed = json.loads(msg.to_line())
        assert "data" not in parsed


# ── Type enums ─────────────────────────────────────────────────────────────────


class TestEventEnum:
    def test_all_events_are_dotted(self):
        for e in Event:
            assert "." in e.value or e.value == "shutdown"

    def test_string_value(self):
        assert Event.APP_READY == "app.ready"
        assert str(Event.APP_READY) == "app.ready"

    def test_from_string(self):
        assert Event("app.ready") is Event.APP_READY


class TestCmdEnum:
    def test_string_value(self):
        assert Cmd.SHELL_EXEC == "shell.exec"

    def test_from_string(self):
        assert Cmd("service.restart") is Cmd.SERVICE_RESTART
