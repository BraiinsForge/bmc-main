"""Unit tests for console formatting helpers."""

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
