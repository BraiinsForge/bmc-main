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

"""Unit tests for the SSH device transport."""

import errno
import gc
import inspect
import os
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
from collections.abc import Callable, Generator, Iterable, Iterator
from contextlib import suppress
from pathlib import Path
from typing import cast, final

import pytest
from rich.console import Console
from rich.text import Text

from bmc_tui import console, device, ssh
from bmc_tui.device import Device, RemotePath, _RealExec
from bmc_tui.ssh import SharedConnections
from bmc_tui.stage import Abort, dry_run
from tests import device_replies


@final
class _FakeExec:
    """Records run/stream calls; returns canned stdout or raises a canned error.
    stream_output() feeds canned lines to the callback and returns a canned code."""

    def __init__(
        self,
        *,
        stdout: str = "",
        error: Exception | None = None,
        lines: list[str] | None = None,
        code: int = 0,
    ) -> None:
        self.runs: list[list[str]] = []
        self.streams: list[tuple[list[str], bytes]] = []
        self.stream_outputs: list[list[str]] = []
        self._stdout = stdout
        self._error = error
        self._lines = lines or []
        self._code = code

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        self.runs.append(argv)
        if self._error is not None:
            raise self._error
        return subprocess.CompletedProcess(argv, 0, stdout=self._stdout, stderr="")

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        self.streams.append((argv, b"".join(chunks)))

    def stream_output(self, argv: list[str], on_line: Callable[[str], None]) -> int:
        self.stream_outputs.append(argv)
        for line in self._lines:
            on_line(line)
        return self._code


# ── one connection per run ────────────────────────────────────────────────────


def _control_path(argv: list[str]) -> str:
    paths = [opt for opt in argv if opt.startswith("ControlPath=")]
    assert paths, f"no shared connection in {argv}"
    return paths[0]


@final
class _Master:
    """Answers `-O` commands as a master does, listening on the forwards it holds.
    Fails `failing` with `error`, and with `keeps_forwards` refuses a cancel yet exits with 0.
    With `ssh_log`, notes in `before_forward` which ssh calls a forward came after."""

    def __init__(
        self,
        *,
        failing: str | None = None,
        error: Exception | None = None,
        keeps_forwards: bool = False,
        ssh_log: Path | None = None,
    ) -> None:
        self.runs: list[list[str]] = []
        self.before_forward: list[str] = []
        self._failing = failing
        self._error = error
        self._keeps_forwards = keeps_forwards
        self._ssh_log = ssh_log
        self._forwards: dict[str, socket.socket] = {}

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        self.runs.append(argv)
        command = argv[argv.index("-O") + 1]
        if command == "forward" and self._ssh_log is not None and self._ssh_log.exists():
            self.before_forward = self._ssh_log.read_text().splitlines()
        if command == self._failing and self._error is not None:
            raise self._error
        if command == "forward":
            spec = argv[argv.index("-L") + 1]
            host, port = spec.split(":")[:2]
            self._forwards[spec] = socket.create_server((host, int(port)))
        if command == "cancel" and not self._keeps_forwards:
            self._forwards.pop(argv[argv.index("-L") + 1]).close()
        return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")

    def sent(self, command: str) -> list[list[str]]:
        return [argv for argv in self.runs if argv[argv.index("-O") + 1] == command]

    def stop(self) -> None:
        for listener in self._forwards.values():
            listener.close()


@pytest.fixture
def control() -> Iterator[_Master]:
    """Where the shared connections send their `-O` commands."""
    master = _Master()
    yield master
    master.stop()


@pytest.fixture
def shared(control: _Master) -> Iterator[SharedConnections]:
    connections = SharedConnections(control)
    yield connections
    connections.close()


def _master_socket(connections: SharedConnections, login: str) -> Path:
    """Put a socket where the login's master would keep one."""
    path = Path(_control_path(connections.options(login)).removeprefix("ControlPath="))
    path.touch()
    return path


@pytest.fixture
def stuck() -> Iterator[SharedConnections]:
    """Connections whose master to `root@deck` takes a forward but answers no `-O check`."""
    master = _Master(failing="check", error=subprocess.TimeoutExpired(["ssh"], 10))
    connections = SharedConnections(master)
    path = Path(_control_path(connections.options("root@deck")).removeprefix("ControlPath="))
    with socket.socket(socket.AF_UNIX) as control_socket:
        control_socket.bind(str(path))
        control_socket.listen()
        yield connections
    master.stop()
    shutil.rmtree(path.parent)


@pytest.fixture
def rendered(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> Callable[[Abort], str]:
    """What `entrypoint` shows for an Abort, which it prints as markup."""
    monkeypatch.setattr(console, "console", Console(stderr=True, width=200))

    def render(abort: Abort) -> str:
        console.error(abort.hint)
        return Text.from_ansi(capsys.readouterr().err).plain

    return render


@pytest.fixture
def stderr_holder(tmp_path: Path) -> Iterator[str]:
    """Shell that leaves a process holding stderr open, as an ssh proxy would.
    It is killed after the test, so neither it nor the thread draining its pipe outlives it."""
    pid_file = tmp_path / "holder.pid"
    yield f"sleep 5 < /dev/null & echo $! > {shlex.quote(str(pid_file))}"
    with suppress(FileNotFoundError, ProcessLookupError):
        os.kill(int(pid_file.read_text()), signal.SIGKILL)


def test_every_ssh_to_a_device_shares_one_connection(
    tmp_path: Path, shared: SharedConnections
) -> None:
    fw = tmp_path / "fw.tar"
    _ = fw.write_bytes(b"x")
    backend = _FakeExec()
    dev = Device("h", backend=backend, connections=shared)

    _ = dev.read("true")
    dev.push(fw, RemotePath("/tmp/fw.tar"))

    argvs = [backend.runs[0], backend.streams[0][0]]
    for argv in argvs:
        assert "ControlMaster=auto" in argv
        persist = next(opt for opt in argv if opt.startswith("ControlPersist="))
        assert persist.removeprefix("ControlPersist=")[:1].isdigit(), (
            "an idle connection closes after a time, rather than never or at once"
        )
        assert argv.index("ControlMaster=auto") < argv.index("root@h"), "options precede the login"
    assert len({_control_path(argv) for argv in argvs}) == 1, "one socket for the device"


def test_devices_for_one_login_share_its_connection_and_other_logins_get_their_own(
    shared: SharedConnections,
) -> None:
    backend = _FakeExec()
    for host in ("h", "h", "other"):
        _ = Device(host, backend=backend, connections=shared).read("true")

    first, second, other = (_control_path(argv) for argv in backend.runs)

    assert first == second, "a procedure builds several devices for one login"
    assert other != first


def _exiting(code: int) -> _FakeExec:
    """A device whose ssh exits with `code`: 255 as when the session dies with a reboot."""
    if code == 0:
        return _FakeExec()
    return _FakeExec(error=subprocess.CalledProcessError(code, ["ssh"], stderr="x"), code=code)


def _flash(dev: Device) -> object:
    return dev.run("sysupgrade x", expect_disconnect=True)


def _flash_streamed(dev: Device) -> object:
    return dev.run_streamed("sysupgrade x", on_line=print, expect_disconnect=True)


def _flash_captured(dev: Device) -> object:
    return dev.run_captured("sysupgrade x")


def _probe(dev: Device) -> object:
    return dev.run("true")


def _sample(dev: Device) -> object:
    return dev.run_streamed("ps", on_line=print)


@pytest.mark.parametrize(
    ("command", "code"),
    [
        (_flash, 0),
        (_flash, 255),
        (_flash_streamed, 0),
        (_flash_streamed, 255),
        (_flash_streamed, -signal.SIGTERM),
        (_flash_captured, 0),
        (_flash_captured, 255),
    ],
    ids=[
        "run exits",
        "run's session dies",
        "run_streamed exits",
        "run_streamed's session dies",
        "run_streamed's ssh is killed",
        "run_captured exits",
        "run_captured's session dies",
    ],
)
def test_a_command_that_takes_the_device_down_ends_every_connection_to_it(
    shared: SharedConnections,
    control: _Master,
    command: Callable[[Device], object],
    code: int,
) -> None:
    _ = _master_socket(shared, "root@h")
    _ = _master_socket(shared, "root@10.0.0.7")

    _ = command(Device("10.0.0.7", backend=_exiting(code), connections=shared))

    assert sorted(argv[-1] for argv in control.sent("exit")) == ["root@10.0.0.7", "root@h"], (
        "a connection left up would wait out its keepalives, the device's other login's included"
    )


@pytest.mark.parametrize(
    ("command", "code"),
    [(_probe, 0), (_sample, 0), (_flash, 1), (_flash_streamed, 1), (_flash_captured, 1)],
    ids=[
        "run without a disconnect",
        "run_streamed without a disconnect",
        "run that fails",
        "run_streamed that fails",
        "run_captured that fails",
    ],
)
def test_a_command_that_leaves_the_device_up_keeps_its_connections(
    shared: SharedConnections,
    control: _Master,
    command: Callable[[Device], object],
    code: int,
) -> None:
    _ = _master_socket(shared, "root@h")

    with suppress(subprocess.CalledProcessError):
        _ = command(Device("h", backend=_exiting(code), connections=shared))

    assert control.sent("exit") == []


@pytest.mark.parametrize(
    "command",
    [_flash, _flash_streamed, _flash_captured],
    ids=["run", "run_streamed", "run_captured"],
)
def test_a_dry_run_of_a_command_that_takes_the_device_down_keeps_its_connections(
    shared: SharedConnections, control: _Master, command: Callable[[Device], object]
) -> None:
    _ = _master_socket(shared, "root@h")
    token = dry_run.set(True)
    try:
        _ = command(Device("h", backend=_FakeExec(), connections=shared))
    finally:
        dry_run.reset(token)

    assert control.sent("exit") == [], "the probes a dry run still makes would log in again"


def test_a_connection_that_does_not_end_after_a_reboot_only_warns(
    monkeypatch: pytest.MonkeyPatch, stuck: SharedConnections
) -> None:
    warnings: list[str] = []
    monkeypatch.setattr(console, "warn", warnings.append)

    _ = Device("deck", backend=_exiting(255), connections=stuck).run(
        "sysupgrade x", expect_disconnect=True
    )

    assert len(warnings) == 1
    assert "still open, so the next ssh to it may wait" in warnings[0]
    assert "root@deck" in warnings[0], "the operator needs to know which one"


def test_a_reboot_goes_through_when_its_warning_meets_a_terminal_that_hung_up(
    monkeypatch: pytest.MonkeyPatch, stuck: SharedConnections
) -> None:
    def hung_up(_msg: str) -> None:
        raise OSError(errno.EIO, "Input/output error")

    monkeypatch.setattr(console, "warn", hung_up)

    outcome = Device("deck", backend=_exiting(255), connections=stuck).run_captured("sysupgrade x")

    assert outcome is not None
    assert outcome.status == "session-death"


def test_a_device_outside_a_run_has_no_connection_to_join() -> None:
    assert Device("h", backend=_FakeExec()).join_options() == [], (
        "the device's own options would override the user's NIX_SSHOPTS"
    )


def test_a_device_shares_the_run_s_connections(shared: SharedConnections) -> None:
    backend = _FakeExec()
    token = ssh.connections.set(shared)
    try:
        dev = Device("h", backend=backend)
    finally:
        ssh.connections.reset(token)

    _ = dev.read("true")

    assert _control_path(backend.runs[0]) == _control_path(shared.options("root@h"))


def test_a_device_given_no_connections_makes_its_own_even_in_a_run(
    shared: SharedConnections,
) -> None:
    backend = _FakeExec()
    token = ssh.connections.set(shared)
    try:
        dev = Device("h", backend=backend, connections=None)
    finally:
        ssh.connections.reset(token)

    _ = dev.read("true")

    assert not any(opt.startswith("Control") for opt in backend.runs[0])


def test_a_device_outside_a_run_makes_its_own_connection() -> None:
    backend = _FakeExec()

    _ = Device("h", backend=backend).read("true")

    assert not any(opt.startswith("Control") for opt in backend.runs[0])


def test_an_upload_cut_short_does_not_wait_for_the_child() -> None:
    def chunks() -> Iterator[bytes]:
        yield b"x"
        raise RuntimeError("stopped")

    started = time.monotonic()
    with pytest.raises(RuntimeError):
        _RealExec().stream(["sh", "-c", "exec sleep 60"], chunks())

    assert time.monotonic() - started < 10, "the child is killed, not waited for"


def test_an_upload_cut_short_after_its_child_died_raises_what_cut_it_short() -> None:
    def chunks() -> Iterator[bytes]:
        yield b"x"
        time.sleep(0.2)
        raise RuntimeError("stopped")

    with pytest.raises(RuntimeError, match="stopped"):
        _RealExec().stream(["sh", "-c", "exit 0"], chunks())


def test_an_upload_cut_short_does_not_wait_for_whatever_else_holds_its_stderr(
    stderr_holder: str,
) -> None:
    def chunks() -> Iterator[bytes]:
        yield b"x"
        time.sleep(0.2)
        raise RuntimeError("stopped")

    started = time.monotonic()
    with pytest.raises(RuntimeError):
        _RealExec().stream(["sh", "-c", f"{stderr_holder}; exec sleep 60"], chunks())

    assert time.monotonic() - started < 3


_PIPE_FULL = 4 * 65_536


def test_an_upload_whose_child_fills_stderr_before_reading_still_finishes() -> None:
    # `timeout` frees a deadlocked thread,
    # so a failure does not hang the suite
    argv = ["timeout", "20", "sh", "-c", f"head -c {_PIPE_FULL} /dev/zero >&2; cat > /dev/null"]
    errors: list[BaseException] = []

    def upload() -> None:
        try:
            _RealExec().stream(argv, iter([b"x" * _PIPE_FULL]))
        except BaseException as e:
            errors.append(e)

    uploading = threading.Thread(target=upload, daemon=True)
    uploading.start()
    uploading.join(timeout=10)

    assert not uploading.is_alive(), "stdin and stderr both blocked"
    assert errors == []


def test_an_upload_whose_child_exits_early_fails_with_what_it_said() -> None:
    argv = ["sh", "-c", "echo 'no space left' >&2; exit 1"]

    with pytest.raises(subprocess.CalledProcessError) as failed:
        _RealExec().stream(argv, iter([b"x" * _PIPE_FULL] * 16))

    assert b"no space left" in cast("bytes", failed.value.stderr)


def test_an_upload_whose_child_fails_does_not_wait_for_whatever_else_holds_its_stderr(
    stderr_holder: str,
) -> None:
    argv = ["sh", "-c", f"{stderr_holder}; echo 'no space left' >&2; exit 1"]

    started = time.monotonic()
    with pytest.raises(subprocess.CalledProcessError) as failed:
        _RealExec().stream(argv, iter([b"x" * _PIPE_FULL] * 16))

    assert time.monotonic() - started < 3
    assert b"no space left" in cast("bytes", failed.value.stderr)


def test_an_upload_whose_child_exits_early_closes_what_it_was_reading_from() -> None:
    closed: list[bool] = []

    def chunks() -> Iterator[bytes]:
        try:
            while True:
                yield b"x" * _PIPE_FULL
        finally:
            closed.append(True)

    # A reference cycle keeps the generator, the file
    # and progress bar it holds until a GC run
    collecting = gc.isenabled()
    gc.disable()

    try:
        with suppress(subprocess.CalledProcessError):
            _RealExec().stream(["sh", "-c", "exit 1"], chunks())
        assert closed == [True]
    finally:
        if collecting:
            gc.enable()


def test_an_upload_whose_child_exits_cleanly_without_taking_it_all_still_fails() -> None:
    with pytest.raises(BrokenPipeError):
        _RealExec().stream(["sh", "-c", "exit 0"], iter([b"x" * _PIPE_FULL] * 16))


# ── read / run / push ─────────────────────────────────────────────────────────


def test_read_returns_stripped_stdout_over_ssh() -> None:
    backend = _FakeExec(stdout="  hello\n")
    assert Device("h", backend=backend).read("echo hi") == "hello"
    assert backend.runs[0][0] == "ssh"
    assert backend.runs[0][-1] == "echo hi"


def test_run_executes_when_not_dry_run() -> None:
    backend = _FakeExec()
    _ = Device("h", backend=backend).run("sysupgrade x")
    assert backend.runs[0][-1] == "sysupgrade x"


def test_run_skips_under_dry_run() -> None:
    backend = _FakeExec()
    token = dry_run.set(True)
    try:
        _ = Device("h", backend=backend).run("sysupgrade x")
    finally:
        dry_run.reset(token)
    assert backend.runs == []


@final
class _StreamFailsAfterAChunk:
    def __init__(self) -> None:
        self.fed: Iterable[bytes] | None = None

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        raise AssertionError(argv)

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        self.fed = chunks
        _ = next(iter(chunks))
        raise subprocess.CalledProcessError(1, argv)

    def stream_output(self, argv: list[str], _on_line: Callable[[str], None]) -> int:
        raise AssertionError(argv)


def _push(dev: Device, local: Path) -> None:
    dev.push(local, RemotePath("/mnt/data/fw.tar"))


def _extract_tar(dev: Device, local: Path) -> None:
    dev.extract_tar(local)


@pytest.mark.parametrize("upload", [_push, _extract_tar], ids=["push", "extract_tar"])
def test_an_upload_that_fails_has_closed_its_file_and_progress_bar_by_the_time_it_says_so(
    tmp_path: Path, upload: Callable[[Device, Path], None]
) -> None:
    local = tmp_path / "fw.tar"
    _ = local.write_bytes(b"firmware-bytes")
    backend = _StreamFailsAfterAChunk()

    try:
        upload(Device("h", backend=backend), local)
    except subprocess.CalledProcessError:
        fed = cast("Generator[bytes]", backend.fed)
        assert inspect.getgeneratorstate(fed) == inspect.GEN_CLOSED
    else:
        pytest.fail("the upload must fail")


def test_push_streams_file_over_ssh(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    _ = fw.write_bytes(b"firmware-bytes")
    backend = _FakeExec()
    Device("h", backend=backend).push(fw, RemotePath("/mnt/data/fw.tar"))
    argv, data = backend.streams[0]
    assert argv[0] == "ssh"
    assert argv[-1] == "cat > /mnt/data/fw.tar"
    assert data == b"firmware-bytes"


def test_push_quotes_remote_path(tmp_path: Path) -> None:
    local = tmp_path / "f.bin"
    _ = local.write_bytes(b"x")
    backend = _FakeExec()
    dev = Device("h", backend=backend)
    dev.push(local, RemotePath("/mnt/data/odd name.tar.gz"))
    argv, _ = backend.streams[0]
    assert argv[-1] == "cat > '/mnt/data/odd name.tar.gz'"


def test_push_skips_under_dry_run(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    _ = fw.write_bytes(b"x")
    backend = _FakeExec()
    token = dry_run.set(True)
    try:
        Device("h", backend=backend).push(fw, RemotePath("/mnt/data/fw.tar"))
    finally:
        dry_run.reset(token)
    assert backend.streams == []


def test_extract_tar_streams_into_remote_tar(tmp_path: Path) -> None:
    tarball = tmp_path / "nix.tar.gz"
    _ = tarball.write_bytes(b"init-bytes")
    backend = _FakeExec()
    Device("h", backend=backend).extract_tar(tarball)
    argv, data = backend.streams[0]
    assert argv[0] == "ssh"
    assert argv[-1] == "tar xzf - -C /"
    assert data == b"init-bytes"


def test_extract_tar_skips_under_dry_run(tmp_path: Path) -> None:
    tarball = tmp_path / "nix.tar.gz"
    _ = tarball.write_bytes(b"x")
    backend = _FakeExec()
    token = dry_run.set(True)
    try:
        Device("h", backend=backend).extract_tar(tarball)
    finally:
        dry_run.reset(token)
    assert backend.streams == []


def test_login_is_explicit_user_at_host() -> None:
    assert Device("h", backend=_FakeExec()).login == "root@h"
    assert Device("h", user="dev", backend=_FakeExec()).login == "dev@h"


# ── getters ──────────────────────────────────────────────────────────────────


def test_reachable_true_on_success() -> None:
    assert Device("h", backend=_FakeExec()).reachable is True


def test_reachable_false_on_ssh_error() -> None:
    backend = _FakeExec(error=subprocess.CalledProcessError(255, ["ssh"]))
    assert Device("h", backend=backend).reachable is False


def test_board_parses_board_name_and_caches() -> None:
    board = device_replies.board("stm32mp15/ii3", name="braiins,stm32mp157c-ii3-bmc1")
    backend = _FakeExec(stdout=board)
    dev = Device("h", backend=backend)
    assert dev.board == "braiins,stm32mp157c-ii3-bmc1"
    assert dev.board == "braiins,stm32mp157c-ii3-bmc1"
    assert len(backend.runs) == 1  # cached: only one ubus call


def test_version_reads_bos_version_and_is_not_cached() -> None:
    backend = _FakeExec(stdout="2026-06-14-0-c84f1b1d-26.07-plus-nightly")
    dev = Device("h", backend=backend)
    assert dev.version == "2026-06-14-0-c84f1b1d-26.07-plus-nightly"
    _ = dev.version
    assert len(backend.runs) == 2  # re-read each access
    assert backend.runs[0][-1] == "cat /etc/bos_version"


def test_version_runs_even_under_dry_run() -> None:
    backend = _FakeExec(stdout="v1")
    token = dry_run.set(True)
    try:
        assert Device("h", backend=backend).version == "v1"
    finally:
        dry_run.reset(token)
    assert backend.runs  # read still executed under dry-run


def test_target_parses_release_target_and_shares_board_cache() -> None:
    board = device_replies.board("stm32mp15/ii3", name="braiins,stm32mp157c-ii3-bmc1")
    backend = _FakeExec(stdout=board)
    dev = Device("h", backend=backend)
    assert dev.target == "stm32mp15/ii3"
    assert dev.board == "braiins,stm32mp157c-ii3-bmc1"
    assert len(backend.runs) == 1  # one ubus call backs both getters


def test_run_swallows_disconnect_when_expected() -> None:
    backend = _FakeExec(error=subprocess.CalledProcessError(255, ["ssh"]))
    _ = Device("h", backend=backend).run("sysupgrade x", expect_disconnect=True)  # no raise


def test_run_swallows_signal_killed_session_when_disconnect_expected() -> None:
    # a rebooting device can kill the remote session with a signal instead of
    # dropping the connection; 246 is -SIGUSR1 wrapped to an unsigned byte
    backend = _FakeExec(error=subprocess.CalledProcessError(246, ["ssh"]))
    _ = Device("h", backend=backend).run("sysupgrade x", expect_disconnect=True)  # no raise


def test_run_swallows_raw_negative_signal_death_when_disconnect_expected() -> None:
    # subprocess.run reports a locally-signalled ssh as a negative returncode
    # (-N), not 128+N; run must treat that as session death too, matching
    # run_captured. The old `>= 128`-only predicate re-raised it.
    backend = _FakeExec(error=subprocess.CalledProcessError(-15, ["ssh"]))
    _ = Device("h", backend=backend).run("sysupgrade x", expect_disconnect=True)  # no raise


def test_run_reraises_remote_failure_even_when_disconnect_expected() -> None:
    backend = _FakeExec(error=subprocess.CalledProcessError(1, ["ssh"]))
    with pytest.raises(subprocess.CalledProcessError):
        _ = Device("h", backend=backend).run("sysupgrade x", expect_disconnect=True)


def test_run_reraises_disconnect_when_not_expected() -> None:
    backend = _FakeExec(error=subprocess.CalledProcessError(255, ["ssh"]))
    with pytest.raises(subprocess.CalledProcessError):
        _ = Device("h", backend=backend).run("sysupgrade x")


# ── run_streamed ─────────────────────────────────────────────────────────────


def _drop(_line: str) -> None:
    """Discard a streamed line — for tests that only assert on the exit path."""


def test_run_streamed_feeds_lines_over_ssh() -> None:
    backend = _FakeExec(lines=['@bmc {"type":"realization_finished"}', "Image check failed."])
    seen: list[str] = []
    Device("h", backend=backend).run_streamed("sysupgrade x", on_line=seen.append)
    assert seen == ['@bmc {"type":"realization_finished"}', "Image check failed."]
    assert backend.stream_outputs[0][0] == "ssh"
    assert backend.stream_outputs[0][-1] == "sysupgrade x"


def test_run_streamed_skips_under_dry_run() -> None:
    backend = _FakeExec(lines=["x"])
    token = dry_run.set(True)
    try:
        Device("h", backend=backend).run_streamed("sysupgrade x", on_line=_drop)
    finally:
        dry_run.reset(token)
    assert backend.stream_outputs == []


def test_run_streamed_swallows_disconnect_when_expected() -> None:
    backend = _FakeExec(code=255)  # ssh's own code for a dropped connection
    Device("h", backend=backend).run_streamed("sysupgrade x", on_line=_drop, expect_disconnect=True)


def test_run_streamed_swallows_signal_killed_session_when_expected() -> None:
    backend = _FakeExec(code=246)  # -SIGUSR1 wrapped to an unsigned byte
    Device("h", backend=backend).run_streamed("sysupgrade x", on_line=_drop, expect_disconnect=True)


def test_run_streamed_reraises_remote_failure_even_when_disconnect_expected() -> None:
    backend = _FakeExec(code=1)  # session alive, command failed → a real failure
    with pytest.raises(subprocess.CalledProcessError):
        Device("h", backend=backend).run_streamed(
            "sysupgrade x", on_line=_drop, expect_disconnect=True
        )


def test_run_streamed_reraises_disconnect_when_not_expected() -> None:
    backend = _FakeExec(code=255)
    with pytest.raises(subprocess.CalledProcessError):
        Device("h", backend=backend).run_streamed("sysupgrade x", on_line=_drop)


def test_print_shows_host(capsys: pytest.CaptureFixture[str]) -> None:
    Device("192.168.1.183", backend=_FakeExec()).print()
    assert "192.168.1.183" in capsys.readouterr().out


# ── run_captured ───────────────────────────────────────────────────────────────


def _failing(returncode: int) -> "_FakeExec":
    err = subprocess.CalledProcessError(returncode, ["ssh"], output="out\n", stderr="err\n")
    return _FakeExec(error=err)


def test_run_captured_clean_exit_preserves_output() -> None:
    backend = _FakeExec(stdout="staged once\n")
    outcome = Device("h", backend=backend).run_captured("sysupgrade x")
    assert outcome is not None
    assert outcome.status == "clean"
    assert outcome.returncode == 0
    assert "staged once" in outcome.output


def test_run_captured_remote_failure_preserves_output() -> None:
    outcome = Device("h", backend=_failing(1)).run_captured("sysupgrade x")
    assert outcome is not None
    assert outcome.status == "failed"
    assert outcome.returncode == 1
    assert "out" in outcome.output and "err" in outcome.output


def test_run_captured_session_death_preserves_output() -> None:
    outcome = Device("h", backend=_failing(255)).run_captured("sysupgrade x")
    assert outcome is not None
    assert outcome.status == "session-death"
    assert "err" in outcome.output


def test_run_captured_signal_death_is_session_death() -> None:
    outcome = Device("h", backend=_failing(-9)).run_captured("sysupgrade x")
    assert outcome is not None
    assert outcome.status == "session-death"


def test_run_captured_skips_under_dry_run() -> None:
    backend = _FakeExec()
    token = dry_run.set(True)
    try:
        assert Device("h", backend=backend).run_captured("sysupgrade x") is None
    finally:
        dry_run.reset(token)
    assert backend.runs == []


# Logs each call to `calls`, runs a command by exiting with 0, and holds a `-L` forward itself
_FORWARDING_SSH = """
import socket, sys
from pathlib import Path
here = Path(sys.argv[0]).parent
with (here / "calls").open("a") as calls:
    calls.write(" ".join(sys.argv[1:]) + "\\n")
if "-L" not in sys.argv:
    sys.exit(0)
(here / "argv").write_text("\\n".join(sys.argv[1:]))
host, port = sys.argv[sys.argv.index("-L") + 1].split(":")[:2]
listener = socket.create_server((host, int(port)))
while True:
    listener.accept()[0].close()
"""


def _ssh_calls(tmp_path: Path) -> list[str]:
    return (tmp_path / "calls").read_text().splitlines()


def _fake_ssh(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, body: str) -> Path:
    ssh = tmp_path / "ssh"
    _ = ssh.write_text(f"#!{sys.executable}\n{body}")
    ssh.chmod(0o755)
    monkeypatch.setenv("PATH", f"{tmp_path}:{Path(sys.executable).parent}")
    return ssh


def _accepts(address: str) -> bool:
    host, port = address.split(":")
    try:
        socket.create_connection((host, int(port)), timeout=1).close()
    except OSError:
        return False
    return True


def test_a_forward_in_a_run_is_held_by_its_connection_until_the_block_ends(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, _FORWARDING_SSH)
    control = _Master(ssh_log=tmp_path / "calls")
    shared = SharedConnections(control)
    _ = _master_socket(shared, "root@deck")
    free_port = device._free_port
    before_port: list[str] = []

    def picked_port() -> int:
        before_port.extend(_ssh_calls(tmp_path) if (tmp_path / "calls").exists() else [])
        return free_port()

    monkeypatch.setattr(device, "_free_port", picked_port)

    try:
        with Device("deck", connections=shared).forward(80) as address:
            assert _accepts(address)
    finally:
        shared.close()
        control.stop()

    spec = f"{address}:127.0.0.1:80"
    assert [argv[-3:] for argv in control.sent("forward")] == [["-L", spec, "root@deck"]], (
        "the device side is its loopback web port"
    )
    assert [argv[-3:] for argv in control.sent("cancel")] == [["-L", spec, "root@deck"]]
    assert not _accepts(address), "the forward must not outlive the block"
    assert len(control.before_forward) == 1, "one ssh, the one opening the connection"
    assert control.before_forward[0].endswith("root@deck true"), (
        "the connection opens first, so a password prompt has all the time it needs"
    )
    assert _control_path(control.before_forward[0].split()) == _control_path(
        control.sent("forward")[0]
    ), "the forward goes to the connection that ssh opened"
    assert before_port == control.before_forward, (
        "the port is picked once the connection is open, so a slow password cannot cost it"
    )


def test_a_forward_its_connection_refuses_aborts_without_logging_in_again(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, rendered: Callable[[Abort], str]
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, _FORWARDING_SSH)
    refusing = _Master(
        failing="forward",
        error=subprocess.CalledProcessError(255, ["ssh"], stderr="Port forwarding [/x] failed"),
    )
    connections = SharedConnections(refusing)
    try:
        with pytest.raises(Abort) as aborted, Device("deck", connections=connections).forward(80):
            pytest.fail("a refused forward must not yield")
    finally:
        connections.close()

    assert "Port forwarding [/x] failed" in rendered(aborted.value)
    assert len(_ssh_calls(tmp_path)) == 1, "only the ssh opening the connection, no second login"


def test_a_forward_whose_login_fails_shows_what_ssh_said_word_for_word(
    shared: SharedConnections, rendered: Callable[[Abort], str]
) -> None:
    refused = subprocess.CalledProcessError(255, ["ssh"], stderr="channel [/x] refused")
    dev = Device("[fe80::1]", backend=_FakeExec(error=refused), connections=shared)

    with pytest.raises(Abort) as aborted, dev.forward(80):
        pytest.fail("a failed login must not yield")

    assert "forwarding root@[fe80::1]:80 failed (ssh exit 255): channel [/x] refused" in (
        rendered(aborted.value)
    )


def test_a_tunnel_whose_ssh_exits_shows_what_it_said_word_for_word(
    rendered: Callable[[Abort], str],
) -> None:
    argv = ["sh", "-c", "echo 'bind [/x] failed' >&2; exit 255"]
    with (
        subprocess.Popen(argv, stderr=subprocess.PIPE, text=True) as proc,
        pytest.raises(Abort) as aborted,
    ):
        device._await_listening(proc, device._free_port(), "the tunnel")

    assert "the tunnel failed (ssh exit 255): bind [/x] failed" in rendered(aborted.value)


def test_a_forward_cut_short_still_cancels_its_forward(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, shared: SharedConnections, control: _Master
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, _FORWARDING_SSH)
    _ = _master_socket(shared, "root@deck")

    with (
        pytest.raises(RuntimeError, match="cut short"),
        Device("deck", connections=shared).forward(80) as address,
    ):
        raise RuntimeError("cut short")

    assert [argv[-3:] for argv in control.sent("cancel")] == [
        ["-L", f"{address}:127.0.0.1:80", "root@deck"]
    ]
    assert not _accepts(address)


def test_a_forward_cut_short_warns_when_its_connection_ignores_the_cancel(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, _FORWARDING_SSH)
    warnings: list[str] = []
    monkeypatch.setattr(console, "warn", warnings.append)
    ignoring = _Master(keeps_forwards=True)
    connections = SharedConnections(ignoring)
    _ = _master_socket(connections, "root@deck")
    try:
        with (
            pytest.raises(RuntimeError, match="cut short"),
            Device("deck", connections=connections).forward(80),
        ):
            raise RuntimeError("cut short")
    finally:
        ignoring.stop()
        connections.close()

    assert len(warnings) == 1
    assert "left 127.0.0.1:" in warnings[0]


def test_a_cancel_its_connection_ignores_aborts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, _FORWARDING_SSH)
    ignoring = _Master(keeps_forwards=True)
    connections = SharedConnections(ignoring)
    _ = _master_socket(connections, "root@deck")
    try:
        with (
            pytest.raises(Abort, match=r"left 127\.0\.0\.1:[0-9]+ listening"),
            Device("deck", connections=connections).forward(80),
        ):
            pass
    finally:
        ignoring.stop()
        connections.close()


def test_a_forward_cut_short_keeps_its_error_and_warns_when_cancelling_fails_too(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, stuck: SharedConnections
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, _FORWARDING_SSH)
    warnings: list[str] = []
    monkeypatch.setattr(console, "warn", warnings.append)

    with (
        pytest.raises(RuntimeError, match="cut short"),
        Device("deck", connections=stuck).forward(80),
    ):
        raise RuntimeError("cut short")

    assert len(warnings) == 1
    assert "cancelling the forward failed too" in warnings[0]
    assert "no answer within 10s" in warnings[0]


def test_a_forward_cut_short_keeps_its_error_when_the_terminal_hung_up(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, stuck: SharedConnections
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, _FORWARDING_SSH)

    def hung_up(_msg: str) -> None:
        raise OSError(errno.EIO, "Input/output error")

    monkeypatch.setattr(console, "warn", hung_up)

    with (
        pytest.raises(RuntimeError, match="cut short"),
        Device("deck", connections=stuck).forward(80),
    ):
        raise RuntimeError("cut short")


def test_a_forward_that_cannot_be_cancelled_aborts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, stuck: SharedConnections
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, _FORWARDING_SSH)

    with (
        pytest.raises(Abort, match="no answer within 10s"),
        Device("deck", connections=stuck).forward(80),
    ):
        pass


def test_a_forward_outside_a_run_is_held_by_an_ssh_of_its_own(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, _FORWARDING_SSH)

    with Device("deck").forward(80) as address:
        assert _accepts(address)
        argv = (tmp_path / "argv").read_text().splitlines()

    assert f"{address}:127.0.0.1:80" in argv, "the device side is its loopback web port"
    assert "ExitOnForwardFailure=yes" in argv, "a refused forward must fail, not hang"
    assert "ControlPath=none" in argv, "a master from the user's ssh config must not take it"
    assert argv[-1] == "root@deck"
    assert not _accepts(address), "the tunnel must not outlive the block"


def test_an_own_ssh_that_ends_without_forwarding_aborts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, "import sys\nsys.exit(0)\n")

    with pytest.raises(Abort, match="ssh exit 0"), Device("deck").forward(80):
        pytest.fail("a forward nothing holds must not yield")


def test_a_forward_in_a_run_whose_connection_cannot_open_aborts_with_ssh_s_reason(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, shared: SharedConnections, control: _Master
) -> None:
    _ = _fake_ssh(
        tmp_path,
        monkeypatch,
        "import sys\nprint('Permission denied', file=sys.stderr)\nsys.exit(255)\n",
    )

    with (
        pytest.raises(Abort, match="Permission denied"),
        Device("deck", connections=shared).forward(80),
    ):
        pytest.fail("a forward without a connection must not yield")

    assert control.sent("forward") == []


def test_a_refused_forward_outside_a_run_aborts_with_ssh_s_reason(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _ = _fake_ssh(
        tmp_path,
        monkeypatch,
        "import sys\nprint('administratively prohibited', file=sys.stderr)\nsys.exit(255)\n",
    )

    with pytest.raises(Abort, match="administratively prohibited"), Device("deck").forward(80):
        pytest.fail("a refused forward must not yield")


def test_a_forward_nobody_listens_on_aborts_at_the_deadline(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _ = _fake_ssh(tmp_path, monkeypatch, "import time\ntime.sleep(30)\n")
    monkeypatch.setattr(device, "_FORWARD_TIMEOUT_S", 0.5)

    with pytest.raises(Abort, match="no listener"), Device("deck").forward(80):
        pytest.fail("a forward nothing listens on must not yield")
