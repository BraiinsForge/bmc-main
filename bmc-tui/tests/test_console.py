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

"""Unit tests for console formatting helpers."""

import io
from pathlib import Path

import pytest
from rich.console import Console, RenderableType
from rich.live import Live

from bmc_tui import console
from bmc_tui.console import human_size, lit


def _rendered(renderable: RenderableType | None) -> str:
    """Text a renderable produces, so a test can assert what the user sees."""
    buf = io.StringIO()
    Console(file=buf, width=100, no_color=True).print(renderable)
    return buf.getvalue()


def test_human_size_bytes() -> None:
    assert human_size(500) == "500.0 B"


def test_human_size_mib() -> None:
    assert human_size(44_596_032) == "42.5 MiB"


def test_human_size_gib() -> None:
    assert human_size(3 * 1024**3) == "3.0 GiB"


def test_lit_wraps_in_literal_style() -> None:
    assert lit("/nix") == "[magenta]/nix[/magenta]"


def test_lit_escapes_markup() -> None:
    assert lit("a[b]") == "[magenta]a\\[b][/magenta]"


# ── live_scrollback ───────────────────────────────────────────────────────────


def test_live_scrollback_bounds_the_raw_tail() -> None:
    overshoot = console.SCROLLBACK_TAIL + 5
    with console.live_scrollback(log_path=Path("/tmp/flash.log")) as live:
        live.phase("realizing")
        live.realize(3)
        live.download(10, 100)
        live.gc(2, freed_bytes=1024)
        for i in range(overshoot):
            live.echo(f"line {i}")
        assert len(live.tail) == console.SCROLLBACK_TAIL
        assert live.tail[0] == f"line {overshoot - console.SCROLLBACK_TAIL}"
        assert live.tail[-1] == f"line {overshoot - 1}"


def test_the_realization_note_does_not_outlive_its_phase() -> None:
    """Realizing sets a note that nothing else clears, so without an explicit
    end the next phase renders beside a line still claiming it is realizing."""
    live = Live(console=Console(file=io.StringIO()), auto_refresh=False)
    region = console.LiveScrollback(live, Path("/tmp/flash.log"), tail=5)

    region.realize(42)
    assert "realizing 42 store paths" in _rendered(live.renderable)

    region.realized()
    region.phase("verifying")
    shown = _rendered(live.renderable)
    assert "verifying" in shown
    assert "realizing" not in shown


# ── notifications ─────────────────────────────────────────────────────────────


def test_confirm_returns_false_without_tty(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(console.sys.stdin, "isatty", lambda: False)
    assert console.confirm("flash it?") is False


def test_alert_silent_without_tty(monkeypatch: pytest.MonkeyPatch) -> None:
    fired: list[str] = []
    monkeypatch.setattr(console.sys.stdout, "isatty", lambda: False)
    monkeypatch.setattr(console, "desktop_notify", lambda *a, **k: fired.append("x"))
    console.alert("attention")
    assert not fired


def test_alert_silent_during_warmup(monkeypatch: pytest.MonkeyPatch) -> None:
    fired: list[str] = []
    monkeypatch.setattr(console.sys.stdout, "isatty", lambda: True)
    monkeypatch.setattr(console, "desktop_notify", lambda *a, **k: fired.append("x"))
    console.mark_run_start()
    console.alert("attention", after=3600.0)  # run is younger than the gate
    assert not fired


def test_notify_fires_desktop(monkeypatch: pytest.MonkeyPatch) -> None:
    fired: list[tuple[str, str | None, str]] = []
    monkeypatch.setattr(console.sys.stdout, "isatty", lambda: False)
    monkeypatch.setattr(
        console,
        "desktop_notify",
        lambda summary, *, body=None, level="info": fired.append((summary, body, level)),
    )
    console.notify("done", body="ready")
    assert fired == [("done", "ready", "info")]


def test_an_error_notification_is_critical(monkeypatch: pytest.MonkeyPatch) -> None:
    """A failure should survive on screen until it is dismissed."""
    sent: list[list[str]] = []
    monkeypatch.setattr(console.shutil, "which", lambda name: name == "notify-send")
    monkeypatch.setattr(console.subprocess, "run", lambda cmd, **_kw: sent.append(cmd))
    console.desktop_notify("run failed", body="boom", level="error")
    assert "--urgency" in sent[0]
    assert sent[0][sent[0].index("--urgency") + 1] == "critical"
    assert sent[0][sent[0].index("--icon") + 1] == "dialog-error"


@pytest.mark.parametrize("level", ["info", "success", "warn"])
def test_a_non_error_notification_still_raises_a_banner(
    level: console.Level, monkeypatch: pytest.MonkeyPatch
) -> None:
    """`low` is filed into the tray unseen, which defeats the point: every
    caller has already decided the user looked away."""
    sent: list[list[str]] = []
    monkeypatch.setattr(console.shutil, "which", lambda name: name == "notify-send")
    monkeypatch.setattr(console.subprocess, "run", lambda cmd, **_kw: sent.append(cmd))
    console.desktop_notify("finished", level=level)
    assert sent[0][sent[0].index("--urgency") + 1] == "normal"


def test_desktop_notify_silent_without_notifier(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(console.shutil, "which", lambda _: None)

    def boom(*_a: object, **_k: object) -> None:
        raise AssertionError("must not shell out when no notifier is on PATH")

    monkeypatch.setattr(console.subprocess, "run", boom)
    console.desktop_notify("done")  # no notifier → silent no-op
