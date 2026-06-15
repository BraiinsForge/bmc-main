"""Unit tests for the staged-procedure engine."""

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


def test_dry_run_defaults_false() -> None:
    assert dry_run.get() is False
