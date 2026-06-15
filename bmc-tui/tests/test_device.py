"""Unit tests for the SSH/SCP device transport."""

import subprocess
from pathlib import Path

import pytest

from bmc_tui.device import Device
from bmc_tui.stage import dry_run


class _FakeRunner:
    """Records argv and returns canned stdout, or raises a canned error."""

    def __init__(self, *, stdout: str = "", error: Exception | None = None) -> None:
        self.calls: list[list[str]] = []
        self._stdout = stdout
        self._error = error

    def __call__(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        self.calls.append(argv)
        if self._error is not None:
            raise self._error
        return subprocess.CompletedProcess(argv, 0, stdout=self._stdout, stderr="")


# ── read / run / push ─────────────────────────────────────────────────────────


def test_read_returns_stripped_stdout_over_ssh() -> None:
    runner = _FakeRunner(stdout="  hello\n")
    assert Device("h", runner=runner).read("echo hi") == "hello"
    assert runner.calls[0][0] == "ssh"
    assert runner.calls[0][-1] == "echo hi"


def test_run_executes_when_not_dry_run() -> None:
    runner = _FakeRunner()
    Device("h", runner=runner).run("sysupgrade x")
    assert runner.calls[0][-1] == "sysupgrade x"


def test_run_skips_under_dry_run() -> None:
    runner = _FakeRunner()
    token = dry_run.set(True)
    try:
        Device("h", runner=runner).run("sysupgrade x")
    finally:
        dry_run.reset(token)
    assert runner.calls == []


def test_push_builds_scp_when_not_dry_run() -> None:
    runner = _FakeRunner()
    Device("h", runner=runner).push(Path("/tmp/fw.tar"), "/mnt/data/")
    argv = runner.calls[0]
    assert argv[0] == "scp"
    assert argv[-2:] == ["/tmp/fw.tar", "root@h:/mnt/data/"]


def test_push_skips_under_dry_run() -> None:
    runner = _FakeRunner()
    token = dry_run.set(True)
    try:
        Device("h", runner=runner).push(Path("/tmp/fw.tar"), "/mnt/data/")
    finally:
        dry_run.reset(token)
    assert runner.calls == []


# ── getters ──────────────────────────────────────────────────────────────────


def test_reachable_true_on_success() -> None:
    assert Device("h", runner=_FakeRunner()).reachable is True


def test_reachable_false_on_ssh_error() -> None:
    runner = _FakeRunner(error=subprocess.CalledProcessError(255, ["ssh"]))
    assert Device("h", runner=runner).reachable is False


def test_board_parses_board_name_and_caches() -> None:
    runner = _FakeRunner(stdout='{"board_name": "braiins,stm32mp157c-ii3-bmc1"}')
    dev = Device("h", runner=runner)
    assert dev.board == "braiins,stm32mp157c-ii3-bmc1"
    assert dev.board == "braiins,stm32mp157c-ii3-bmc1"
    assert len(runner.calls) == 1  # cached: only one ubus call


def test_version_reads_bos_version_and_is_not_cached() -> None:
    runner = _FakeRunner(stdout="2026-06-14-0-c84f1b1d-26.07-plus-nightly")
    dev = Device("h", runner=runner)
    assert dev.version == "2026-06-14-0-c84f1b1d-26.07-plus-nightly"
    _ = dev.version
    assert len(runner.calls) == 2  # re-read each access
    assert runner.calls[0][-1] == "cat /etc/bos_version"


def test_version_runs_even_under_dry_run() -> None:
    runner = _FakeRunner(stdout="v1")
    token = dry_run.set(True)
    try:
        assert Device("h", runner=runner).version == "v1"
    finally:
        dry_run.reset(token)
    assert runner.calls  # read still executed under dry-run


def test_target_parses_release_target_and_shares_board_cache() -> None:
    runner = _FakeRunner(
        stdout=(
            '{"board_name": "braiins,stm32mp157c-ii3-bmc1", "release": {"target": "stm32mp15/ii3"}}'
        )
    )
    dev = Device("h", runner=runner)
    assert dev.target == "stm32mp15/ii3"
    assert dev.board == "braiins,stm32mp157c-ii3-bmc1"
    assert len(runner.calls) == 1  # one ubus call backs both getters


def test_run_swallows_disconnect_when_expected() -> None:
    runner = _FakeRunner(error=subprocess.CalledProcessError(255, ["ssh"]))
    Device("h", runner=runner).run("sysupgrade x", expect_disconnect=True)  # no raise


def test_run_reraises_disconnect_when_not_expected() -> None:
    runner = _FakeRunner(error=subprocess.CalledProcessError(255, ["ssh"]))
    with pytest.raises(subprocess.CalledProcessError):
        Device("h", runner=runner).run("sysupgrade x")


def test_print_shows_host(capsys: pytest.CaptureFixture[str]) -> None:
    Device("192.168.1.183", runner=_FakeRunner()).print()
    assert "192.168.1.183" in capsys.readouterr().out
