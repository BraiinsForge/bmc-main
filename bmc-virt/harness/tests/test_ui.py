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

"""Unit tests for ui.instruct_user — no VM required."""

import io
from datetime import datetime

import pytest

from bmc_virt import ui


class _FakeStdin:
    """Stand-in for ``sys.stdin`` with a controllable ``isatty`` result."""

    def __init__(self, *, tty: bool) -> None:
        self._tty = tty

    def isatty(self) -> bool:
        return self._tty


class _RecordingCapture:
    """Minimal ``SupportsCapture`` implementation that records its labels."""

    def __init__(self) -> None:
        self.labels: list[str] = []

    def capture(self, label: str = "") -> object:
        self.labels.append(label)
        return None


@pytest.fixture
def tty_stdin(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("sys.stdin", _FakeStdin(tty=True))
    # ``input()`` reads from the real stdin by default; replace it with a
    # canned no-op that simulates the user pressing Enter immediately.
    monkeypatch.setattr("builtins.input", lambda *_a, **_kw: "")


def test_returns_timezone_aware_timestamp(tty_stdin: None) -> None:
    before = datetime.now().astimezone()
    ts = ui.instruct_user("Tap +D20")
    after = datetime.now().astimezone()

    assert ts.tzinfo is not None
    assert before <= ts <= after


def test_captures_prompt_and_ack_when_metrics_given(tty_stdin: None) -> None:
    metrics = _RecordingCapture()

    ui.instruct_user("+D20", metrics=metrics)

    assert metrics.labels == [">+D20", "<+D20"]


def test_no_metrics_calls_when_metrics_omitted(tty_stdin: None) -> None:
    # Just exercises the no-metrics path; the assertion is implicit
    # (no exception, no recorded labels because there's no recorder).
    ts = ui.instruct_user("Confirm device is on screen")
    assert isinstance(ts, datetime)


def test_raises_when_stdin_is_not_a_tty(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("sys.stdin", _FakeStdin(tty=False))
    monkeypatch.setattr("builtins.input", lambda *_a, **_kw: "")

    with pytest.raises(RuntimeError, match="requires a TTY"):
        ui.instruct_user("Tap +D20")


def test_raises_before_capturing_when_not_a_tty(monkeypatch: pytest.MonkeyPatch) -> None:
    """Non-TTY path must fail before any metrics side effect."""
    monkeypatch.setattr("sys.stdin", _FakeStdin(tty=False))
    monkeypatch.setattr("builtins.input", lambda *_a, **_kw: "")
    metrics = _RecordingCapture()

    with pytest.raises(RuntimeError):
        ui.instruct_user("Tap +D20", metrics=metrics)

    assert metrics.labels == []


def test_panel_is_rendered_to_stdout(tty_stdin: None, monkeypatch: pytest.MonkeyPatch) -> None:
    """Smoke test: the instruction message reaches the user-visible console."""
    buffer = io.StringIO()
    monkeypatch.setattr(ui.out, "file", buffer)

    ui.instruct_user("Tap +D20")

    rendered = buffer.getvalue()
    assert "Tap +D20" in rendered
    assert ui.INSTRUCT_HINT in rendered
