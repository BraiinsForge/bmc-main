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

import json
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import NewType, Protocol

from bmc_tui import console
from bmc_tui.stage import dry_run

# Discovered straight from the nix-owned `category` metadata, so new widgets are
# picked up without touching this code (mirrors nix-deploy.sh's explicit list).
_DECK_PACKAGES = ".#deck-packages"
_WIDGET_FILTER = (
    'ps: builtins.filter (n: (ps.${n}.category or "") == "widget") (builtins.attrNames ps)'
)


# An absolute `/nix/store/…` path. Two equal ones mean the same build,
# so they stay distinct from the device paths they are compared with.
StorePath = NewType("StorePath", str)

# A flake attribute or installable, e.g. `.#deck-packages.core` or `…^out`.
Attr = NewType("Attr", str)


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

    def list_packages(self) -> list[str]:
        """Leaf names of every deck package."""
        ...

    def resolve(self, attr: Attr) -> Pkg:
        """Detect index-vs-raw package and read its name, version, installable."""
        ...

    def build(self, pkgs: list[Pkg]) -> list[Built]:
        """Realise every package's closure in one nix invocation; native nix
        progress goes to stderr. Returns the builts in the input order."""
        ...

    def build_out(self, attr: Attr) -> StorePath:
        """Realise a single derivation and return its lone out-path — for
        artifacts (e.g. the init tarball) that carry no package metadata."""
        ...

    def build_file(self, file: str, attrs: list[Attr], args: dict[str, str]) -> list[StorePath]:
        """Realise attrs of a parameterized nix file (`--impure -f`) in ONE
        invocation — one consistent evaluation of the mutable worktree —
        returning their out-paths in attr order. Flake outputs cannot take
        parameters."""
        ...

    def copy(self, store_paths: list[StorePath], dest: str) -> None:
        """Copy closures to `dest`. Under --dry-run, log and skip."""
        ...

    def generate_cache_key(self, name: str, secret: Path) -> str:
        """Write a fresh NAR-signing secret to `secret` and return its
        public key."""
        ...

    def copy_signed(self, store_paths: list[StorePath], cache: Path, secret: Path) -> None:
        """Copy closures into a local file:// binary cache, signing every
        path with `secret`."""
        ...


class _RealNix:
    def __init__(self, *, max_jobs: int | None = None) -> None:
        self._max_jobs = max_jobs

    def discover_widgets(self) -> list[str]:
        out = _eval_json(_DECK_PACKAGES, _WIDGET_FILTER)
        if not isinstance(out, list):
            return []
        return sorted(str(name) for name in out)

    def list_packages(self) -> list[str]:
        out = _eval_json(_DECK_PACKAGES, "builtins.attrNames")
        if not isinstance(out, list):
            return []
        return sorted(str(name) for name in out)

    def resolve(self, attr: Attr) -> Pkg:
        version = _eval_raw(f"{attr}.version")
        if _eval_ok(f"{attr}.pkg.name"):  # index package — build its .pkg output
            return Pkg(
                name=attr.rsplit(".", 1)[-1], version=version, installable=Attr(f"{attr}.pkg^out")
            )
        # Raw nixpkgs derivation — build directly, prefix the name like nix-deploy.sh.
        return Pkg(
            name=f"nixpkgs-{_eval_raw(f'{attr}.pname')}",
            version=version,
            installable=Attr(f"{attr}^out"),
        )

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

    def build_out(self, attr: Attr) -> StorePath:
        # Single `nix build` of one attr; capture stdout for the out-path, let
        # nix draw its own progress on the inherited stderr.
        proc = subprocess.run(
            ["nix", "build", "--no-link", "--print-out-paths", attr],
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
        subprocess.run(["nix", "copy", "--to", dest, *store_paths], check=True, env=env)

    def generate_cache_key(self, name: str, secret: Path) -> str:
        proc = subprocess.run(
            ["nix", "key", "generate-secret", "--key-name", name],
            stdout=subprocess.PIPE,
            text=True,
            check=True,
        )
        secret.write_text(proc.stdout)
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
        subprocess.run(
            ["nix", "copy", "--to", f"file://{cache}?secret-key={secret}", *store_paths],
            check=True,
        )


def real(*, max_jobs: int | None = None) -> Nix:
    """The production nix backend that shells out to `nix`. `max_jobs` caps how
    many derivations build at once; left None, nix uses its own configuration."""
    return _RealNix(max_jobs=max_jobs)


def _eval_ok(expr: str) -> bool:
    return subprocess.run(["nix", "eval", expr], capture_output=True, check=False).returncode == 0


def _eval_raw(expr: str) -> str:
    return subprocess.run(
        ["nix", "eval", "--raw", expr], capture_output=True, text=True, check=True
    ).stdout.strip()


def _eval_json(attr: str, apply: str) -> object:
    out = subprocess.run(
        ["nix", "eval", "--json", attr, "--apply", apply],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return json.loads(out)
