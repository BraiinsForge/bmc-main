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

"""Unit tests for the cleanup that runs however a run ends."""

import os
import signal
import subprocess

import pytest

from bmc_tui import exits


def test_cleanups_run_last_first_when_the_run_ends() -> None:
    ran: list[str] = []

    with exits.trapped():
        exits.on_exit("first", lambda: ran.append("first"))
        exits.on_exit("second", lambda: ran.append("second"))

    assert ran == ["second", "first"]


def test_cleanups_run_when_the_run_fails() -> None:
    ran: list[str] = []

    with pytest.raises(RuntimeError), exits.trapped():
        exits.on_exit("undo", lambda: ran.append("undo"))
        raise RuntimeError("boom")

    assert ran == ["undo"]


@pytest.mark.parametrize("signum", [signal.SIGTERM, signal.SIGHUP])
def test_a_terminating_signal_stops_the_run_and_still_cleans_up(signum: int) -> None:
    ran: list[str] = []
    before = signal.getsignal(signum)

    with pytest.raises(exits.Stopped) as stopped, exits.trapped():
        exits.on_exit("undo", lambda: ran.append("undo"))
        os.kill(os.getpid(), signum)
        pytest.fail("the signal must end the run")

    assert stopped.value.signum == signum
    assert ran == ["undo"]
    assert signal.getsignal(signum) is before, "the previous handler comes back"


@pytest.mark.parametrize("signum", [signal.SIGTERM, signal.SIGHUP])
def test_a_repeated_signal_does_not_cut_short_the_unwinding_the_first_started(
    signum: int,
) -> None:
    restored: list[str] = []
    before = signal.getsignal(signum)

    with pytest.raises(exits.Stopped), exits.trapped():
        try:
            os.kill(os.getpid(), signum)
        finally:
            # A second one, as a closed terminal's job gets SIGHUP from the kernel and from bash
            os.kill(os.getpid(), signum)
            restored.append("restored")

    assert restored == ["restored"]
    assert signal.getsignal(signum) is before, "the previous handler comes back"


@pytest.mark.parametrize("signum", [signal.SIGTERM, signal.SIGHUP])
def test_a_process_the_unwinding_starts_still_dies_of_the_signal(signum: int) -> None:
    codes: list[int] = []

    with pytest.raises(exits.Stopped), exits.trapped():
        try:
            os.kill(os.getpid(), signum)
        finally:
            # An ignored signal would stay ignored across its exec
            with subprocess.Popen(["sleep", "30"]) as child:
                try:
                    os.kill(child.pid, signum)
                    codes.append(child.wait(timeout=5))
                finally:
                    child.kill()

    assert codes == [-signum]


def test_a_hangup_after_another_signal_still_sends_the_terminal_to_devnull(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    sent: list[tuple[int, ...]] = []
    monkeypatch.setattr(exits, "_send_to_devnull", sent.append)

    with pytest.raises(exits.Stopped) as stopped, exits.trapped():
        try:
            os.kill(os.getpid(), signal.SIGTERM)
        finally:
            os.kill(os.getpid(), signal.SIGHUP)

    assert stopped.value.signum == signal.SIGTERM
    assert len(sent) == 1, "the next write to the closed terminal would fail with EIO"


def test_an_interrupt_during_the_cleanups_still_restores_the_handlers() -> None:
    before = signal.getsignal(signal.SIGTERM)

    def interrupt() -> None:
        raise KeyboardInterrupt

    with pytest.raises(KeyboardInterrupt), exits.trapped():
        exits.on_exit("interrupted cleanup", interrupt)

    assert signal.getsignal(signal.SIGTERM) is before


@pytest.mark.parametrize("signum", [signal.SIGINT, signal.SIGTERM, signal.SIGHUP])
def test_a_signal_during_the_cleanups_does_not_cut_the_rest_off(signum: int) -> None:
    ran: list[str] = []

    with exits.trapped():
        exits.on_exit("last", lambda: ran.append("last"))
        exits.on_exit("signalled", lambda: os.kill(os.getpid(), signum))

    assert ran == ["last"]


def test_a_signal_ignored_before_the_run_stays_ignored() -> None:
    before = signal.signal(signal.SIGHUP, signal.SIG_IGN)
    try:
        with exits.trapped():
            os.kill(os.getpid(), signal.SIGHUP)
            assert signal.getsignal(signal.SIGHUP) == signal.SIG_IGN, "as under `nohup`"
    finally:
        _ = signal.signal(signal.SIGHUP, before)


def test_the_outcome_names_each_cleanup_that_failed() -> None:
    def fail() -> None:
        raise RuntimeError("boom")

    with exits.trapped(report=lambda _label, _error: None) as outcome:
        exits.on_exit("first", fail)
        exits.on_exit("fine", lambda: None)
        exits.on_exit("last", fail)

    assert outcome.failed == ["last", "first"]


def test_a_report_that_fails_still_lets_the_rest_run() -> None:
    ran: list[str] = []

    def fail() -> None:
        raise OSError("no ssh")

    def refuse(_label: str, _error: Exception) -> None:
        raise OSError("the terminal is gone")

    with exits.trapped(report=refuse):
        exits.on_exit("last", lambda: ran.append("last"))
        exits.on_exit("failing", fail)

    assert ran == ["last"]


def test_a_failed_cleanup_is_reported_by_its_label_and_the_rest_still_run() -> None:
    reported: list[tuple[str, str]] = []
    ran: list[str] = []

    def fail() -> None:
        raise OSError("no ssh")

    with exits.trapped(report=lambda label, error: reported.append((label, str(error)))):
        exits.on_exit("first", lambda: ran.append("first"))
        exits.on_exit("closing the connection", fail)

    assert reported == [("closing the connection", "no ssh")]
    assert ran == ["first"]


def test_a_cleanup_outside_a_run_is_a_bug() -> None:
    with pytest.raises(RuntimeError, match="BUG"):
        exits.on_exit("undo", lambda: None)
