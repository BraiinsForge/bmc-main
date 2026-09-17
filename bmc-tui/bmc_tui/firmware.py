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

"""Where a firmware image comes from: a local tar used in place, a direct URL,
or a release picked from the published index. Downloads land in the user's cache,
so a verified tar is flashed to the next device without another fetch."""

import os
import urllib.error
import urllib.request
from collections.abc import Generator
from contextlib import contextmanager
from http.client import HTTPResponse
from pathlib import Path
from urllib.parse import urlsplit

from rich.markup import escape

from bmc_tui import console
from bmc_tui.bos_version import parse_bos_version
from bmc_tui.fw_index import Release, parse_releases
from bmc_tui.image import Image
from bmc_tui.stage import Abort, require, stage

DEFAULT_INDEX_URL = "https://downloads.braiins.com/braiins-deck/index.v1.json"
_CHUNK = 1024 * 1024
_TIMEOUT = 60  # seconds without bytes before a fetch is given up


def obtain(spec: str | None, *, index_url: str) -> Image:
    """The local tarball for `spec`: a path is used in place, a URL is downloaded,
    and no spec at all offers a pick from the release index."""
    if spec is None:
        release = pick_release(fetch_releases(index_url))
        return download(release.url, sha256=release.sha256, size=release.size)
    if is_url(spec):
        return download(spec)
    image = Image(Path(spec))
    require(image.path.is_file(), f"image not found: {console.lit(image.path)}")
    return image


def is_url(spec: str) -> bool:
    return urlsplit(spec).scheme in {"http", "https"}


def cache_dir() -> Path:
    cache_home = os.environ.get("XDG_CACHE_HOME", "")
    base = Path(cache_home) if cache_home else Path.home() / ".cache"
    return base / "deck" / "firmware"


def fetch_releases(url: str) -> list[Release]:
    console.kv("index", url)
    with _opened(url) as response:
        document = response.read().decode(errors="replace")
    try:
        releases = parse_releases(document)
    except ValueError as exc:
        raise Abort(f"cannot read the release index at {console.lit(url)}: {exc}") from None
    require(bool(releases), f"no releases for this platform in {console.lit(url)}")
    return releases


def pick_release(releases: list[Release]) -> Release:
    """Ask for one of `releases`, newest first."""
    newest_first = sorted(releases, key=lambda release: release.release_date, reverse=True)
    picked = console.choose(
        "Release",
        [_release_row(release) for release in newest_first],
        columns=("release", "released", "version", ""),
    )
    if picked is None:
        raise Abort("no terminal to pick a release from — pass --image")
    return newest_first[picked]


def _release_row(release: Release) -> tuple[str, str, str, str]:
    """Table cells: the release name to scan by, then the canonical version
    dimmed beside it, as that is what the device reports as its own."""
    try:
        name = parse_bos_version(release.version).short
    except ValueError:
        name = release.version
    return (
        console.lit(name),
        release.release_date,
        f"[dim]{escape(release.version)}[/dim]",
        "[yellow]major[/yellow]" if release.is_major else "",
    )


def download(url: str, *, sha256: str | None = None, size: int | None = None) -> Image:
    name = urlsplit(url).path.rsplit("/", 1)[-1]
    require(bool(name), f"no file name in {console.lit(url)}")
    dest = cache_dir() / name
    download_firmware(url, dest, sha256=sha256, size=size)
    return Image(dest)


@stage("Download firmware")
def download_firmware(url: str, dest: Path, *, sha256: str | None, size: int | None) -> str:
    """Fetch `url` to `dest`, verified against `sha256` when there is one;
    a cached copy that already matches is kept.
    Without a checksum there is nothing to trust a cached copy against,
    so a direct URL is always fetched afresh."""
    if sha256 is not None and dest.is_file() and Image(dest).sha256 == sha256:
        return f"cached {console.lit(dest)} (sha256 verified)"
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(f"{dest.name}.part")
    try:
        _fetch(url, part, label=dest.name, size=size)
    except BaseException:
        part.unlink(missing_ok=True)
        raise
    part.replace(dest)
    if sha256 is not None and Image(dest).sha256 != sha256:
        dest.unlink()
        raise Abort(f"download corrupted: {console.lit(dest.name)} checksum mismatch")
    verified = " (sha256 verified)" if sha256 is not None else ""
    return f"→ {console.lit(dest)}{verified}"


def _fetch(url: str, dest: Path, *, label: str, size: int | None) -> None:
    received = 0
    with _opened(url) as response, dest.open("wb") as sink:
        total = size if size is not None else _content_length(response)
        with console.progress(label, total) as advance:
            while chunk := response.read(_CHUNK):
                sink.write(chunk)
                advance(len(chunk))
                received += len(chunk)
    # http.client returns short on a cut connection rather than raising,
    # so a truncated tar is only caught by counting the bytes.
    require(
        total is None or received == total,
        f"download of {console.lit(label)} is {console.human_size(received)}, "
        f"expected {console.human_size(total or 0)}",
    )


def _content_length(response: HTTPResponse) -> int | None:
    length = response.headers.get("Content-Length")
    return int(length) if length else None


@contextmanager
def _opened(url: str) -> Generator[HTTPResponse, None, None]:
    """`urlopen` with its failures rendered as an `Abort` naming the URL."""
    try:
        with urllib.request.urlopen(url, timeout=_TIMEOUT) as response:
            yield response
    except urllib.error.HTTPError as exc:
        raise Abort(f"fetch failed: {console.lit(url)} → HTTP {exc.code} {exc.reason}") from None
    except urllib.error.URLError as exc:
        raise Abort(f"fetch failed: {console.lit(url)} → {exc.reason}") from None
    except TimeoutError:
        raise Abort(f"fetch failed: {console.lit(url)} → timed out after {_TIMEOUT}s") from None
