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

"""Unit tests for the run's shared SSH connections."""

import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import final

import pytest

from bmc_tui import exits, ssh
from bmc_tui.ssh import ControlError, SharedConnections

# ssh's own words for a socket whose master was killed
_REFUSED = subprocess.CalledProcessError(255, ["ssh"], stderr="Connection refused")


@final
class _Runner:
    """Answers `-O` commands as ssh does, failing `failing` with `error`, to `login` if given.
    `exit` removes the socket, as a master that ends does, and so does `failing` with `ends`."""

    def __init__(
        self,
        *,
        failing: str | None = None,
        error: Exception | None = None,
        ends: bool = False,
        login: str | None = None,
    ) -> None:
        self.runs: list[list[str]] = []
        self._failing = failing
        self._error = error
        self._ends = ends
        self._login = login

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        self.runs.append(argv)
        command = argv[argv.index("-O") + 1]
        socket_path = Path(argv[2].removeprefix("ControlPath="))
        fails = command == self._failing and self._login in (None, argv[-1])
        if fails and self._error is not None:
            if self._ends:
                socket_path.unlink()
            raise self._error
        if command == "exit":
            socket_path.unlink()
        return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")


def _socket_path(connections: SharedConnections, login: str) -> Path:
    option = next(o for o in connections.options(login) if o.startswith("ControlPath="))
    return Path(option.removeprefix("ControlPath="))


def _master(connections: SharedConnections, login: str) -> socket.socket:
    """Listen where the login's master would."""
    master = socket.socket(socket.AF_UNIX)
    master.bind(str(_socket_path(connections, login)))
    master.listen()
    return master


def _sent(runner: _Runner, command: str) -> list[str]:
    return [argv[-1] for argv in runner.runs if argv[argv.index("-O") + 1] == command]


def test_each_login_gets_its_own_socket_and_keeps_it() -> None:
    shared = SharedConnections(_Runner())
    try:
        a = _socket_path(shared, "root@a")
        b = _socket_path(shared, "root@b")

        assert a != b
        assert _socket_path(shared, "root@a") == a
        assert a.parent == b.parent
    finally:
        shared.close()


def test_joining_a_connection_names_its_socket_and_never_becomes_its_master() -> None:
    shared = SharedConnections(_Runner())
    try:
        joining = shared.join_options("root@a")

        socket_option = f"ControlPath={_socket_path(shared, 'root@a')}"
        assert joining == ["-o", "ControlMaster=no", "-o", socket_option], (
            "an ssh that became a master would outlive its program, whatever the user's config says"
        )
    finally:
        shared.close()


def test_closing_ends_each_connection_and_removes_the_sockets() -> None:
    runner = _Runner()
    shared = SharedConnections(runner)
    with _master(shared, "root@a"), _master(shared, "root@b"):
        sockets = _socket_path(shared, "root@a").parent

        shared.close()

    assert _sent(runner, "exit") == ["root@a", "root@b"]
    assert not sockets.exists()


def test_a_connection_that_never_opened_is_not_ended() -> None:
    runner = _Runner()
    shared = SharedConnections(runner)
    _ = _socket_path(shared, "root@a")
    with _master(shared, "root@b"):
        shared.close()

    assert _sent(runner, "check") == ["root@b"], "ssh -O fails where no master keeps a socket"
    assert _sent(runner, "exit") == ["root@b"]


def test_a_socket_its_killed_master_left_behind_is_not_ended() -> None:
    runner = _Runner(failing="check", error=_REFUSED)
    shared = SharedConnections(runner)
    with _master(shared, "root@a"):
        sockets = _socket_path(shared, "root@a").parent

    shared.close()

    assert _sent(runner, "exit") == []
    assert not sockets.exists()


@pytest.mark.parametrize(
    ("failing", "error"),
    [
        ("exit", subprocess.CalledProcessError(255, ["ssh"], stderr="read from master failed")),
        ("check", subprocess.TimeoutExpired(["ssh"], 10)),
        ("check", FileNotFoundError("ssh")),
    ],
    ids=["broken", "stuck", "no ssh"],
)
def test_a_connection_that_does_not_end_keeps_its_socket_and_says_how_to_end_it(
    failing: str, error: Exception
) -> None:
    shared = SharedConnections(_Runner(failing=failing, error=error))
    path = _socket_path(shared, "root@a")
    try:
        with _master(shared, "root@a"), pytest.raises(RuntimeError) as raised:
            shared.close()

        assert f"ssh -O {failing}` to root@a failed" in str(raised.value)
        commands = str(raised.value).splitlines()[-2:]
        assert commands == [
            f"pkill -KILL -f '[/]{str(path.parent)[1:]}/'",
            f"rm -r '{path.parent}'",
        ]
        assert path.exists()
    finally:
        shutil.rmtree(path.parent)


def test_closing_again_after_a_connection_did_not_end_does_nothing() -> None:
    runner = _Runner(failing="exit", error=_REFUSED)
    shared = SharedConnections(runner)
    path = _socket_path(shared, "root@a")
    try:
        with _master(shared, "root@a"):
            with pytest.raises(RuntimeError):
                shared.close()
            sent = len(runner.runs)

            shared.close()

            assert len(runner.runs) == sent, "the first close already said how to end it"
    finally:
        shutil.rmtree(path.parent)


def test_closing_takes_one_that_ends_on_its_own_meanwhile_as_ended() -> None:
    shared = SharedConnections(_Runner(failing="exit", error=_REFUSED, ends=True))
    sockets = _socket_path(shared, "root@a").parent
    try:
        with _master(shared, "root@a"):
            shared.close()

        assert not sockets.exists()
    finally:
        shutil.rmtree(sockets, ignore_errors=True)


def test_closing_takes_one_killed_meanwhile_as_ended() -> None:
    shared = SharedConnections(_Runner(failing="exit", error=_REFUSED))
    sockets = _socket_path(shared, "root@a").parent
    try:
        with _master(shared, "root@a"):
            # Killed, it leaves its socket behind with nothing listening
            pass

        shared.close()

        assert not sockets.exists()
    finally:
        shutil.rmtree(sockets, ignore_errors=True)


def _command_line(pid: int) -> str:
    return subprocess.run(
        ["ps", "-o", "args=", "-p", str(pid)], capture_output=True, text=True, check=False
    ).stdout


@pytest.mark.skipif(shutil.which("pkill") is None, reason="needs pkill")
def test_the_way_to_end_a_stuck_connection_kills_its_master_not_the_shell() -> None:
    shared = SharedConnections(_Runner(failing="exit", error=_REFUSED))
    path = _socket_path(shared, "root@a")
    # Under the title a master takes on Linux.
    # Python rather than `sleep`: a multi-call coreutils picks its program by that title.
    stuck = subprocess.Popen(
        [f"ssh: {path} [mux]", "-c", "import time; time.sleep(60)"], executable=sys.executable
    )
    try:
        deadline = time.monotonic() + 5
        while "[mux]" not in _command_line(stuck.pid):
            assert time.monotonic() < deadline, "the stand-in never took the master's title"
            time.sleep(0.05)
        with _master(shared, "root@a"), pytest.raises(RuntimeError) as raised:
            shared.close()
        remedy = "\n".join(str(raised.value).splitlines()[-2:])

        ran = subprocess.run(["bash", "-c", remedy], check=False, timeout=10)

        assert stuck.wait(timeout=5) == -signal.SIGKILL
        assert ran.returncode == 0, "pkill must not match the shell carrying the pattern"
        assert not path.parent.exists()
    finally:
        stuck.kill()
        _ = stuck.wait()
        shutil.rmtree(path.parent, ignore_errors=True)


def test_ending_all_connections_ends_the_open_ones_and_keeps_their_directory() -> None:
    runner = _Runner()
    shared = SharedConnections(runner)
    try:
        sockets = _socket_path(shared, "root@a").parent
        with _master(shared, "root@b"):
            shared.end_all()

        assert _sent(runner, "exit") == ["root@b"], "ssh -O fails where no master keeps a socket"
        assert sockets.is_dir(), "the run's next ssh still needs it"
    finally:
        shared.close()


def test_ending_all_connections_ends_the_rest_and_names_the_one_that_does_not_end() -> None:
    runner = _Runner(failing="check", error=subprocess.TimeoutExpired(["ssh"], 10), login="root@a")
    shared = SharedConnections(runner)
    path = _socket_path(shared, "root@a")
    try:
        with (
            _master(shared, "root@a"),
            _master(shared, "root@b"),
            pytest.raises(ControlError) as raised,
        ):
            shared.end_all()

        assert _sent(runner, "exit") == ["root@b"], "a stuck connection must not keep the rest up"
        assert "to root@a failed" in str(raised.value)
        assert "root@b" not in str(raised.value)
    finally:
        shutil.rmtree(path.parent)


def test_ending_all_connections_names_each_one_that_does_not_end() -> None:
    shared = SharedConnections(
        _Runner(failing="check", error=subprocess.TimeoutExpired(["ssh"], 10))
    )
    path = _socket_path(shared, "root@a")
    try:
        with (
            _master(shared, "root@a"),
            _master(shared, "root@b"),
            pytest.raises(ControlError) as raised,
        ):
            shared.end_all()

        assert "to root@a failed" in str(raised.value)
        assert "to root@b failed" in str(raised.value)
    finally:
        shutil.rmtree(path.parent)


def test_ending_all_connections_takes_one_that_ends_on_its_own_meanwhile_as_ended() -> None:
    # A master going down with its device answers the check, then is gone for the exit
    shared = SharedConnections(_Runner(failing="exit", error=_REFUSED, ends=True))
    try:
        with _master(shared, "root@a"):
            shared.end_all()
    finally:
        shared.close()


def test_ending_all_connections_takes_one_killed_meanwhile_as_ended() -> None:
    shared = SharedConnections(_Runner(failing="exit", error=_REFUSED))
    path = _socket_path(shared, "root@a")
    try:
        with _master(shared, "root@a"):
            # Killed, it leaves its socket behind with nothing listening
            pass

        shared.end_all()
    finally:
        shutil.rmtree(path.parent)


def test_an_ssh_after_closing_is_a_bug() -> None:
    shared = SharedConnections(_Runner())
    shared.close()

    with pytest.raises(RuntimeError, match="BUG"):
        _ = shared.options("root@a")


def test_adding_a_forward_asks_the_login_s_connection() -> None:
    runner = _Runner()
    shared = SharedConnections(runner)
    try:
        path = _socket_path(shared, "root@a")

        shared.add_forward("root@a", "1:h:2")

        assert runner.runs == [
            ["ssh", "-o", f"ControlPath={path}", "-O", "forward", "-L", "1:h:2", "root@a"]
        ]
    finally:
        shared.close()


def test_a_forward_its_connection_refuses_fails() -> None:
    shared = SharedConnections(_Runner(failing="forward", error=_REFUSED))
    try:
        with pytest.raises(ControlError, match="ssh -O forward` to root@a failed"):
            shared.add_forward("root@a", "1:h:2")
    finally:
        shared.close()


def test_cancelling_a_forward_asks_its_connection() -> None:
    runner = _Runner()
    shared = SharedConnections(runner)
    try:
        with _master(shared, "root@a"):
            shared.cancel_forward("root@a", "1:h:2")

        assert runner.runs[-1][-4:] == ["cancel", "-L", "1:h:2", "root@a"]
    finally:
        shared.close()


def test_a_forward_whose_connection_ended_needs_no_cancelling() -> None:
    runner = _Runner()
    shared = SharedConnections(runner)
    try:
        _ = _socket_path(shared, "root@a")

        shared.cancel_forward("root@a", "1:h:2")

        assert runner.runs == []
    finally:
        shared.close()


def test_a_forward_whose_connection_ends_while_cancelling_needs_no_cancelling() -> None:
    shared = SharedConnections(_Runner(failing="cancel", error=_REFUSED, ends=True))
    try:
        with _master(shared, "root@a"):
            shared.cancel_forward("root@a", "1:h:2")
    finally:
        shared.close()


def test_a_forward_whose_master_is_killed_while_cancelling_needs_no_cancelling() -> None:
    shared = SharedConnections(_Runner(failing="cancel", error=_REFUSED))
    try:
        with _master(shared, "root@a"):
            # Killed, it leaves its socket behind with nothing listening
            pass

        shared.cancel_forward("root@a", "1:h:2")
    finally:
        shared.close()


def test_a_forward_its_running_connection_refuses_to_cancel_fails() -> None:
    shared = SharedConnections(_Runner(failing="cancel", error=_REFUSED))
    path = _socket_path(shared, "root@a")
    try:
        with _master(shared, "root@a"), pytest.raises(ControlError, match="ssh -O cancel"):
            shared.cancel_forward("root@a", "1:h:2")
    finally:
        shutil.rmtree(path.parent)


def test_a_forward_on_a_stuck_connection_fails_to_cancel() -> None:
    shared = SharedConnections(
        _Runner(failing="check", error=subprocess.TimeoutExpired(["ssh"], 10))
    )
    path = _socket_path(shared, "root@a")
    try:
        with (
            _master(shared, "root@a"),
            pytest.raises(ControlError, match="no answer within 10s"),
        ):
            shared.cancel_forward("root@a", "1:h:2")
    finally:
        shutil.rmtree(path.parent)


def test_the_socket_path_fits_macos_whatever_the_temp_dir(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Two nested `nix develop` shells under macOS's TMPDIR
    long_tmp = tmp_path / ("y" * 82)
    long_tmp.mkdir()
    monkeypatch.setattr(tempfile, "tempdir", str(long_tmp))
    shared = SharedConnections(_Runner())
    try:
        path = _socket_path(shared, "root@h")
    finally:
        shared.close()

    # ssh binds the master at the path plus 17 characters, then links it into place
    longest = len(str(path)) + 17
    assert longest < 104, f"{longest} bytes do not fit macOS's socket path"


def test_the_run_s_connections_close_when_the_run_ends() -> None:
    runner = _Runner()
    with exits.trapped(), ssh.for_the_run(runner):
        shared = ssh.connections.get()
        assert shared is not None
        master = _master(shared, "root@a")

    with master:
        assert _sent(runner, "exit") == ["root@a"]
    assert ssh.connections.get() is None


def test_the_run_s_connections_need_a_run_to_close_them() -> None:
    with pytest.raises(RuntimeError, match="BUG"), ssh.for_the_run(_Runner()):
        pytest.fail("connections nothing closes must not open")
