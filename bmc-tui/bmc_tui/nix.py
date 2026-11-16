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

"""Local nix operations on the dev host, with a dry-run-aware seam.

`nix build`/`eval` run on the build host; `nix copy` runs here too but drives
the device's own `nix-store` remotely (the device has the store, not full nix).
Building runs even under `--dry-run` — it is the closure's verification — while
`copy` is a device mutation and is skipped, mirroring the `Device` seam.

All work goes through an injected `Nix` backend so tests need no real nix.
"""

import os
import re
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import NewType, Protocol, TypedDict, final

import msgspec

from bmc_tui import console, json_value
from bmc_tui.json_value import Json
from bmc_tui.process import stream_lines
from bmc_tui.stage import dry_run

# Discovered straight from the nix-owned `category` metadata, so new widgets are
# picked up without touching this code (mirrors nix-deploy.sh's explicit list).
_DECK_PACKAGES = ".#deck-packages"
_WIDGET_FILTER = (
    'ps: builtins.filter (n: (ps.${n}.category or "") == "widget") (builtins.attrNames ps)'
)

# nix's wording when a flake genuinely lacks an attribute, as opposed
# to holding one it could not evaluate. The two need different diagnoses.
ABSENT_ATTR = "does not provide attribute"

# nix repeats a dirty-worktree warning on every flake invocation, and the build calls
# inherit stderr so nix can draw its progress bar. They switch it off and the TUI
# states the fact once itself — read from the flake metadata, not from the warning,
# so `warn-dirty = false` in nix.conf cannot silence our notice too.
_NO_WARN_DIRTY = "--no-warn-dirty"


# An absolute `/nix/store/…` path. Two equal ones mean the same build,
# so they stay distinct from the device paths they are compared with.
StorePath = NewType("StorePath", str)

# A flake attribute or installable, e.g. `.#deck-packages.core` or `…^out`.
Attr = NewType("Attr", str)

# A store path's base name: 32 characters of Nix's base-32 alphabet (no e, o, t or u),
# then the name its derivation gave it
_STORE_NAME = re.compile(r"[0-9a-df-np-sv-z]{32}-(.+)")

# Nix's `--log-format internal-json` events, and the codes they carry (Nix's `logging.hh`).
_NIX_EVENT = "@nix "
_ACT_COPY_PATH = 100
_RES_PROGRESS = 105
_RES_SET_EXPECTED = 106
_LVL_WARN = 1  # this level and below, errors, are meant for the user

# A copy sends an event per NAR write;
# the bar is redrawn at most this often
_REDRAW_S = 0.1


class _NixEvent(TypedDict, total=False):
    action: str
    id: int
    type: int
    fields: list[int | str] | None
    level: int
    msg: str


def _shown_name(path: str) -> str:
    """The store path's name without its hash, or its whole base name if it is no store path."""
    base = Path(path).name
    match = _STORE_NAME.fullmatch(base)
    return match.group(1) if match else base


@final
class CopyProgress:
    """What a `nix copy --log-format internal-json` has copied so far, in bytes of its total."""

    def __init__(self) -> None:
        self.total: int | None = None
        self.done: int = 0
        self.current: str = ""
        # Bytes done and size of each path still being copied
        self._paths: dict[int, tuple[int, int]] = {}

    def feed(self, line: str) -> str | None:
        """Take a line of the copy's output; return the text in it meant for the user, if any:
        Nix's warnings and errors, or a line that is no event at all."""
        if not line.startswith(_NIX_EVENT):
            return line
        try:
            event = msgspec.json.decode(line.removeprefix(_NIX_EVENT), type=_NixEvent)
        except msgspec.DecodeError:
            return line
        action, kind, ident = event.get("action"), event.get("type"), event.get("id")
        fields = event.get("fields") or []
        if action == "msg":
            return event.get("msg", "") if event.get("level", _LVL_WARN + 1) <= _LVL_WARN else None
        if ident is None:
            return None
        match fields:
            case [str(path), *_] if action == "start" and kind == _ACT_COPY_PATH:
                self._paths[ident] = (0, 0)
                self.current = _shown_name(path)
            case [int(activity), int(total), *_] if (
                action == "result" and kind == _RES_SET_EXPECTED and activity == _ACT_COPY_PATH
            ):
                self.total = total
            case [int(done), int(size), *_] if (
                action == "result" and kind == _RES_PROGRESS and ident in self._paths
            ):
                self.done += done - self._paths[ident][0]
                self._paths[ident] = (done, size)
            case _ if action == "stop" and ident in self._paths:
                done, size = self._paths.pop(ident)
                self.done += size - done
            case _:
                pass
        return None


@dataclass(frozen=True)
class Pkg:
    """A resolved package: how to build it and how to register it afterwards."""

    name: str
    version: str
    installable: Attr  # e.g. ".#deck-packages.core.pkg^out"


@dataclass(frozen=True)
class Built(Pkg):
    """A `Pkg` after its closure has been realised into the store."""

    store_path: StorePath


class Nix(Protocol):
    """Nix backend the catalog runs through — injected so tests need no nix."""

    def discover_widgets(self) -> list[str]:
        """Leaf names of every `category == "widget"` deck package."""
        ...

    def list_packages(self, prefix: str = _DECK_PACKAGES, /) -> list[str]:
        """Leaf names of every deck package in `prefix`'s set.

        Each profile holds its own set, so a suggestion drawn from the release
        one while deploying a debug one can name the very package it rejected.
        """
        ...

    def resolve(self, attr: Attr, /) -> Pkg:
        """Detect index-vs-raw package and read its name, version, installable."""
        ...

    def dirty_tree(self) -> bool:
        """Whether the flake is being built from an uncommitted worktree."""
        ...

    def build(self, pkgs: list[Pkg], /) -> list[Built]:
        """Realise every package's closure in one nix invocation; native nix
        progress goes to stderr. Returns the builts in the input order."""
        ...

    def build_out(self, attr: Attr, /) -> StorePath:
        """Realise a single derivation and return its lone out-path — for
        artifacts (e.g. the init tarball) that carry no package metadata."""
        ...

    def out_path(self, attr: Attr, /) -> StorePath:
        """The store path an index package *would* have, without building it.

        Input-addressed, so it answers whether a device is running this
        tree's build without realising the derivation.
        """
        ...

    def build_file(self, file: str, attrs: list[Attr], args: dict[str, str], /) -> list[StorePath]:
        """Realise attrs of a parameterized nix file (`--impure -f`) in ONE
        invocation — one consistent evaluation of the mutable worktree —
        returning their out-paths in attr order. Flake outputs cannot take
        parameters."""
        ...

    def copy(self, store_paths: list[StorePath], dest: str, /) -> None:
        """Copy closures to `dest`. Under --dry-run, log and skip."""
        ...

    def generate_cache_key(self, name: str, secret: Path, /) -> str:
        """Write a fresh NAR-signing secret to `secret` and return its
        public key."""
        ...

    def copy_signed(self, store_paths: list[StorePath], cache: Path, secret: Path, /) -> None:
        """Copy closures into a local file:// binary cache, signing every
        path with `secret`."""
        ...


@final
class _RealNix:
    def __init__(self, *, max_jobs: int | None = None) -> None:
        self._max_jobs = max_jobs

    def discover_widgets(self) -> list[str]:
        out = _eval_json(_DECK_PACKAGES, _WIDGET_FILTER)
        if not isinstance(out, list):
            return []
        return sorted(str(name) for name in out)

    def list_packages(self, prefix: str = _DECK_PACKAGES) -> list[str]:
        out = _eval_json(prefix, "builtins.attrNames")
        if not isinstance(out, list):
            return []
        return sorted(str(name) for name in out)

    def resolve(self, attr: Attr) -> Pkg:
        version = _eval_raw(f"{attr}.version")
        if _eval_present(f"{attr}.pkg.name"):  # index package — build its .pkg output
            return Pkg(
                name=attr.rsplit(".", 1)[-1], version=version, installable=Attr(f"{attr}.pkg^out")
            )
        # Raw nixpkgs derivation — build directly, prefix the name like nix-deploy.sh.
        return Pkg(
            name=f"nixpkgs-{_eval_raw(f'{attr}.pname')}",
            version=version,
            installable=Attr(f"{attr}^out"),
        )

    def dirty_tree(self) -> bool:
        # `dirtyRevision` is present only for a dirty tree. A cosmetic notice
        # must not be able to fail a deploy, so a query that fails says nothing.
        proc = subprocess.run(
            ["nix", "flake", "metadata", "--json", _NO_WARN_DIRTY, "."],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            return False
        meta = json_value.loads(proc.stdout)
        return isinstance(meta, dict) and "dirtyRevision" in meta

    def build(self, pkgs: list[Pkg]) -> list[Built]:
        # One `nix build` for the whole set so nix schedules the derivations
        # concurrently up to its max-jobs, instead of us serialising them.
        # Inherit stderr so nix draws its own progress bar; capture stdout,
        # which prints one out-path per installable in argument order.
        max_jobs = ["--max-jobs", str(self._max_jobs)] if self._max_jobs is not None else []
        proc = subprocess.run(
            [
                "nix",
                "build",
                "--no-link",
                "--print-out-paths",
                _NO_WARN_DIRTY,
                *max_jobs,
                *(p.installable for p in pkgs),
            ],
            stdout=subprocess.PIPE,
            text=True,
            check=True,
        )
        paths = proc.stdout.split()
        if len(paths) != len(pkgs):
            msg = f"BUG: nix build printed {len(paths)} paths for {len(pkgs)} package(s)"
            raise RuntimeError(msg)
        return [
            Built(pkg.name, pkg.version, pkg.installable, store_path=StorePath(path))
            for pkg, path in zip(pkgs, paths, strict=True)
        ]

    def out_path(self, attr: Attr) -> StorePath:
        return StorePath(_eval_raw(f"{attr}.pkg.outPath"))

    def build_out(self, attr: Attr) -> StorePath:
        # Single `nix build` of one attr; capture stdout for the out-path, let
        # nix draw its own progress on the inherited stderr.
        proc = subprocess.run(
            ["nix", "build", "--no-link", "--print-out-paths", _NO_WARN_DIRTY, attr],
            stdout=subprocess.PIPE,
            text=True,
            check=True,
        )
        paths = proc.stdout.split()
        if len(paths) != 1:
            msg = f"BUG: nix build printed {len(paths)} paths for {attr}"
            raise RuntimeError(msg)
        return StorePath(paths[0])

    def build_file(self, file: str, attrs: list[Attr], args: dict[str, str]) -> list[StorePath]:
        argstrs = [s for key, value in sorted(args.items()) for s in ("--argstr", key, value)]
        proc = subprocess.run(
            [
                "nix",
                "build",
                "--impure",
                "-f",
                file,
                *attrs,
                "--no-link",
                "--print-out-paths",
                *argstrs,
            ],
            stdout=subprocess.PIPE,
            text=True,
            check=True,
        )
        paths = proc.stdout.split()
        if len(paths) != len(attrs):
            msg = f"BUG: nix build printed {len(paths)} paths for {len(attrs)} attrs"
            raise RuntimeError(msg)
        return [StorePath(p) for p in paths]

    def copy(self, store_paths: list[StorePath], dest: str) -> None:
        if dry_run.get():
            console.kv("would copy", f"{len(store_paths)} closure(s) -> {dest}")
            return
        # quiet ssh's post-quantum-KEX warning
        env = dict(os.environ)
        env["NIX_SSHOPTS"] = f"{env.get('NIX_SSHOPTS', '')} -o LogLevel=ERROR".strip()
        argv = ["nix", "copy", "--log-format", "internal-json", "--to", dest, *store_paths]
        progress = CopyProgress()
        shown_at = -_REDRAW_S
        with console.transfer() as show:

            def on_line(line: str) -> None:
                nonlocal shown_at
                if (text := progress.feed(line)) is not None:
                    console.note(text)
                now = time.monotonic()
                # A path is copied only once ssh is through,
                # so its password prompt stays readable
                if progress.current and now - shown_at >= _REDRAW_S:
                    show(progress.current, progress.done, progress.total)
                    shown_at = now

            code = stream_lines(argv, on_line, env=env)
            if progress.current:
                show(progress.current, progress.done, progress.total)
        if code:
            # What Nix said about it is already printed above
            raise subprocess.CalledProcessError(code, argv)

    def generate_cache_key(self, name: str, secret: Path) -> str:
        proc = subprocess.run(
            ["nix", "key", "generate-secret", "--key-name", name],
            stdout=subprocess.PIPE,
            text=True,
            check=True,
        )
        _ = secret.write_text(proc.stdout)
        secret.chmod(0o600)
        public = subprocess.run(
            ["nix", "key", "convert-secret-to-public"],
            input=proc.stdout,
            stdout=subprocess.PIPE,
            text=True,
            check=True,
        )
        return public.stdout.strip()

    def copy_signed(self, store_paths: list[StorePath], cache: Path, secret: Path) -> None:
        # Host-side artifact staging, not a device mutation — runs under
        # --dry-run like `build`; the ?secret-key param signs on write.
        _ = subprocess.run(
            ["nix", "copy", "--to", f"file://{cache}?secret-key={secret}", *store_paths],
            check=True,
        )


def real(*, max_jobs: int | None = None) -> Nix:
    """The production nix backend that shells out to `nix`. `max_jobs` caps how
    many derivations build at once; left None, nix uses its own configuration."""
    return _RealNix(max_jobs=max_jobs)


def _eval_present(expr: str) -> bool:
    """Whether `expr` exists, distinguishing an absent attribute
    from an evaluation that broke for some other reason.

    An unpushed git-LFS object makes `.pkg.name` fail while `.version`
    still evaluates; reading that as "absent" made `resolve` take the package
    for a raw nixpkgs derivation, and report the wrong problem two calls later.
    """
    argv = ["nix", "eval", expr]
    proc = subprocess.run(argv, capture_output=True, text=True, check=False)

    if proc.returncode == 0:
        return True

    if ABSENT_ATTR in proc.stderr:
        return False

    raise subprocess.CalledProcessError(proc.returncode, argv, proc.stdout, proc.stderr)


def _eval_raw(expr: str) -> str:
    return subprocess.run(
        ["nix", "eval", "--raw", expr], capture_output=True, text=True, check=True
    ).stdout.strip()


def _eval_json(attr: str, apply: str) -> Json:
    out = subprocess.run(
        ["nix", "eval", "--json", attr, "--apply", apply],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return json_value.loads(out)
