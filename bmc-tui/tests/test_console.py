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

import pytest

from bmc_tui import console
from bmc_tui.console import human_size, lit


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
    fired: list[tuple[str, str | None]] = []
    monkeypatch.setattr(console.sys.stdout, "isatty", lambda: False)
    monkeypatch.setattr(
        console, "desktop_notify", lambda summary, *, body=None: fired.append((summary, body))
    )
    console.notify("done", body="ready")
    assert fired == [("done", "ready")]


def test_desktop_notify_silent_without_notifier(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(console.shutil, "which", lambda _: None)

    def boom(*_a: object, **_k: object) -> None:
        raise AssertionError("must not shell out when no notifier is on PATH")

    monkeypatch.setattr(console.subprocess, "run", boom)
    console.desktop_notify("done")  # no notifier → silent no-op
