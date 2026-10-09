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

"""Unit tests for the deploy stage that undoes earlier server cleanups."""

import json
import subprocess
from collections.abc import Callable, Iterable

import pytest

from bmc_tui import catalog
from bmc_tui.device import Device
from bmc_tui.stage import dry_run

_FACTORY = {
    "id": "factory",
    "base_url": "https://f",
    "known_public_key": "k",
    "priority": 0,
    "enabled": True,
}
_CLEARED = json.dumps({"factory": _FACTORY, "servers": []})
_REGISTERED = json.dumps(
    {"factory": _FACTORY, "servers": [{"id": "dev", "index_url": "https://i"}]}
)
_DEFAULT_ENTRY = {
    "id": "braiins",
    "index_url": "https://i",
    "known_public_key": "k",
    "priority": 50,
    "enabled": True,
}
_DEFAULT = json.dumps({"factory": _FACTORY, "servers": [_DEFAULT_ENTRY]})


class _Ssh:
    """Fake device holding `servers.json` and the shipped default, empty when absent."""

    def __init__(self, servers_json: str, *, default: str = _DEFAULT) -> None:
        self._servers_json = servers_json
        self._default = default
        self.commands: list[str] = []

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        command = argv[-1]
        self.commands.append(command)
        stdout = ""
        if "servers.json.default" in command:
            stdout = self._default
        elif "cat " in command:
            stdout = self._servers_json
        return subprocess.CompletedProcess(argv, 0, stdout, "")

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        pytest.fail("the stage never streams to the device")

    def stream_output(self, argv: list[str], on_line: Callable[[str], None]) -> int:
        pytest.fail("the stage never streams from the device")

    def removed(self) -> bool:
        return any(c.startswith("rm ") for c in self.commands)


def _restore(ssh: _Ssh) -> None:
    catalog.restore_cleared_servers(Device("h", backend=ssh))


def test_an_emptied_registry_is_removed_so_the_default_applies() -> None:
    ssh = _Ssh(_CLEARED)
    _restore(ssh)
    assert "rm /etc/nix-upgrade/servers.json" in ssh.commands


def test_registered_servers_are_kept() -> None:
    ssh = _Ssh(_REGISTERED)
    _restore(ssh)
    assert not ssh.removed(), "a developer's own servers must survive a deploy"


def test_a_missing_registry_is_left_alone() -> None:
    ssh = _Ssh("")
    _restore(ssh)
    assert not ssh.removed()


def test_an_unparsable_registry_is_left_to_bmc_nix_quarantine() -> None:
    ssh = _Ssh("{not json")
    _restore(ssh)
    assert not ssh.removed()


def test_an_emptied_registry_without_a_default_is_kept() -> None:
    ssh = _Ssh(_CLEARED, default="")
    _restore(ssh)
    assert not ssh.removed(), "without a default, removing it would lose the factory entry too"


def test_dry_run_removes_nothing(capsys: pytest.CaptureFixture[str]) -> None:
    ssh = _Ssh(_CLEARED)
    token = dry_run.set(True)
    try:
        _restore(ssh)
    finally:
        dry_run.reset(token)
    assert not ssh.removed()
    assert "would remove" in capsys.readouterr().out, "a dry run must not claim the file is gone"


@pytest.mark.parametrize(
    "default",
    [
        pytest.param("{not json", id="unparsable"),
        pytest.param(
            json.dumps({"factory": _FACTORY, "servers": [{"id": "braiins"}]}), id="invalid-entry"
        ),
        pytest.param(
            json.dumps({"factory": _FACTORY, "servers": [{**_DEFAULT_ENTRY, "enabled": False}]}),
            id="disabled",
        ),
    ],
)
def test_an_emptied_registry_without_a_usable_default_is_kept(default: str) -> None:
    ssh = _Ssh(_CLEARED, default=default)
    _restore(ssh)
    assert not ssh.removed(), "a default with no server to probe would lose the factory entry too"


def test_an_emptied_registry_with_its_own_factory_is_kept() -> None:
    ssh = _Ssh(
        _CLEARED,
        default=json.dumps(
            {"factory": {**_FACTORY, "known_public_key": "other"}, "servers": [_DEFAULT_ENTRY]}
        ),
    )
    _restore(ssh)
    assert not ssh.removed(), "falling back would replace the key that verifies firmware inits"


def test_an_emptied_registry_with_a_bootstrapped_factory_is_kept() -> None:
    ssh = _Ssh(json.dumps({"factory": _FACTORY, "servers": [], "bootstrapped_factory": True}))
    _restore(ssh)
    assert not ssh.removed(), "without the marker, re-registering the factory id is refused"
