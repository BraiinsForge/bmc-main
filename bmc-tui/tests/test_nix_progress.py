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

"""Unit tests for the @bmc progress consumer."""

from collections.abc import Callable, Iterable
from pathlib import Path
from subprocess import CompletedProcess

import pytest

from bmc_tui import nix_progress
from bmc_tui.device import Device
from bmc_tui.nix_progress import (
    Download,
    GcFinished,
    GcPhase,
    GcProgress,
    Phase,
    RealizationFinished,
    RealizationStarted,
    parse_line,
)
from bmc_tui.stage import Abort

_DOWNLOAD = (
    '@bmc {"type":"download","downloaded_bytes":1024,'
    '"total_bytes":4096,"remaining_bytes":3072,"active":[]}'
)


class _Exec:
    """Fake Exec whose stream_output feeds canned lines then returns a code."""

    def __init__(self, lines: list[str], code: int) -> None:
        self._lines = lines
        self._code = code

    def run(self, argv: list[str]) -> CompletedProcess[str]:
        raise NotImplementedError

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        raise NotImplementedError

    def stream_output(self, argv: list[str], on_line: Callable[[str], None]) -> int:
        for line in self._lines:
            on_line(line)
        return self._code


# ── parse_line ────────────────────────────────────────────────────────────────


def test_parse_line_reads_every_event_type() -> None:
    assert parse_line('@bmc {"type":"phase","phase":"realizing"}') == Phase("realizing")
    assert parse_line('@bmc {"type":"realization_started","total_paths":3}') == RealizationStarted(
        3
    )
    assert parse_line(_DOWNLOAD) == Download(1024, 4096)
    assert parse_line('@bmc {"type":"realization_finished"}') == RealizationFinished()
    assert parse_line('@bmc {"type":"gc_phase","phase":"finding_roots"}') == GcPhase(
        "finding_roots"
    )
    assert parse_line('@bmc {"type":"gc_progress","deleted_paths":300}') == GcProgress(300)
    assert parse_line('@bmc {"type":"gc_finished","deleted_paths":2,"freed_bytes":9999}') == (
        GcFinished(2, 9999)
    )


def test_parse_line_reads_null_optionals_as_none() -> None:
    line = (
        '@bmc {"type":"download","downloaded_bytes":5,'
        '"total_bytes":null,"remaining_bytes":null,"active":[]}'
    )
    assert parse_line(line) == Download(5, None)
    assert parse_line('@bmc {"type":"gc_finished","deleted_paths":0,"freed_bytes":null}') == (
        GcFinished(0, None)
    )


def test_parse_line_returns_none_for_non_bmc_and_malformed() -> None:
    assert parse_line("") is None
    assert parse_line("fwtool: writing image") is None
    assert parse_line("Image check failed.") is None
    assert parse_line("@bmc ") is None
    assert parse_line('@bmc {"type":') is None
    assert parse_line("@bmc not json") is None
    assert parse_line("@bmc [1,2,3]") is None


def test_parse_line_rejects_unknown_type_and_negative_counts() -> None:
    assert parse_line('@bmc {"type":"from_the_future"}') is None
    assert parse_line('@bmc {"type":"realization_started","total_paths":-1}') is None


def test_parse_line_tolerates_unknown_fields_and_crlf() -> None:
    forward_compatible = (
        '@bmc {"type":"download","downloaded_bytes":1,"total_bytes":null,'
        '"remaining_bytes":null,"active":[],"added_in_the_future":true}\r\n'
    )
    assert parse_line(forward_compatible) == Download(1, None)


# ── stream_flash ──────────────────────────────────────────────────────────────

_MIXED = [
    '@bmc {"type":"phase","phase":"realizing"}',
    '@bmc {"type":"realization_started","total_paths":2}',
    _DOWNLOAD,
    '@bmc {"type":"realization_finished"}',
    '@bmc {"type":"gc_phase","phase":"finding_roots"}',
    '@bmc {"type":"gc_progress","deleted_paths":5}',
    '@bmc {"type":"gc_finished","deleted_paths":5,"freed_bytes":2048}',
    "fwtool: writing image",
]


def _log_path(tmp_path: Path) -> Path:
    """The one log the run created; its name is generated, not derived."""
    (log,) = tmp_path.glob("deck-sysupgrade-*.log")
    return log


def test_stream_flash_tees_every_line_to_the_log(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(nix_progress.tempfile, "gettempdir", lambda: str(tmp_path))
    dev = Device("h", backend=_Exec(_MIXED, code=0))
    log = nix_progress.stream_flash(dev, "sysupgrade x")
    assert log.parent == tmp_path
    assert log.read_text(encoding="utf-8").splitlines() == _MIXED


def test_each_flash_gets_its_own_log(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """A pid-derived name is reused within one process, so a second flash would
    truncate the first run's log — the one record of what went wrong."""
    monkeypatch.setattr(nix_progress.tempfile, "gettempdir", lambda: str(tmp_path))
    first = nix_progress.stream_flash(Device("h", backend=_Exec(["one"], code=0)), "sysupgrade x")
    second = nix_progress.stream_flash(Device("h", backend=_Exec(["two"], code=0)), "sysupgrade x")
    assert first != second
    assert first.read_text(encoding="utf-8").strip() == "one"
    assert second.read_text(encoding="utf-8").strip() == "two"


def test_stream_flash_frames_errors_and_points_at_the_log(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr(nix_progress.tempfile, "gettempdir", lambda: str(tmp_path))
    lines = [
        _DOWNLOAD,
        "Error: nix-store --realise failed",
        "Image check failed.",
    ]
    dev = Device("h", backend=_Exec(lines, code=1))

    with pytest.raises(Abort) as exc:
        nix_progress.stream_flash(dev, "sysupgrade x")

    log = _log_path(tmp_path)
    # The @bmc line drove the display yet is still in the log.
    assert log.read_text(encoding="utf-8").splitlines() == lines
    assert str(log) in exc.value.hint

    out = capsys.readouterr().out
    # Short tokens dodge any panel line wrapping at narrow console widths.
    assert "Image check failed." in out
    assert "nix-store" in out
    assert "@bmc" not in out


def test_the_error_tail_is_bounded_while_it_is_collected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Reporting only ever shows the tail, so bounding it at display time would
    hide unbounded growth: assert on what collection hands over, not what prints."""
    monkeypatch.setattr(nix_progress.tempfile, "gettempdir", lambda: str(tmp_path))
    handed_over: list[list[str]] = []
    monkeypatch.setattr(
        nix_progress,
        "_report_failure",
        lambda errors, log_path: handed_over.append(list(errors)),
    )
    noise = [f"line {i}" for i in range(nix_progress.ERROR_TAIL * 3)]
    dev = Device("h", backend=_Exec(noise, code=1))

    with pytest.raises(Abort):
        nix_progress.stream_flash(dev, "sysupgrade x")

    (retained,) = handed_over
    assert len(retained) == nix_progress.ERROR_TAIL
    assert retained[-1] == f"line {len(noise) - 1}", "the newest line must survive"
