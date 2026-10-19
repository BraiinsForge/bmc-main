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

"""Unit tests for the local nix seam."""

import subprocess
from collections.abc import Callable
from pathlib import Path

import pytest

from bmc_tui import nix
from bmc_tui.nix import Attr, Built, Pkg, StorePath
from bmc_tui.stage import dry_run
from tests import stubs

type _Run = Callable[..., "subprocess.CompletedProcess[str]"]


def _cp(argv: list[str], stdout: str = "") -> "subprocess.CompletedProcess[str]":
    return subprocess.CompletedProcess(argv, 0, stdout=stdout, stderr="")


def _failed(argv: list[str], stderr: str) -> "subprocess.CompletedProcess[str]":
    return subprocess.CompletedProcess(argv, 1, stdout="", stderr=stderr)


def _answering(stdout: str) -> _Run:
    def run(argv: list[str], **_: object) -> "subprocess.CompletedProcess[str]":
        return _cp(argv, stdout)

    return run


def _failing(stderr: str) -> _Run:
    def run(argv: list[str], **_: object) -> "subprocess.CompletedProcess[str]":
        return _failed(argv, stderr)

    return run


def _recording(seen: list[list[str]], stdout: str = "") -> _Run:
    def run(argv: list[str], **_: object) -> "subprocess.CompletedProcess[str]":
        seen.append(argv)
        return _cp(argv, stdout)

    return run


def test_eval_present_reads_a_missing_attribute_as_absent(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    stderr = f"error: flake 'git+file:///repo' {nix.ABSENT_ATTR} 'x'"
    monkeypatch.setattr(subprocess, "run", _failing(stderr))
    assert not nix._eval_present("x")


def test_eval_present_raises_when_evaluation_itself_broke(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """An unpushed LFS object fails `.pkg.name` while `.version` still evaluates.

    Swallowing that as absent had `resolve` fall through to the raw-nixpkgs
    branch and blame a nonexistent package.
    """
    stderr = "error: bad json from /info/lfs/objects/batch: 404"
    monkeypatch.setattr(subprocess, "run", _failing(stderr))
    with pytest.raises(subprocess.CalledProcessError) as caught:
        nix._eval_present("x")
    assert caught.value.stderr == stderr, "the catalog classifies on this text"


def test_dirty_tree_reads_the_flake_metadata(monkeypatch: pytest.MonkeyPatch) -> None:
    """`dirtyRevision` is present only for a dirty tree, whatever `warn-dirty` says."""
    meta = '{"revision":"abc","dirtyRevision":"abc-dirty"}'
    monkeypatch.setattr(subprocess, "run", _answering(meta))
    assert nix.real().dirty_tree()


def test_a_clean_tree_has_no_dirty_revision(monkeypatch: pytest.MonkeyPatch) -> None:
    clean = '{"revision":"abc"}'
    monkeypatch.setattr(subprocess, "run", _answering(clean))
    assert not nix.real().dirty_tree()


def test_dirty_tree_says_nothing_when_the_query_fails(monkeypatch: pytest.MonkeyPatch) -> None:
    """A cosmetic notice must not be able to fail a deploy."""
    monkeypatch.setattr(subprocess, "run", _failing("error: boom"))
    assert not nix.real().dirty_tree()


def test_build_switches_off_the_dirty_warning(monkeypatch: pytest.MonkeyPatch) -> None:
    """`build` inherits stderr for nix's progress bar, so the warning would land
    in the middle of the TUI's own output."""
    seen: list[list[str]] = []

    def run(argv: list[str], **_: object) -> "subprocess.CompletedProcess[str]":
        seen.append(argv)
        return _cp(argv, "/nix/store/one\n")

    monkeypatch.setattr(subprocess, "run", run)
    nix.real().build([Pkg("one", "1.0", Attr(".#deck-packages.one.pkg^out"))])
    nix.real().build_out(Attr(".#init-tarball"))
    assert all("--no-warn-dirty" in argv for argv in seen)


def test_resolve_index_package(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(nix, "_eval_present", stubs.returning(True))
    monkeypatch.setattr(nix, "_eval_raw", stubs.returning("2.0"))
    assert nix.real().resolve(Attr(".#deck-packages.core")) == Pkg(
        "core", "2.0", Attr(".#deck-packages.core.pkg^out")
    )


def test_resolve_raw_nixpkgs(monkeypatch: pytest.MonkeyPatch) -> None:
    def eval_raw(expr: str) -> str:
        return "9.0" if expr.endswith(".version") else "file"

    monkeypatch.setattr(nix, "_eval_present", stubs.returning(False))
    monkeypatch.setattr(nix, "_eval_raw", eval_raw)
    assert nix.real().resolve(Attr(".#armv7-nixpkgs.file")) == Pkg(
        "nixpkgs-file", "9.0", Attr(".#armv7-nixpkgs.file^out")
    )


def test_build_maps_paths_to_packages_in_order(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[list[str]] = []

    def run(argv: list[str], **_: object) -> "subprocess.CompletedProcess[str]":
        seen.append(argv)
        return _cp(argv, "/nix/store/core\n/nix/store/clock\n")

    monkeypatch.setattr(subprocess, "run", run)
    built = nix.real().build(
        [Pkg("core", "1.0", Attr(".#core^out")), Pkg("clock", "2.0", Attr(".#clock^out"))]
    )
    assert built == [
        Built("core", "1.0", Attr(".#core^out"), StorePath("/nix/store/core")),
        Built("clock", "2.0", Attr(".#clock^out"), StorePath("/nix/store/clock")),
    ]
    # One invocation for the whole set, both installables passed through.
    assert seen == [
        [
            "nix",
            "build",
            "--no-link",
            "--print-out-paths",
            "--no-warn-dirty",
            ".#core^out",
            ".#clock^out",
        ]
    ]


def test_build_passes_max_jobs_when_set(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[list[str]] = []
    monkeypatch.setattr(subprocess, "run", _recording(seen, "/nix/store/abc\n"))
    nix.real(max_jobs=4).build([Pkg("core", "1.0", Attr(".#x^out"))])
    assert "--max-jobs" in seen[0] and seen[0][seen[0].index("--max-jobs") + 1] == "4"


def test_build_rejects_path_count_mismatch(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(subprocess, "run", _answering("/nix/store/only\n"))
    with pytest.raises(RuntimeError, match="printed 1 paths for 2"):
        nix.real().build([Pkg("a", "1", Attr(".#a^out")), Pkg("b", "1", Attr(".#b^out"))])


def test_build_out_returns_the_lone_path(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[list[str]] = []
    monkeypatch.setattr(subprocess, "run", _recording(seen, "/nix/store/init-tarball\n"))
    assert nix.real().build_out(Attr(".#init-tarball-armv7")) == "/nix/store/init-tarball"
    assert seen == [
        [
            "nix",
            "build",
            "--no-link",
            "--print-out-paths",
            "--no-warn-dirty",
            ".#init-tarball-armv7",
        ]
    ]


def test_build_out_rejects_multiple_paths(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(subprocess, "run", _answering("/one\n/two\n"))
    with pytest.raises(RuntimeError, match="printed 2 paths"):
        nix.real().build_out(Attr(".#init-tarball-armv7"))


def test_build_file_builds_all_attrs_in_one_invocation(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[list[str]] = []
    monkeypatch.setattr(subprocess, "run", _recording(seen, "/nix/store/ia\n/nix/store/ta\n"))
    outs = nix.real().build_file(
        "nix/e2e-artifacts.nix",
        [Attr("index-a"), Attr("tarball-a")],
        {"bosVersionA": "va", "bosVersionB": "vb"},
    )
    assert outs == ["/nix/store/ia", "/nix/store/ta"]
    assert seen == [
        [
            "nix",
            "build",
            "--impure",
            "-f",
            "nix/e2e-artifacts.nix",
            "index-a",
            "tarball-a",
            "--no-link",
            "--print-out-paths",
            "--argstr",
            "bosVersionA",
            "va",
            "--argstr",
            "bosVersionB",
            "vb",
        ]
    ]


def test_build_file_rejects_a_path_count_mismatch(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(subprocess, "run", _answering("/one\n"))
    with pytest.raises(RuntimeError, match="printed 1 paths for 2 attrs"):
        nix.real().build_file("nix/e2e-artifacts.nix", [Attr("index-a"), Attr("tarball-a")], {})


def test_copy_skips_under_dry_run(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[list[str]] = []
    monkeypatch.setattr(subprocess, "run", _recording(calls))
    token = dry_run.set(True)
    try:
        nix.real().copy([StorePath("/nix/store/x")], "ssh://h")
    finally:
        dry_run.reset(token)
    assert not calls


def test_copy_runs_when_not_dry(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[list[str]] = []
    monkeypatch.setattr(subprocess, "run", _recording(seen))
    nix.real().copy([StorePath("/nix/store/x")], "ssh://h")
    assert seen == [["nix", "copy", "--to", "ssh://h", "/nix/store/x"]]


def test_generate_cache_key_writes_secret_and_returns_public(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    def run(argv: list[str], **kwargs: object) -> "subprocess.CompletedProcess[str]":
        if "generate-secret" in argv:
            assert argv == ["nix", "key", "generate-secret", "--key-name", "cache-1"]
            return _cp(argv, "SECRETKEY\n")
        assert argv == ["nix", "key", "convert-secret-to-public"]
        assert kwargs["input"] == "SECRETKEY\n"
        return _cp(argv, "cache-1:PUB\n")

    monkeypatch.setattr(subprocess, "run", run)
    secret = tmp_path / "key.secret"
    assert nix.real().generate_cache_key("cache-1", secret) == "cache-1:PUB"
    assert secret.read_text() == "SECRETKEY\n"


def test_copy_signed_signs_into_the_local_cache(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    seen: list[list[str]] = []
    monkeypatch.setattr(subprocess, "run", _recording(seen))
    nix.real().copy_signed([StorePath("/nix/store/x")], tmp_path / "cache", tmp_path / "key.secret")
    dest = f"file://{tmp_path}/cache?secret-key={tmp_path}/key.secret"
    assert seen == [["nix", "copy", "--to", dest, "/nix/store/x"]]
