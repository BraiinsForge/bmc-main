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

"""Consume a device flash's `@bmc {json}` progress stream and render it live.

An on-device `sysupgrade` emits its Nix-staging progress as `@bmc {json}` lines
(schema: `bmc-nix/src/progress.rs`). Run directly over SSH those lines have no
consumer and flood the terminal unthrottled — one download re-emits the same
snapshot thousands of times. This parses them, collapses them into a bounded
live region, and tees the raw stream to a log.

`parse_line` mirrors the Rust contract: a line without the prefix, or one whose
payload cannot be classified, returns None — raw device output the caller echoes.
"""

import json
import os
import subprocess
import tempfile
from collections import deque
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from pathlib import Path

from rich.markup import escape

from bmc_tui import console
from bmc_tui.device import Device
from bmc_tui.stage import Abort

# Prefix every progress line carries (bmc-nix/src/progress.rs::BMC_PREFIX).
BMC_PREFIX = "@bmc "

# Friendlier labels for the GC sub-phases; upgrade phase names are already words.
_GC_PHASE_LABELS = {
    "finding_roots": "collecting garbage",
    "determining_liveness": "determining live/dead paths",
}

ERROR_TAIL = 20


@dataclass(frozen=True)
class Phase:
    name: str


@dataclass(frozen=True)
class RealizationStarted:
    total_paths: int


@dataclass(frozen=True)
class Download:
    downloaded_bytes: int
    total_bytes: int | None


@dataclass(frozen=True)
class RealizationFinished:
    """Consumed so the `@bmc` line never leaks as raw JSON; nothing to render."""


@dataclass(frozen=True)
class GcPhase:
    name: str


@dataclass(frozen=True)
class GcProgress:
    deleted_paths: int


@dataclass(frozen=True)
class GcFinished:
    deleted_paths: int
    freed_bytes: int | None


Event = (
    Phase | RealizationStarted | Download | RealizationFinished | GcPhase | GcProgress | GcFinished
)


def parse_line(line: str) -> Event | None:
    """Parse one `@bmc {json}` line into an event, or None for a line the caller
    should echo verbatim: no prefix, malformed, or an unknown `type`. Every known
    `type` yields an event so it is consumed, not echoed; unknown fields are
    tolerated (forward-compat)."""
    payload = line.rstrip("\r\n")
    if not payload.startswith(BMC_PREFIX):
        return None
    try:
        obj = json.loads(payload.removeprefix(BMC_PREFIX))
    except json.JSONDecodeError:
        return None
    if not isinstance(obj, dict):
        return None
    return _event(obj)


def _count(obj: dict[str, object], key: str) -> int | None:
    """The value at `key` as a non-negative int, else None. Rejects bool and
    negatives (the wire counts are unsigned) and maps a JSON `null` optional to
    None — so None means 'absent, invalid, or null' alike."""
    value = obj.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        return None
    return value


def _phase(obj: dict[str, object]) -> Event | None:
    phase = obj.get("phase")
    return Phase(phase) if isinstance(phase, str) else None


def _gc_phase(obj: dict[str, object]) -> Event | None:
    phase = obj.get("phase")
    return GcPhase(phase) if isinstance(phase, str) else None


def _realization_started(obj: dict[str, object]) -> Event | None:
    total = _count(obj, "total_paths")
    return RealizationStarted(total) if total is not None else None


def _download(obj: dict[str, object]) -> Event | None:
    done = _count(obj, "downloaded_bytes")
    return Download(done, _count(obj, "total_bytes")) if done is not None else None


def _gc_progress(obj: dict[str, object]) -> Event | None:
    deleted = _count(obj, "deleted_paths")
    return GcProgress(deleted) if deleted is not None else None


def _gc_finished(obj: dict[str, object]) -> Event | None:
    deleted = _count(obj, "deleted_paths")
    return GcFinished(deleted, _count(obj, "freed_bytes")) if deleted is not None else None


def _realization_finished(_obj: dict[str, object]) -> Event:
    return RealizationFinished()


_EVENT_BUILDERS: dict[str, Callable[[dict[str, object]], Event | None]] = {
    "phase": _phase,
    "realization_started": _realization_started,
    "download": _download,
    "realization_finished": _realization_finished,
    "gc_phase": _gc_phase,
    "gc_progress": _gc_progress,
    "gc_finished": _gc_finished,
}


def _event(obj: dict[str, object]) -> Event | None:
    kind = obj.get("type")
    if not isinstance(kind, str):
        return None
    builder = _EVENT_BUILDERS.get(kind)
    return builder(obj) if builder is not None else None


def _apply(event: Event, live: console.LiveScrollback) -> None:
    match event:
        case Phase(name):
            live.phase(name)
        case RealizationStarted(total_paths):
            live.realize(total_paths)
        case Download(done, total):
            live.download(done, total)
        case RealizationFinished():
            live.realized()
        case GcPhase(name):
            live.phase(_GC_PHASE_LABELS.get(name, name))
        case GcProgress(deleted):
            live.gc(deleted)
        case GcFinished(deleted, freed):
            live.gc(deleted, freed)


def stream_flash(dev: Device, command: str) -> Path:
    """Run a device flash, rendering its `@bmc` progress in a bounded live region
    and teeing every line to a log; return the log path.

    On failure, frame the collected error lines and raise `Abort` at the log. The
    stream is never captured, so the generic handler can't re-flood it."""
    # mkstemp, not a pid-derived name: it creates exclusively, so a second flash
    # in one process cannot truncate the first log and a planted symlink in a
    # shared /tmp is not followed.
    fd, name = tempfile.mkstemp(prefix="deck-sysupgrade-", suffix=".log")
    log_path = Path(name)
    errors: deque[str] = deque(maxlen=ERROR_TAIL)
    failure: subprocess.CalledProcessError | None = None

    with (
        os.fdopen(fd, "w", encoding="utf-8") as log,
        console.live_scrollback(log_path=log_path, window=30) as live,
    ):

        def on_line(line: str) -> None:
            log.write(f"{line}\n")
            event = parse_line(line)
            if event is None:
                # Blank lines are shown but not retained: inside a bounded tail
                # they would evict the error text the panel exists to show.
                if line.strip():
                    errors.append(line)
                live.echo(line)
            else:
                _apply(event, live)

        try:
            dev.run_streamed(command, on_line=on_line, expect_disconnect=True)
        except subprocess.CalledProcessError as e:
            failure = e

    if failure is not None:
        _report_failure(errors, log_path)
        raise Abort(
            f"firmware flash failed (exit {failure.returncode}) — see {console.lit(log_path)}"
        ) from failure
    return log_path


def _report_failure(errors: Iterable[str], log_path: Path) -> None:
    body = "\n".join(escape(line) for line in errors) or "(no error text captured)"
    console.panel(f"{body}\n\nsee {console.lit(log_path)}", title="Flash failed", style="red")
