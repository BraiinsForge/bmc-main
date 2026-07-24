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
import socket
import threading
from dataclasses import dataclass
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from bmc_tui.nix import Nix, StorePath

FEED_NAME = "nix-package-feed.v1.json"
INDEX_NAME = "nix-package-index.v1.json"


@dataclass(frozen=True)
class Variant:
    """One firmware version's rig artifacts, as built by nix/e2e-artifacts.nix."""

    bos_version: str
    profile_path: str  # from the tarball's metadata.json
    index: Path  # out-path directory holding nix-package-index.v1.json
    tarball: Path  # the archive itself, inside its out-path directory


def feed_document(variants: list[Variant], base_url: str) -> str:
    """The feed body: one entry per variant, URLs matching write_serve_root's
    layout exactly — init follows download_url, upgrade follows index_url."""
    entries = [
        {
            "bos_version": v.bos_version,
            "download_url": f"{base_url}/tarballs/{v.tarball.name}",
            "profile_path": v.profile_path,
            "index_url": f"{base_url}/index/{v.bos_version}/{INDEX_NAME}",
        }
        for v in variants
    ]
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


def make_cache(nix: Nix, secret: Path, cache: Path, variants: list[Variant]) -> str:
    """Create the signed binary cache under `cache`; returns the public key
    the device must trust."""
    public = nix.generate_cache_key("sysupgrade-e2e-1", secret)
    paths = sorted({p for v in variants for p in index_store_paths(v.index)})
    nix.copy_signed(paths, cache, secret)
    return public


def default_serve_ip(device_host: str, *, port: int = 22) -> str:
    """The IPv4 address the device can reach us on: the source address the
    kernel picks for an AF_INET connection towards the device. Pinned to
    AF_INET — the rig's URLs never bracket IPv6 literals."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(8)
        sock.connect((device_host, port))
        ip: str = sock.getsockname()[0]
        return ip


class RigServer:
    """Serve `root` from a daemon thread; use as a context manager."""

    def __init__(self, root: Path, *, port: int, bind_ip: str = "0.0.0.0") -> None:
        handler = partial(SimpleHTTPRequestHandler, directory=str(root))
        self._httpd = ThreadingHTTPServer((bind_ip, port), handler)
        self._thread = threading.Thread(target=self._httpd.serve_forever, daemon=True)

    @property
    def port(self) -> int:
        """The bound port — the requested one, or the ephemeral pick for 0."""
        return int(self._httpd.server_address[1])

    def __enter__(self) -> "RigServer":
        self._thread.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self._httpd.shutdown()
        self._httpd.server_close()
