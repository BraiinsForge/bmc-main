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

"""Unit tests for the deploy stage that keeps automatic upgrades away."""

import json
import shlex
import subprocess
import time
from collections.abc import Callable, Generator, Iterable
from contextlib import contextmanager

import pytest

from bmc_tui import catalog, json_value
from bmc_tui.boser_rest import BoserRest
from bmc_tui.device import Device
from bmc_tui.stage import Abort, dry_run
from tests import stubs

_SESSION = "0123abcd"
_EXIT_DIAL_FAILED = 1
_EXIT_DEADLINE_EXCEEDED = 64 + 4
_EXIT_UNAVAILABLE = 64 + 14
_EXIT_UNAUTHENTICATED = 64 + 16
_FORWARDED = "127.0.0.1:4321"


@contextmanager
def _forward(_dev: Device, remote_port: int) -> Generator[str]:
    assert remote_port == 80, "the API is on the plain web port"
    yield _FORWARDED


class _Ssh:
    """Fake device transport answering ubus session calls."""

    def __init__(self) -> None:
        self.commands: list[str] = []

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        command = argv[-1]
        self.commands.append(command)
        stdout = json.dumps({"ubus_rpc_session": _SESSION}) if "session create" in command else ""
        return subprocess.CompletedProcess(argv, 0, stdout, "")

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        pytest.fail("the stage never streams to the device")

    def stream_output(self, argv: list[str], on_line: Callable[[str], None]) -> int:
        pytest.fail("the stage never streams from the device")

    def ran(self, fragment: str) -> bool:
        return any(fragment in c for c in self.commands)


class _Grpcurl:
    """Fake grpcurl answering each call with the next exit code, or hanging on None."""

    def __init__(self, *codes: int | None) -> None:
        self._codes = list(codes)
        self.calls: list[list[str]] = []

    def __call__(self, argv: list[str], **_kwargs: object) -> "subprocess.CompletedProcess[str]":
        self.calls.append(argv)
        code = self._codes.pop(0)
        if code is None:
            raise subprocess.TimeoutExpired(argv, 0)
        return subprocess.CompletedProcess(argv, code, "{}", "" if code == 0 else "ERROR")


class _Boser:
    """Records the REST calls the stage makes to Boser."""

    def __init__(self, failure: Abort | None = None) -> None:
        self._failure = failure
        self.calls: list[tuple[str, str | None, bool]] = []

    def __call__(self, client: BoserRest, *, enabled: bool) -> None:
        self.calls.append((client.base_url, client.token, enabled))
        if self._failure is not None:
            raise self._failure


def _patch(monkeypatch: pytest.MonkeyPatch, grpcurl: _Grpcurl, boser: _Boser) -> None:
    monkeypatch.setattr(subprocess, "run", grpcurl)

    def set_auto_upgrade(client: BoserRest, *, enabled: bool) -> None:
        boser(client, enabled=enabled)

    monkeypatch.setattr(time, "sleep", stubs.returning(None))
    monkeypatch.setattr(Device, "forward", _forward)
    monkeypatch.setattr(BoserRest, "set_auto_upgrade", set_auto_upgrade)


def _run(
    monkeypatch: pytest.MonkeyPatch,
    grpcurl: _Grpcurl,
    ssh: _Ssh | None = None,
    boser: _Boser | None = None,
    plan: catalog.Deployment | None = None,
) -> tuple[_Ssh, _Boser]:
    ssh = ssh or _Ssh()
    boser = boser or _Boser()
    _patch(monkeypatch, grpcurl, boser)
    catalog.disable_auto_upgrade(Device("h", backend=ssh), plan or catalog.Deployment(attrs=[]))
    return ssh, boser


def _run_deferred(
    monkeypatch: pytest.MonkeyPatch, grpcurl: _Grpcurl, plan: catalog.Deployment
) -> _Ssh:
    ssh = _Ssh()
    _patch(monkeypatch, grpcurl, _Boser())
    catalog.disable_deferred_auto_upgrade(Device("h", backend=ssh), plan)
    return ssh


def test_automatic_upgrades_are_disabled_through_the_ssh_made_session(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    grpcurl = _Grpcurl(0)
    ssh, boser = _run(monkeypatch, grpcurl)

    argv = grpcurl.calls[0]
    assert argv[-2:] == [_FORWARDED, "braiins.bmc.web.UpgradeService/SetAutoUpgrade"], (
        "deploy reaches the API through SSH, wherever SSH reaches the device"
    )
    assert json.loads(argv[argv.index("-d") + 1]) == {"enabled": False}
    assert f"cookie: session_id={_SESSION}" in argv, "the call must ride the SSH-made session"
    assert boser.calls == [], "only products Boser manages hand the setting to Boser"
    assert ssh.ran("session destroy")


@pytest.mark.parametrize(
    "code",
    [
        pytest.param(_EXIT_DIAL_FAILED, id="dial-failed"),
        pytest.param(_EXIT_DEADLINE_EXCEEDED, id="deadline-exceeded"),
        pytest.param(_EXIT_UNAVAILABLE, id="unavailable"),
        pytest.param(None, id="stalled"),
    ],
)
def test_a_bmc_that_does_not_answer_defers_the_disable(
    monkeypatch: pytest.MonkeyPatch, code: int | None
) -> None:
    plan = catalog.Deployment(attrs=[])
    ssh, _ = _run(monkeypatch, _Grpcurl(code), plan=plan)
    assert not plan.auto_upgrade_disabled, "a deploy that fixes a crashing bmc must not abort here"
    assert ssh.ran("session destroy")


def test_a_bmc_refusing_the_call_fails_the_deploy(monkeypatch: pytest.MonkeyPatch) -> None:
    with pytest.raises(Abort, match="SetAutoUpgrade failed"):
        _run(monkeypatch, _Grpcurl(_EXIT_UNAUTHENTICATED))


def test_a_disable_that_succeeded_is_not_repeated(monkeypatch: pytest.MonkeyPatch) -> None:
    plan = catalog.Deployment(attrs=[])
    _run(monkeypatch, _Grpcurl(0), plan=plan)
    grpcurl = _Grpcurl()
    _run_deferred(monkeypatch, grpcurl, plan)
    assert grpcurl.calls == []


def test_a_deferred_disable_waits_for_the_deployed_bmc(monkeypatch: pytest.MonkeyPatch) -> None:
    plan = catalog.Deployment(attrs=[])
    grpcurl = _Grpcurl(_EXIT_DIAL_FAILED, _EXIT_UNAVAILABLE, 0)
    _run_deferred(monkeypatch, grpcurl, plan)
    assert len(grpcurl.calls) == 3, "a restarted bmc takes a while to answer"
    assert plan.auto_upgrade_disabled


def test_a_deferred_attempt_cannot_outlast_the_deadline(monkeypatch: pytest.MonkeyPatch) -> None:
    grpcurl = _Grpcurl(0)
    _run_deferred(monkeypatch, grpcurl, catalog.Deployment(attrs=[]))
    argv = grpcurl.calls[0]
    max_time = int(argv[argv.index("-max-time") + 1])
    assert max_time <= catalog._BMC_READY_TIMEOUT, (
        "a stalled bmc must not hold deploy past the wait"
    )


def test_a_deployed_bmc_that_never_answers_fails_the_deploy(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(catalog, "_BMC_READY_TIMEOUT", 0.0)
    with pytest.raises(Abort, match="automatic upgrades are still on"):
        _run_deferred(monkeypatch, _Grpcurl(_EXIT_DIAL_FAILED), catalog.Deployment(attrs=[]))


def test_the_session_outlives_the_grpc_call_and_the_boser_fallback(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    ssh, _ = _run(monkeypatch, _Grpcurl(catalog._GRPCURL_EXIT_UNIMPLEMENTED))

    [create] = [c for c in ssh.commands if "session create" in c]
    request = json_value.loads(shlex.split(create)[-1])
    assert isinstance(request, dict)
    timeout_s = request["timeout"]
    assert isinstance(timeout_s, int)
    assert timeout_s >= catalog._GRPCURL_MAX_TIME + BoserRest("").timeout, (
        "a session expiring before the Boser fallback would get it refused"
    )


def test_boser_managed_products_have_boser_disable_them(monkeypatch: pytest.MonkeyPatch) -> None:
    ssh, boser = _run(monkeypatch, _Grpcurl(catalog._GRPCURL_EXIT_UNIMPLEMENTED))

    assert boser.calls == [(f"http://{_FORWARDED}", _SESSION, False)], (
        "Boser takes the same ubus session, through the same tunnel"
    )
    assert not ssh.ran("clear-servers"), "manual upgrades need the servers left in place"


def test_a_boser_refusal_fails_the_deploy_and_destroys_the_session(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    ssh = _Ssh()
    refused = _Boser(Abort("PATCH /api/v1/upgrade/auto-upgrade: HTTP 401"))
    with pytest.raises(Abort, match="HTTP 401"):
        _run(monkeypatch, _Grpcurl(catalog._GRPCURL_EXIT_UNIMPLEMENTED), ssh, refused)
    assert ssh.ran("session destroy")


def test_dry_run_leaves_the_device_alone(monkeypatch: pytest.MonkeyPatch) -> None:
    plan = catalog.Deployment(attrs=[])
    token = dry_run.set(True)
    try:
        ssh, boser = _run(monkeypatch, _Grpcurl(), plan=plan)
    finally:
        dry_run.reset(token)
    assert ssh.commands == []
    assert boser.calls == []
    assert plan.auto_upgrade_disabled, "the stage after activation must not dial the device either"
