"""Unit tests for the staged-procedure engine."""

import subprocess
from dataclasses import dataclass

import pytest

from bmc_tui.stage import Abort, done_if, dry_run, ensure, entrypoint, require, stage

# ── guard verbs ──────────────────────────────────────────────────────────────


def test_require_passes_when_true() -> None:
    require(True, "unused")


def test_require_aborts_when_false() -> None:
    with pytest.raises(Abort, match="boom"):
        require(False, "boom")


def test_ensure_skips_remedy_when_check_true() -> None:
    ran: list[int] = []
    ensure(lambda: True, lambda: ran.append(1))
    assert ran == []


def test_ensure_runs_remedy_then_rechecks() -> None:
    state = {"ok": False}

    def remedy() -> None:
        state["ok"] = True

    ensure(lambda: state["ok"], remedy)
    assert state["ok"] is True


def test_ensure_aborts_when_remedy_does_not_satisfy() -> None:
    with pytest.raises(Abort, match="still bad"):
        ensure(lambda: False, lambda: None, "still bad")


# ── stage wrapper ────────────────────────────────────────────────────────────


def test_stage_runs_step_and_reports_ok() -> None:
    ran: list[str] = []

    @stage("Demo")
    def demo() -> None:
        ran.append("step")

    demo()
    assert ran == ["step"]


def test_stage_skips_rest_on_done_if() -> None:
    ran: list[str] = []

    @stage("Demo")
    def demo() -> None:
        done_if(True)
        ran.append("step")

    demo()
    assert ran == []


def test_stage_reraises_abort() -> None:
    @stage("Demo")
    def demo() -> None:
        require(False, "nope")

    with pytest.raises(Abort, match="nope"):
        demo()


# ── entrypoint + tyro args ───────────────────────────────────────────────────


@dataclass
class _Args:
    device: str
    force: bool = False


def test_entrypoint_parses_args_and_runs() -> None:
    seen: dict[str, object] = {}

    @entrypoint
    def main(args: _Args) -> None:
        seen.update(device=args.device, force=args.force)

    main(["--device", "1.2.3.4", "--force"])
    assert seen == {"device": "1.2.3.4", "force": True}


def test_entrypoint_aborts_with_exit_1() -> None:
    @entrypoint
    def main(args: _Args) -> None:
        raise Abort("kaboom")

    with pytest.raises(SystemExit) as exc:
        main(["--device", "x"])
    assert exc.value.code == 1


def test_entrypoint_bad_args_exits_nonzero() -> None:
    @entrypoint
    def main(args: _Args) -> None: ...

    with pytest.raises(SystemExit) as exc:
        main([])  # missing required --device
    assert exc.value.code != 0


def test_entrypoint_renders_captured_output_on_command_failure(
    capsys: pytest.CaptureFixture[str],
) -> None:
    @entrypoint
    def main(args: _Args) -> None:
        raise subprocess.CalledProcessError(
            1,
            ["ssh", "root@device", "bmc-nix-cli add-packages"],
            output="conflicting symlink /profile/bin/flip-clock\n",
            stderr="Error: profile activation failed\n",
        )

    with pytest.raises(SystemExit) as exc:
        main(["--device", "x"])
    assert exc.value.code == 1
    # Header and captured output must share stderr — a redirect like
    # `2>err.log` must not split the error from the output explaining it.
    err = capsys.readouterr().err
    assert "conflicting symlink /profile/bin/flip-clock" in err
    assert "Error: profile activation failed" in err
    assert "exit 1" in err


def test_entrypoint_renders_bytes_stderr_on_command_failure(
    capsys: pytest.CaptureFixture[str],
) -> None:
    @entrypoint
    def main(args: _Args) -> None:
        raise subprocess.CalledProcessError(
            1, ["ssh", "root@device", "cat > /tmp/fw.tar"], stderr=b"dd: no space left\n"
        )

    with pytest.raises(SystemExit) as exc:
        main(["--device", "x"])
    assert exc.value.code == 1
    assert "dd: no space left" in capsys.readouterr().err


def test_entrypoint_renders_signal_death_on_command_failure(
    capsys: pytest.CaptureFixture[str],
) -> None:
    @entrypoint
    def main(args: _Args) -> None:
        # Negative returncode is subprocess's encoding for "killed by signal".
        raise subprocess.CalledProcessError(-15, ["ssh", "root@device", "sleep 100"])

    with pytest.raises(SystemExit) as exc:
        main(["--device", "x"])
    assert exc.value.code == 1
    err = capsys.readouterr().err
    assert "killed by signal 15" in err
    assert "exit -15" not in err


def test_dry_run_defaults_false() -> None:
    assert dry_run.get() is False
