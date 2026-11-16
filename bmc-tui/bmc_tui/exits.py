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

"""Cleanup that runs however a run ends: normally, on an error, on Ctrl+C, or on SIGTERM or SIGHUP.

A run wraps itself in `trapped`, and code anywhere below registers what to undo with `on_exit`::

    with exits.trapped(report=lambda label, error: print(f"{label} failed: {error}")):
        exits.on_exit("close the tunnel", tunnel.close)
        ...
"""

import contextlib
import os
import signal
import sys
import threading
from collections.abc import Callable, Generator
from contextlib import contextmanager
from dataclasses import dataclass, field
from types import FrameType
from typing import final

# A closed terminal or a `kill`, which end the process without running `atexit`
_STOPPING_SIGNALS = (signal.SIGTERM, signal.SIGHUP)
_INTERRUPTING_SIGNALS = (signal.SIGINT, *_STOPPING_SIGNALS)

type Report = Callable[[str, Exception], None]


@final
class Stopped(KeyboardInterrupt):
    """A signal ended the run.
    Code that lets Ctrl+C through, or stops on it, does the same for this."""

    def __init__(self, signum: int) -> None:
        super().__init__(signal.Signals(signum).name)
        self.signum: int = signum


def _stopper(terminals: tuple[int, ...]) -> Callable[[int, FrameType | None], None]:
    stopping = False

    def stop(signum: int, _frame: FrameType | None) -> None:
        nonlocal stopping
        first = not stopping
        stopping = True
        if signum == signal.SIGHUP:
            # The terminal is gone, so the first write to it would fail with EIO
            # and cut short the cleanup it was reporting on
            _send_to_devnull(terminals)
        # A closed terminal's job gets SIGHUP twice, from the kernel and from bash,
        # and the second must not cut short the unwinding the first started
        if first:
            raise Stopped(signum)

    return stop


def _send_to_devnull(fds: tuple[int, ...]) -> None:
    devnull = os.open(os.devnull, os.O_WRONLY)
    try:
        for fd in fds:
            _ = os.dup2(devnull, fd)
    finally:
        os.close(devnull)


@dataclass(frozen=True)
class _Cleanup:
    label: str
    action: Callable[[], object]


def _report_to_stderr(label: str, error: Exception) -> None:
    print(f"{label} failed: {error}", file=sys.stderr)


_runs: list[list[_Cleanup]] = []


def on_exit(label: str, action: Callable[[], object]) -> None:
    """Run `action` when the enclosing `trapped` run ends. `label` names it if it fails."""
    if not _runs:
        # `atexit` would not run it on SIGTERM or SIGHUP
        msg = f"BUG: {label!r} registered outside a `trapped` run"
        raise RuntimeError(msg)
    _runs[-1].append(_Cleanup(label, action))


@dataclass
class Outcome:
    """How a `trapped` run's cleanups went, known once the block is left."""

    failed: list[str] = field(default_factory=list)


def _run(cleanups: list[_Cleanup], report: Report, outcome: Outcome) -> None:
    for cleanup in reversed(cleanups):
        # One failed cleanup must not skip the rest
        try:
            _ = cleanup.action()
        except Exception as error:
            outcome.failed.append(cleanup.label)
            try:
                report(cleanup.label, error)
            except Exception:
                # A closed terminal can refuse the report, and stderr is the last place left
                with contextlib.suppress(OSError):
                    _report_to_stderr(cleanup.label, error)


@contextmanager
def trapped(report: Report = _report_to_stderr) -> Generator[Outcome]:
    """Run the cleanups registered with `on_exit` inside, last first, however the block is left.
    Inside, SIGTERM and SIGHUP raise `Stopped`, unless they were ignored already, as by `nohup`.
    Only the first of them raises, so a repeat does not cut short the unwinding it started.
    `kill -9` ends an unwinding that hangs.
    On SIGHUP, what went to the terminal goes to /dev/null from then on.
    While the cleanups run, Ctrl+C and both signals are ignored, so none of the cleanups is cut off.
    A cleanup that fails goes to `report`, and its label to the `Outcome` the block gets."""
    # Only the main thread may set a signal handler
    on_main = threading.current_thread() is threading.main_thread()
    outcome = Outcome()
    cleanups: list[_Cleanup] = []
    _runs.append(cleanups)
    previous: dict[signal.Signals, Callable[[int, FrameType | None], object] | int] = {}
    try:
        if on_main:
            # Asked now, as a terminal that has hung up may no longer say it is one
            stop = _stopper(tuple(fd for fd in (1, 2) if os.isatty(fd)))
            previous = {
                sig: handler
                for sig in _INTERRUPTING_SIGNALS
                if (handler := signal.getsignal(sig)) is not None
            }
            for sig in _STOPPING_SIGNALS:
                if sig in previous and previous[sig] != signal.SIG_IGN:
                    _ = signal.signal(sig, stop)
        yield outcome
    finally:
        _ = _runs.pop()
        try:
            for sig in previous:
                _ = signal.signal(sig, signal.SIG_IGN)
            _run(cleanups, report, outcome)
        finally:
            for sig, handler in previous.items():
                _ = signal.signal(sig, handler)
