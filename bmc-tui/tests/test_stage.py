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

"""Unit tests for the staged-procedure engine."""

import subprocess
import sys
import threading
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


def test_a_worker_crash_is_rendered_like_the_main_thread(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """sys.excepthook is never consulted for a thread; entrypoint bridges it."""
    seen: list[type[BaseException]] = []
    monkeypatch.setattr(sys, "excepthook", lambda kind, *_rest: seen.append(kind))

    def boom() -> None:
        raise ValueError("from a worker")

    worker = threading.Thread(target=boom)
    worker.start()
    worker.join()
    assert seen == [ValueError]


def test_a_worker_exiting_on_purpose_is_not_a_crash(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[type[BaseException]] = []
    monkeypatch.setattr(sys, "excepthook", lambda kind, *_rest: seen.append(kind))

    def quit_thread() -> None:
        raise SystemExit(0)

    worker = threading.Thread(target=quit_thread)
    worker.start()
    worker.join()
    assert seen == []
