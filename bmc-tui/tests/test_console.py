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
