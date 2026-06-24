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
from typing import Protocol

from bmc_tui import console
from bmc_tui.stage import dry_run

# Discovered straight from the nix-owned `category` metadata, so new widgets are
# picked up without touching this code (mirrors nix-deploy.sh's explicit list).
_DECK_PACKAGES = ".#deck-packages"
_WIDGET_FILTER = (
    'ps: builtins.filter (n: (ps.${n}.category or "") == "widget") (builtins.attrNames ps)'
)


@dataclass(frozen=True)
class Pkg:
    """A resolved package: how to build it and how to register it afterwards."""

    name: str
    version: str
    installable: str  # e.g. ".#deck-packages.core.pkg^out"


@dataclass(frozen=True)
class Built(Pkg):
    """A `Pkg` after its closure has been realised into the store."""

    store_path: str


class Nix(Protocol):
    """Nix backend the catalog runs through — injected so tests need no nix."""

    def discover_widgets(self) -> list[str]:
        """Leaf names of every `category == "widget"` deck package."""
        ...

    def list_packages(self) -> list[str]:
        """Leaf names of every deck package."""
        ...

    def resolve(self, attr: str) -> Pkg:
        """Detect index-vs-raw package and read its name, version, installable."""
        ...

    def build(self, pkgs: list[Pkg]) -> list[Built]:
        """Realise every package's closure in one nix invocation; native nix
        progress goes to stderr. Returns the builts in the input order."""
        ...

    def build_out(self, attr: str) -> str:
        """Realise a single derivation and return its lone out-path — for
        artifacts (e.g. the init tarball) that carry no package metadata."""
        ...

    def copy(self, store_paths: list[str], dest: str) -> None:
        """Copy closures to `dest`. Under --dry-run, log and skip."""
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

    def resolve(self, attr: str) -> Pkg:
        version = _eval_raw(f"{attr}.version")
        if _eval_ok(f"{attr}.pkg.name"):  # index package — build its .pkg output
            return Pkg(name=attr.rsplit(".", 1)[-1], version=version, installable=f"{attr}.pkg^out")
        # Raw nixpkgs derivation — build directly, prefix the name like nix-deploy.sh.
        return Pkg(
            name=f"nixpkgs-{_eval_raw(f'{attr}.pname')}",
            version=version,
            installable=f"{attr}^out",
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
            Built(pkg.name, pkg.version, pkg.installable, store_path=path)
            for pkg, path in zip(pkgs, paths, strict=True)
        ]

    def build_out(self, attr: str) -> str:
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
        return paths[0]

    def copy(self, store_paths: list[str], dest: str) -> None:
        if dry_run.get():
            console.kv("would copy", f"{len(store_paths)} closure(s) -> {dest}")
            return
        # quiet ssh's post-quantum-KEX warning
        env = dict(os.environ)
        env["NIX_SSHOPTS"] = f"{env.get('NIX_SSHOPTS', '')} -o LogLevel=ERROR".strip()
        subprocess.run(["nix", "copy", "--to", dest, *store_paths], check=True, env=env)


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
