# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

"""Unit tests for SSH-authorized BMC application sessions."""

import json
import subprocess
from collections.abc import Callable, Iterable
from typing import final

import pytest

from bmc_tui.device import Device
from bmc_tui.stage import Abort
from bmc_tui.ubus_session import ssh_session

_CREATED = json.dumps({"ubus_rpc_session": "0123abcd", "timeout": 60, "expires": 60})


@final
class _Exec:
    def __init__(self, created: str) -> None:
        self._created = created
        self.commands: list[str] = []

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        command = argv[-1]
        self.commands.append(command)
        stdout = self._created if "session create" in command else ""
        return subprocess.CompletedProcess(argv, 0, stdout, "")

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        pytest.fail("a session never streams to the device")

    def stream_output(self, argv: list[str], on_line: Callable[[str], None]) -> int:
        pytest.fail("a session never streams from the device")


def _destroyed(backend: _Exec) -> list[str]:
    return [c for c in backend.commands if "session destroy" in c]


def test_the_created_session_is_yielded_and_then_destroyed() -> None:
    backend = _Exec(_CREATED)
    with ssh_session(Device("h", backend=backend), timeout_s=60) as session_id:
        assert session_id == "0123abcd"
        assert _destroyed(backend) == [], "the session must stay alive while in use"
    assert _destroyed(backend) == ['ubus call session destroy \'{"ubus_rpc_session": "0123abcd"}\'']


def test_a_failing_caller_still_destroys_the_session() -> None:
    backend = _Exec(_CREATED)
    with (
        pytest.raises(Abort, match="call failed"),
        ssh_session(Device("h", backend=backend), timeout_s=60),
    ):
        raise Abort("call failed")
    assert len(_destroyed(backend)) == 1, "a root-equivalent session must not outlive its use"


def test_the_requested_timeout_bounds_the_session() -> None:
    backend = _Exec(_CREATED)
    with ssh_session(Device("h", backend=backend), timeout_s=42):
        pass
    assert backend.commands[0] == "ubus call session create '{\"timeout\": 42}'"


def test_an_unexpected_ubus_reply_aborts() -> None:
    backend = _Exec("Command failed: Permission denied")
    with (
        pytest.raises(Abort, match="did not create a session"),
        ssh_session(Device("h", backend=backend), timeout_s=60),
    ):
        pytest.fail("no session id may be handed out without a session")
