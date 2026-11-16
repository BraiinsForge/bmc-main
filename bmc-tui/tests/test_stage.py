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

import os
import pty
import select
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from types import SimpleNamespace
from typing import final

import pytest
from rich.console import Console
from rich.text import Text

from bmc_tui import console, exits, ssh
from bmc_tui.stage import Abort, best_effort, done_if, dry_run, ensure, entrypoint, require, stage

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


def test_best_effort_runs_action() -> None:
    ran: list[int] = []
    best_effort(lambda: ran.append(1))
    assert ran == [1]


def test_best_effort_warns_on_stderr_and_swallows_the_failure(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """A swallowed failure is a diagnostic, so it shares the stream with the
    error it must not mask — a redirect capturing stdout would lose it."""

    def boom() -> None:
        raise RuntimeError("kaboom")

    best_effort(boom)
    captured = capsys.readouterr()
    assert "cleanup failed: kaboom" in captured.err
    assert captured.out == ""


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


class _Alerts:
    def __init__(self, monkeypatch: pytest.MonkeyPatch) -> None:
        self.summaries: list[str] = []
        self.bodies: list[str | None] = []
        monkeypatch.setattr(console, "alert", self.alert)

    def alert(self, summary: str, *, body: str | None = None, level: str = "info") -> None:
        _ = level
        self.summaries.append(summary)
        self.bodies.append(body)


def test_the_finished_alert_says_how_long_the_run_took(monkeypatch: pytest.MonkeyPatch) -> None:
    alerts = _Alerts(monkeypatch)
    clock = iter((10.0, 262.0))
    monkeypatch.setattr("bmc_tui.stage.time", SimpleNamespace(monotonic=lambda: next(clock)))

    @entrypoint
    def main() -> None:
        pass

    main()

    assert alerts.bodies == ["took 4 min 12 s"]


def test_the_finished_alert_names_the_module_by_default(monkeypatch: pytest.MonkeyPatch) -> None:
    alerts = _Alerts(monkeypatch)

    @entrypoint
    def main() -> None:
        pass

    main()

    assert alerts.summaries == ["test_stage finished"]


def test_the_finished_alert_comes_after_the_cleanups(monkeypatch: pytest.MonkeyPatch) -> None:
    alerts = _Alerts(monkeypatch)
    alerted_by_cleanup: list[list[str]] = []

    @entrypoint
    def main() -> None:
        exits.on_exit("undo", lambda: alerted_by_cleanup.append(list(alerts.summaries)))

    main()

    assert alerted_by_cleanup == [[]]
    assert alerts.summaries == ["test_stage finished"]


def test_a_run_whose_cleanup_fails_fails(
    capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    alerts = _Alerts(monkeypatch)
    monkeypatch.setattr(console, "console", Console(stderr=True, width=120))

    def fail() -> None:
        raise RuntimeError("still open")

    @entrypoint
    def main() -> None:
        exits.on_exit("closing", fail)

    with pytest.raises(SystemExit) as exc:
        main()

    assert exc.value.code == 1, "in CI or an unattended run, exit 0 reads as a pass"
    assert alerts.summaries == ["run failed"]
    assert alerts.bodies == ["closing failed"]
    assert "✗ run failed: closing failed" in Text.from_ansi(capsys.readouterr().err).plain, (
        "the alert stays silent off a terminal and on a quick run"
    )


@pytest.mark.parametrize(
    ("failure", "code", "bodies"),
    [(Abort("kaboom"), 1, ["kaboom"]), (KeyboardInterrupt(), 130, [])],
    ids=["aborted", "interrupted"],
)
def test_a_run_that_fails_keeps_its_exit_code_when_a_cleanup_fails_too(
    monkeypatch: pytest.MonkeyPatch, failure: BaseException, code: int, bodies: list[str]
) -> None:
    alerts = _Alerts(monkeypatch)

    def fail() -> None:
        raise RuntimeError("still open")

    @entrypoint
    def main() -> None:
        exits.on_exit("closing", fail)
        raise failure

    with pytest.raises(SystemExit) as exc:
        main()

    assert exc.value.code == code
    assert alerts.bodies == bodies, "no alert of its own for the cleanup"


def test_a_given_name_replaces_the_module_of_a_script_run_with_dash_m(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    alerts = _Alerts(monkeypatch)

    def main() -> None:
        pass

    main.__module__ = "__main__"

    entrypoint(main, name="intl sweep")()

    assert alerts.summaries == ["intl sweep finished"]


def test_entrypoint_aborts_with_exit_1() -> None:
    @entrypoint
    def main(_args: _Args) -> None:
        raise Abort("kaboom")

    with pytest.raises(SystemExit) as exc:
        main(["--device", "x"])
    assert exc.value.code == 1


@pytest.mark.parametrize("ending", ["normally", "with an abort", "by SIGTERM"])
def test_a_run_closes_its_ssh_connections_however_it_ends(
    monkeypatch: pytest.MonkeyPatch, ending: str
) -> None:
    sent: list[list[str]] = []

    @final
    class _Recording:
        def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
            sent.append(argv)
            return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")

    monkeypatch.setattr(ssh, "_TimedRunner", _Recording)
    sockets: list[Path] = []

    @entrypoint
    def main(_args: _Args) -> None:
        shared = ssh.connections.get()
        assert shared is not None, "a run shares its connections"
        option = next(o for o in shared.options("root@deck") if o.startswith("ControlPath="))
        path = Path(option.removeprefix("ControlPath="))
        path.touch()
        sockets.append(path.parent)
        if ending == "with an abort":
            raise Abort("stopped")
        if ending == "by SIGTERM":
            os.kill(os.getpid(), signal.SIGTERM)

    if ending == "normally":
        main(["--device", "x"])
    else:
        with pytest.raises(SystemExit):
            main(["--device", "x"])

    assert [argv[-1] for argv in sent if "exit" in argv] == ["root@deck"]
    assert not sockets[0].exists()


_PRINTS_UNTIL_HUNG_UP = """
import sys, time
from pathlib import Path
from bmc_tui import console
from bmc_tui.stage import best_effort, entrypoint

def main() -> None:
    try:
        print("ready", flush=True)
        while True:
            console.warn("working")
            time.sleep(0.05)
    finally:
        best_effort(lambda: console.warn("restoring"))
        Path(sys.argv[1]).write_text("restored")

entrypoint(main, name="hung-up")([])
"""


def _exit_code_within(pid: int, timeout_s: float) -> int:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        done, status = os.waitpid(pid, os.WNOHANG)
        if done:
            return os.waitstatus_to_exitcode(status)
        time.sleep(0.05)
    os.kill(pid, signal.SIGKILL)
    _ = os.waitpid(pid, 0)
    pytest.fail(f"the run did not end within {timeout_s:.0f}s of its terminal closing")


def test_a_run_whose_terminal_closes_still_finishes_restoring(tmp_path: Path) -> None:
    script = tmp_path / "run.py"
    _ = script.write_text(_PRINTS_UNTIL_HUNG_UP)
    marker = tmp_path / "restored"

    pid, terminal = pty.fork()
    if pid == 0:
        os.execv(sys.executable, [sys.executable, str(script), str(marker)])
    try:
        said = b""
        deadline = time.monotonic() + 10
        while b"ready" not in said:
            assert time.monotonic() < deadline, f"the run never got going: {said!r}"
            if select.select([terminal], [], [], 0.5)[0]:
                said += os.read(terminal, 4096)
    finally:
        os.close(terminal)

    assert _exit_code_within(pid, 10) == 128 + signal.SIGHUP
    assert marker.read_text() == "restored", "what printed after the hangup cut nothing short"


def test_a_terminating_signal_ends_the_run_through_the_normal_exit() -> None:
    before = signal.getsignal(signal.SIGTERM)
    ran: list[str] = []

    @entrypoint
    def main(_args: _Args) -> None:
        exits.on_exit("undo", lambda: ran.append("undo"))
        os.kill(os.getpid(), signal.SIGTERM)
        pytest.fail("the signal must end the run")

    with pytest.raises(SystemExit) as exc:
        main(["--device", "x"])

    assert exc.value.code == 128 + signal.SIGTERM
    assert ran == ["undo"], "the run's cleanups run, unlike under the default death by signal"
    assert signal.getsignal(signal.SIGTERM) is before, "the previous handler comes back"


@final
class _MasterWontExit:
    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        if "exit" in argv:
            raise subprocess.CalledProcessError(255, argv, stderr="read from master failed")
        return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")


def test_a_stuck_connection_s_commands_come_out_whole_on_a_narrow_terminal(
    capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    # Sized here, as the environment's COLUMNS or a dumb TERM would override it
    monkeypatch.setattr(console, "console", Console(stderr=True, width=40, height=25))
    shared = ssh.SharedConnections(_MasterWontExit())
    option = next(o for o in shared.options("root@a") if o.startswith("ControlPath="))
    path = Path(option.removeprefix("ControlPath="))

    @entrypoint
    def main(_args: _Args) -> None:
        exits.on_exit("closing", shared.close)

    try:
        with socket.socket(socket.AF_UNIX) as master:
            master.bind(str(path))
            master.listen()
            with pytest.raises(SystemExit):
                main(["--device", "x"])
    finally:
        shutil.rmtree(path.parent)

    lines = Text.from_ansi(capsys.readouterr().err).plain.splitlines()
    warned = [i for i, line in enumerate(lines) if line.startswith("⚠ closing failed:")]
    assert warned, "reported as a warning, not by the fallback for a terminal that closed"
    assert lines[warned[0] + 1 : warned[0] + 3] == [
        f"pkill -KILL -f '[/]{str(path.parent)[1:]}/'",
        f"rm -r '{path.parent}'",
    ], "a command wrapped apart runs as two"


def test_entrypoint_bad_args_exits_nonzero() -> None:
    @entrypoint
    def main(_args: _Args) -> None: ...

    with pytest.raises(SystemExit) as exc:
        main([])  # missing required --device
    assert exc.value.code != 0


def test_entrypoint_renders_captured_output_on_command_failure(
    capsys: pytest.CaptureFixture[str],
) -> None:
    @entrypoint
    def main(_args: _Args) -> None:
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
    def main(_args: _Args) -> None:
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
    def main(_args: _Args) -> None:
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


def _capture_excepthook(monkeypatch: pytest.MonkeyPatch) -> list[type[BaseException]]:
    seen: list[type[BaseException]] = []

    def excepthook(kind: type[BaseException], *_rest: object) -> None:
        seen.append(kind)

    monkeypatch.setattr(sys, "excepthook", excepthook)
    return seen


def test_a_worker_crash_is_rendered_like_the_main_thread(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """sys.excepthook is never consulted for a thread; entrypoint bridges it."""
    seen = _capture_excepthook(monkeypatch)

    def boom() -> None:
        raise ValueError("from a worker")

    worker = threading.Thread(target=boom)
    worker.start()
    worker.join()
    assert seen == [ValueError]


def test_a_worker_exiting_on_purpose_is_not_a_crash(monkeypatch: pytest.MonkeyPatch) -> None:
    seen = _capture_excepthook(monkeypatch)

    def quit_thread() -> None:
        raise SystemExit(0)

    worker = threading.Thread(target=quit_thread)
    worker.start()
    worker.join()
    assert seen == []
