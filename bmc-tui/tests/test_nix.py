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

import json
import subprocess
import time
from collections.abc import Callable, Generator, Mapping
from contextlib import contextmanager
from pathlib import Path
from typing import cast, final

import pytest

from bmc_tui import console, nix
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
        _ = nix._eval_present("x")
    assert cast("str", caught.value.stderr) == stderr, "the catalog classifies on this text"


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
    _ = nix.real().build([Pkg("one", "1.0", Attr(".#deck-packages.one.pkg^out"))])
    _ = nix.real().build_out(Attr(".#init-tarball"))
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
    _ = nix.real(max_jobs=4).build([Pkg("core", "1.0", Attr(".#x^out"))])
    assert "--max-jobs" in seen[0] and seen[0][seen[0].index("--max-jobs") + 1] == "4"


def test_build_rejects_path_count_mismatch(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(subprocess, "run", _answering("/nix/store/only\n"))
    with pytest.raises(RuntimeError, match="printed 1 paths for 2"):
        _ = nix.real().build([Pkg("a", "1", Attr(".#a^out")), Pkg("b", "1", Attr(".#b^out"))])


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
        _ = nix.real().build_out(Attr(".#init-tarball-armv7"))


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
        _ = nix.real().build_file("nix/e2e-artifacts.nix", [Attr("index-a"), Attr("tarball-a")], {})


def test_copy_skips_under_dry_run(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[list[str]] = []
    monkeypatch.setattr(subprocess, "run", _recording(calls))
    token = dry_run.set(True)
    try:
        nix.real().copy([StorePath("/nix/store/x")], "ssh://root@h", [])
    finally:
        dry_run.reset(token)
    assert not calls


@final
class _CopyRun:
    """Stands in for `stream_lines`: records the call and feeds canned output lines."""

    def __init__(self, lines: list[str], code: int = 0) -> None:
        self.argv: list[str] = []
        self.env: Mapping[str, str] = {}
        self._lines = lines
        self._code = code

    def __call__(
        self,
        argv: list[str],
        on_line: Callable[[str], None],
        *,
        env: Mapping[str, str] | None = None,
    ) -> int:
        self.argv, self.env = argv, env or {}
        for line in self._lines:
            on_line(line)
        return self._code


def _copy(
    monkeypatch: pytest.MonkeyPatch, run: _CopyRun, ssh_options: tuple[str, ...] = ()
) -> None:
    monkeypatch.setattr(nix, "stream_lines", run)
    nix.real().copy([StorePath("/nix/store/x")], "ssh://root@h", list(ssh_options))


def test_copy_hands_nix_the_device_s_ssh_options_ahead_of_the_user_s_own(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("NIX_SSHOPTS", "-o ControlPath=/tmp/theirs")
    run = _CopyRun([])

    _copy(monkeypatch, run, ("-o", "ControlPath=/tmp/deck-ssh-x/0"))

    assert run.env["NIX_SSHOPTS"] == (
        "-o ControlPath=/tmp/deck-ssh-x/0 -o ControlPath=/tmp/theirs -o LogLevel=ERROR"
    ), "ssh takes the first ControlPath, and nix copy must join the device's connection"


def test_copy_without_ssh_options_of_its_own_only_quiets_ssh(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("NIX_SSHOPTS", raising=False)
    run = _CopyRun([])

    _copy(monkeypatch, run)

    assert run.env["NIX_SSHOPTS"] == "-o LogLevel=ERROR"


def test_copy_asks_nix_for_its_progress_events(monkeypatch: pytest.MonkeyPatch) -> None:
    run = _CopyRun([])
    _copy(monkeypatch, run)
    assert run.argv == [
        "nix",
        "copy",
        "--log-format",
        "internal-json",
        "--to",
        "ssh://root@h",
        "/nix/store/x",
    ]


def test_the_bar_waits_for_the_first_path_so_a_password_prompt_stays_readable(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    shown: list[str] = []

    @contextmanager
    def transfer() -> Generator[Callable[[str, int, int | None], None]]:
        yield lambda what, _done, _total: shown.append(what)

    monkeypatch.setattr(console, "transfer", transfer)
    run = _CopyRun(
        [
            _event(action="msg", level=1, msg="warning: no nixbld"),
            _event(action="start", id=7, type=109, fields=["/nix/store/aaa-x", "ssh://h"]),
            _start_copy(1, f"/nix/store/{_HASH}-bmc-core"),
        ]
    )

    _copy(monkeypatch, run)

    assert set(shown) == {"bmc-core"}


def test_a_flood_of_events_redraws_the_bar_sparingly(monkeypatch: pytest.MonkeyPatch) -> None:
    shown: list[int] = []

    @contextmanager
    def transfer() -> Generator[Callable[[str, int, int | None], None]]:
        yield lambda _what, done, _total: shown.append(done)

    monkeypatch.setattr(console, "transfer", transfer)
    monkeypatch.setattr(time, "monotonic", lambda: 0.0)
    progress = [
        _event(action="result", id=1, type=105, fields=[n, 1000, 0, 0]) for n in range(1, 1001)
    ]
    run = _CopyRun([_start_copy(1, "/nix/store/aaa-glibc-2.44"), *progress])

    _copy(monkeypatch, run)

    assert shown == [0, 1000], "one redraw while no time passes, then the final state"


def test_a_failed_copy_shows_what_nix_said_once(monkeypatch: pytest.MonkeyPatch) -> None:
    noted: list[str] = []
    monkeypatch.setattr(console, "note", noted.append)
    run = _CopyRun([_event(action="msg", level=0, msg="error: cannot connect to 'h'")], code=1)

    with pytest.raises(subprocess.CalledProcessError) as failed:
        _copy(monkeypatch, run)

    assert noted == ["error: cannot connect to 'h'"]
    stderr = cast("str | None", failed.value.stderr)
    assert stderr is None, "it is on screen already, the failure must not repeat it"


# ── copy progress ─────────────────────────────────────────────────────────────


def _event(**event: object) -> str:
    return "@nix " + json.dumps(event)


# As long as a real store path's hash, in Nix's base-32 alphabet
_HASH = "3jf2rxan20awdggs6z3x95gwjfpi0zz8"


def _start_copy(ident: int, path: str) -> str:
    # The shape `nix copy` sent, recorded for a copy into a local file store
    return _event(action="start", id=ident, type=100, fields=[path, "daemon", "ssh://h"])


def test_copy_progress_sums_the_bytes_of_every_path() -> None:
    progress = nix.CopyProgress()
    for line in [
        _start_copy(1, "/nix/store/aaa-glibc-2.44"),
        _event(action="result", id=9, type=106, fields=[100, 300]),
        _event(action="result", id=1, type=105, fields=[50, 100, 0, 0]),
        _event(action="result", id=1, type=105, fields=[90, 100, 0, 0]),
        _start_copy(2, f"/nix/store/{_HASH}-bmc-core"),
        _event(action="result", id=2, type=105, fields=[20, 200, 0, 0]),
    ]:
        assert progress.feed(line) is None

    assert (progress.done, progress.total) == (110, 300), "each event says how far its path is"
    assert progress.current == "bmc-core"


def test_a_path_is_named_without_its_store_hash() -> None:
    progress = nix.CopyProgress()

    _ = progress.feed(_start_copy(1, f"/nix/store/{_HASH}-glibc-2.44"))

    assert progress.current == "glibc-2.44"


def test_a_name_outside_the_store_layout_is_shown_whole() -> None:
    progress = nix.CopyProgress()

    _ = progress.feed(_start_copy(1, "/nix/store/bbb-bmc-core"))

    assert progress.current == "bbb-bmc-core", "a short prefix is no hash to cut"


def test_a_finished_path_counts_in_full() -> None:
    progress = nix.CopyProgress()
    for line in [
        _start_copy(1, "/nix/store/aaa-glibc-2.44"),
        _event(action="result", id=1, type=105, fields=[99, 100, 0, 0]),
        _event(action="stop", id=1),
    ]:
        _ = progress.feed(line)

    assert progress.done == 100


def test_an_event_of_an_unexpected_shape_is_skipped() -> None:
    progress = nix.CopyProgress()
    for line in [
        _start_copy(1, "/nix/store/aaa-glibc-2.44"),
        _event(action="result", id=9, type=106, fields=[100]),
        _event(action="result", id=1, type=105, fields=["50", 100]),
        _event(action="result", id=1, type=105, fields=[50, 100, 0, 0]),
    ]:
        assert progress.feed(line) is None

    assert (progress.done, progress.total) == (50, None)


def test_copy_progress_hands_back_only_text_meant_for_the_user() -> None:
    progress = nix.CopyProgress()

    assert progress.feed(_event(action="msg", level=1, msg="warning: no nixbld")) == (
        "warning: no nixbld"
    )
    assert progress.feed(_event(action="msg", level=3, msg="copying 2 paths")) is None
    assert progress.feed("plain stderr") == "plain stderr"


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
