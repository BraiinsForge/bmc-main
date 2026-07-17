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

"""Local package rig for the sysupgrade e2e: feed, indexes, tarballs, cache.

Assembles a throwaway HTTP tree that mimics the production package
infrastructure — the package feed at the serve root (whose URL doubles as
the factory base URL), one index and tarball per firmware version, and a
signed file:// binary cache holding both variants' closures — and serves
it from a stdlib ThreadingHTTPServer on a daemon thread.
"""

import json
import subprocess
from collections.abc import Callable
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Self

from bmc_tui.nix import Nix, StorePath
from bmc_tui.server import ServerHandle, server

FEED_NAME = "nix-package-feed.v1.json"
INDEX_NAME = "nix-package-index.v1.json"


@dataclass(frozen=True)
class Variant:
    """One firmware version's rig artifacts, as built by nix/e2e-artifacts.nix."""

    bos_version: str
    profile_path: str  # from the tarball's metadata.json
    index: Path  # out-path directory holding nix-package-index.v1.json
    tarball: Path  # the archive itself, inside its out-path directory
    signature: str | None = None  # nix-style Ed25519 line for the feed entry


def feed_document(variants: list[Variant], base_url: str) -> str:
    """The feed body: one entry per variant, URLs matching write_serve_root's
    layout exactly — init follows download_url, upgrade follows index_url.
    Signed variants carry the init-tarball signature the device verifies."""
    entries = []
    for v in variants:
        entry = {
            "bos_version": v.bos_version,
            "download_url": f"{base_url}/tarballs/{v.tarball.name}",
            "profile_path": v.profile_path,
            "index_url": f"{base_url}/index/{v.bos_version}/{INDEX_NAME}",
        }
        if v.signature is not None:
            entry["signature"] = v.signature
        entries.append(entry)
    return json.dumps({"version": 1, "entries": entries}, indent=2)


def index_store_paths(index: Path) -> list[StorePath]:
    """Every package store path an index lists — the closure roots the rig
    cache must hold."""
    doc = json.loads((index / INDEX_NAME).read_text())
    return [StorePath(p["store_path"]) for p in doc["packages"]]


def package_store_path(index: Path, name: str) -> str | None:
    """The store path an index lists for package `name`; None when absent."""
    doc = json.loads((index / INDEX_NAME).read_text())
    return next((p["store_path"] for p in doc["packages"] if p["name"] == name), None)


def write_serve_root(root: Path, variants: list[Variant], base_url: str) -> None:
    """Lay out the HTTP tree: the feed at the root, one index + tarball per
    variant. Store artifacts are symlinked, not copied — the handler follows
    symlinks and the tarballs are large."""
    root.mkdir(parents=True, exist_ok=True)
    (root / FEED_NAME).write_text(feed_document(variants, base_url))
    (root / "tarballs").mkdir(exist_ok=True)
    for v in variants:
        index_dir = root / "index" / v.bos_version
        index_dir.mkdir(parents=True, exist_ok=True)
        _relink(index_dir / INDEX_NAME, v.index / INDEX_NAME)
        _relink(root / "tarballs" / v.tarball.name, v.tarball)


def _relink(link: Path, target: Path) -> None:
    """Idempotent symlink — the layout is re-runnable against the same root,
    matching the mkdir(exist_ok=True) calls around it."""
    link.unlink(missing_ok=True)
    link.symlink_to(target)


CACHE_KEY_NAME = "sysupgrade-e2e-1"


def generate_cache_key(nix: Nix, secret: Path) -> str:
    """Write the suite's signing secret to `secret`; returns the public key.
    One key serves both roles: nix-cache NAR signing and the init-tarball
    trust anchor (production precedent: the downloads key does the same)."""
    return nix.generate_cache_key(CACHE_KEY_NAME, secret)


def sign_variant(
    host_cli: str,
    secret: Path,
    variant: Variant,
    *,
    run: Callable[..., "subprocess.CompletedProcess[str]"] = subprocess.run,
) -> Variant:
    """A copy of `variant` carrying its tarball's Ed25519 signature, produced
    by the host-built bmc-nix-cli so the fingerprint format never leaves
    Rust."""
    proc = run(
        [
            f"{host_cli}/bin/bmc-nix-cli",
            "sign-init-tarball",
            "--secret-key",
            str(secret),
            str(variant.tarball),
        ],
        stdout=subprocess.PIPE,
        text=True,
        check=True,
    )
    return replace(variant, signature=proc.stdout.strip())


def populate_cache(nix: Nix, secret: Path, cache: Path, variants: list[Variant]) -> None:
    """Copy both variants' closures into the signed file:// cache."""
    paths = sorted({p for v in variants for p in index_store_paths(v.index)})
    nix.copy_signed(paths, cache, secret)


class RigServer:
    """Mount `root` at the serve root; use as a context manager.

    Binding is deferred to `__enter__` because the caller populates `root`
    after construction — the mount resolves per request, so files appearing
    later are still served.
    """

    def __init__(self, root: Path, *, port: int, bind_ip: str = "0.0.0.0") -> None:
        self._root = root
        self._port = port
        self._bind_ip = bind_ip
        self._handle: ServerHandle | None = None

    @property
    def port(self) -> int:
        """The bound port — the requested one, or the ephemeral pick for 0."""
        if self._handle is None:
            raise RuntimeError("BUG: rig server port read before it was started")
        return self._handle.port

    def __enter__(self) -> Self:
        self._handle = server({"/": self._root}, bind_host=self._bind_ip, port=self._port)
        return self

    def __exit__(self, *_exc: object) -> None:
        if self._handle is not None:
            self._handle.stop()
