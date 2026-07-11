"""Unit tests for the SSH device transport."""

import subprocess
from collections.abc import Iterable
from pathlib import Path

import pytest

from bmc_tui.device import Device
from bmc_tui.stage import dry_run


class _FakeExec:
    """Records run/stream calls; returns canned stdout or raises a canned error."""

    def __init__(self, *, stdout: str = "", error: Exception | None = None) -> None:
        self.runs: list[list[str]] = []
        self.streams: list[tuple[list[str], bytes]] = []
        self._stdout = stdout
        self._error = error

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        self.runs.append(argv)
        if self._error is not None:
            raise self._error
        return subprocess.CompletedProcess(argv, 0, stdout=self._stdout, stderr="")

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        self.streams.append((argv, b"".join(chunks)))


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
    Device("h", backend=backend).push(fw, "/mnt/data/fw.tar")
    argv, data = backend.streams[0]
    assert argv[0] == "ssh"
    assert argv[-1] == "cat > /mnt/data/fw.tar"
    assert data == b"firmware-bytes"


def test_push_quotes_remote_path(tmp_path: Path) -> None:
    local = tmp_path / "f.bin"
    local.write_bytes(b"x")
    backend = _FakeExec()
    dev = Device("h", backend=backend)
    dev.push(local, "/mnt/data/odd name.tar.gz")
    argv, _ = backend.streams[0]
    assert argv[-1] == "cat > '/mnt/data/odd name.tar.gz'"


def test_push_skips_under_dry_run(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    fw.write_bytes(b"x")
    backend = _FakeExec()
    token = dry_run.set(True)
    try:
        Device("h", backend=backend).push(fw, "/mnt/data/fw.tar")
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


def test_run_reraises_disconnect_when_not_expected() -> None:
    backend = _FakeExec(error=subprocess.CalledProcessError(255, ["ssh"]))
    with pytest.raises(subprocess.CalledProcessError):
        Device("h", backend=backend).run("sysupgrade x")


def test_print_shows_host(capsys: pytest.CaptureFixture[str]) -> None:
    Device("192.168.1.183", backend=_FakeExec()).print()
    assert "192.168.1.183" in capsys.readouterr().out
