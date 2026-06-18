"""Unit tests for the local nix seam."""

import subprocess

import pytest

from bmc_tui import nix
from bmc_tui.nix import Built, Pkg
from bmc_tui.stage import dry_run


def _cp(argv: list[str], stdout: str = "") -> "subprocess.CompletedProcess[str]":
    return subprocess.CompletedProcess(argv, 0, stdout=stdout, stderr="")


def test_resolve_index_package(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(nix, "_eval_ok", lambda _expr: True)
    monkeypatch.setattr(nix, "_eval_raw", lambda _expr: "2.0")
    assert nix.real().resolve(".#deck-packages.core") == Pkg(
        "core", "2.0", ".#deck-packages.core.pkg^out"
    )


def test_resolve_raw_nixpkgs(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(nix, "_eval_ok", lambda _expr: False)
    monkeypatch.setattr(
        nix, "_eval_raw", lambda expr: "9.0" if expr.endswith(".version") else "file"
    )
    assert nix.real().resolve(".#armv7-nixpkgs.file") == Pkg(
        "nixpkgs-file", "9.0", ".#armv7-nixpkgs.file^out"
    )


def test_build_maps_paths_to_packages_in_order(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[list[str]] = []

    def run(argv: list[str], **_: object) -> "subprocess.CompletedProcess[str]":
        seen.append(argv)
        return _cp(argv, "/nix/store/core\n/nix/store/clock\n")

    monkeypatch.setattr(nix.subprocess, "run", run)
    built = nix.real().build([Pkg("core", "1.0", ".#core^out"), Pkg("clock", "2.0", ".#clock^out")])
    assert built == [
        Built("core", "1.0", ".#core^out", "/nix/store/core"),
        Built("clock", "2.0", ".#clock^out", "/nix/store/clock"),
    ]
    # One invocation for the whole set, both installables passed through.
    assert seen == [["nix", "build", "--no-link", "--print-out-paths", ".#core^out", ".#clock^out"]]


def test_build_passes_max_jobs_when_set(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[list[str]] = []
    monkeypatch.setattr(
        nix.subprocess, "run", lambda argv, **_: seen.append(argv) or _cp(argv, "/nix/store/abc\n")
    )
    nix.real(max_jobs=4).build([Pkg("core", "1.0", ".#x^out")])
    assert "--max-jobs" in seen[0] and seen[0][seen[0].index("--max-jobs") + 1] == "4"


def test_build_rejects_path_count_mismatch(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(nix.subprocess, "run", lambda *a, **_: _cp(list(a[0]), "/nix/store/only\n"))
    with pytest.raises(RuntimeError, match="printed 1 paths for 2"):
        nix.real().build([Pkg("a", "1", ".#a^out"), Pkg("b", "1", ".#b^out")])


def test_build_out_returns_the_lone_path(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[list[str]] = []
    monkeypatch.setattr(
        nix.subprocess,
        "run",
        lambda argv, **_: seen.append(argv) or _cp(argv, "/nix/store/init-tarball\n"),
    )
    assert nix.real().build_out(".#init-tarball-armv7") == "/nix/store/init-tarball"
    assert seen == [["nix", "build", "--no-link", "--print-out-paths", ".#init-tarball-armv7"]]


def test_build_out_rejects_multiple_paths(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(nix.subprocess, "run", lambda *a, **_: _cp(list(a[0]), "/one\n/two\n"))
    with pytest.raises(RuntimeError, match="printed 2 paths"):
        nix.real().build_out(".#init-tarball-armv7")


def test_copy_skips_under_dry_run(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[object] = []
    monkeypatch.setattr(nix.subprocess, "run", lambda *a, **k: calls.append(a))
    token = dry_run.set(True)
    try:
        nix.real().copy(["/nix/store/x"], "ssh://h")
    finally:
        dry_run.reset(token)
    assert not calls


def test_copy_runs_when_not_dry(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[list[str]] = []
    monkeypatch.setattr(nix.subprocess, "run", lambda argv, **k: seen.append(argv) or _cp(argv))
    nix.real().copy(["/nix/store/x"], "ssh://h")
    assert seen == [["nix", "copy", "--to", "ssh://h", "/nix/store/x"]]
