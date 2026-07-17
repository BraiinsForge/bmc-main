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

import subprocess
from collections.abc import Callable, Iterable
from pathlib import Path

import pytest

from bmc_tui.device import Device, RemotePath
from bmc_tui.stage import dry_run


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


# ── read / run / push ─────────────────────────────────────────────────────────


def test_read_returns_stripped_stdout_over_ssh() -> None:
    backend = _FakeExec(stdout="  hello\n")
    assert Device("h", backend=backend).read("echo hi") == "hello"
    assert backend.runs[0][0] == "ssh"
    assert backend.runs[0][-1] == "echo hi"


def test_run_executes_when_not_dry_run() -> None:
    backend = _FakeExec()
    Device("h", backend=backend).run("sysupgrade x")
    assert backend.runs[0][-1] == "sysupgrade x"


def test_run_skips_under_dry_run() -> None:
    backend = _FakeExec()
    token = dry_run.set(True)
    try:
        Device("h", backend=backend).run("sysupgrade x")
    finally:
        dry_run.reset(token)
    assert backend.runs == []


def test_push_streams_file_over_ssh(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    fw.write_bytes(b"firmware-bytes")
    backend = _FakeExec()
    Device("h", backend=backend).push(fw, RemotePath("/mnt/data/fw.tar"))
    argv, data = backend.streams[0]
    assert argv[0] == "ssh"
    assert argv[-1] == "cat > /mnt/data/fw.tar"
    assert data == b"firmware-bytes"


def test_push_quotes_remote_path(tmp_path: Path) -> None:
    local = tmp_path / "f.bin"
    local.write_bytes(b"x")
    backend = _FakeExec()
    dev = Device("h", backend=backend)
    dev.push(local, RemotePath("/mnt/data/odd name.tar.gz"))
    argv, _ = backend.streams[0]
    assert argv[-1] == "cat > '/mnt/data/odd name.tar.gz'"


def test_push_skips_under_dry_run(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    fw.write_bytes(b"x")
    backend = _FakeExec()
    token = dry_run.set(True)
    try:
        Device("h", backend=backend).push(fw, RemotePath("/mnt/data/fw.tar"))
    finally:
        dry_run.reset(token)
    assert backend.streams == []


def test_extract_tar_streams_into_remote_tar(tmp_path: Path) -> None:
    tarball = tmp_path / "nix.tar.gz"
    tarball.write_bytes(b"init-bytes")
    backend = _FakeExec()
    Device("h", backend=backend).extract_tar(tarball)
    argv, data = backend.streams[0]
    assert argv[0] == "ssh"
    assert argv[-1] == "tar xzf - -C /"
    assert data == b"init-bytes"


def test_extract_tar_skips_under_dry_run(tmp_path: Path) -> None:
    tarball = tmp_path / "nix.tar.gz"
    tarball.write_bytes(b"x")
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
    backend = _FakeExec(stdout='{"board_name": "braiins,stm32mp157c-ii3-bmc1"}')
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
    backend = _FakeExec(
        stdout=(
            '{"board_name": "braiins,stm32mp157c-ii3-bmc1", "release": {"target": "stm32mp15/ii3"}}'
        )
    )
    dev = Device("h", backend=backend)
    assert dev.target == "stm32mp15/ii3"
    assert dev.board == "braiins,stm32mp157c-ii3-bmc1"
    assert len(backend.runs) == 1  # one ubus call backs both getters


def test_run_swallows_disconnect_when_expected() -> None:
    backend = _FakeExec(error=subprocess.CalledProcessError(255, ["ssh"]))
    Device("h", backend=backend).run("sysupgrade x", expect_disconnect=True)  # no raise


def test_run_swallows_signal_killed_session_when_disconnect_expected() -> None:
    # a rebooting device can kill the remote session with a signal instead of
    # dropping the connection; 246 is -SIGUSR1 wrapped to an unsigned byte
    backend = _FakeExec(error=subprocess.CalledProcessError(246, ["ssh"]))
    Device("h", backend=backend).run("sysupgrade x", expect_disconnect=True)  # no raise


def test_run_swallows_raw_negative_signal_death_when_disconnect_expected() -> None:
    # subprocess.run reports a locally-signalled ssh as a negative returncode
    # (-N), not 128+N; run must treat that as session death too, matching
    # run_captured. The old `>= 128`-only predicate re-raised it.
    backend = _FakeExec(error=subprocess.CalledProcessError(-15, ["ssh"]))
    Device("h", backend=backend).run("sysupgrade x", expect_disconnect=True)  # no raise


def test_run_reraises_remote_failure_even_when_disconnect_expected() -> None:
    backend = _FakeExec(error=subprocess.CalledProcessError(1, ["ssh"]))
    with pytest.raises(subprocess.CalledProcessError):
        Device("h", backend=backend).run("sysupgrade x", expect_disconnect=True)


def test_run_reraises_disconnect_when_not_expected() -> None:
    backend = _FakeExec(error=subprocess.CalledProcessError(255, ["ssh"]))
    with pytest.raises(subprocess.CalledProcessError):
        Device("h", backend=backend).run("sysupgrade x")


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
