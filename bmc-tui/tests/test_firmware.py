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

"""Unit tests for resolving a firmware image from a path, a URL or the index."""

import hashlib
import json
from collections.abc import Iterator, Sequence
from pathlib import Path

import pytest

from bmc_tui import console, firmware
from bmc_tui.console import lit
from bmc_tui.fw_index import PLATFORM_ASSET_KEY, FwIndexServer, Release
from bmc_tui.stage import Abort

_FIRMWARE_BYTES = b"firmware fixture\n"
_SHA256 = hashlib.sha256(_FIRMWARE_BYTES).hexdigest()


def _release(version: str, url: str, *, is_major: bool = False) -> Release:
    return Release(
        version=version,
        release_date=version[:10],
        is_major=is_major,
        url=url,
        sha256=_SHA256,
        size=len(_FIRMWARE_BYTES),
    )


def _index(*releases: Release) -> str:
    return json.dumps(
        {
            "version": "v1",
            "releases": [
                {
                    "metadata": {
                        "bmc_version": release.version,
                        "release_date": release.release_date,
                        "is_major": release.is_major,
                        "assets": {
                            PLATFORM_ASSET_KEY: {
                                "url": release.url,
                                "integrity": {
                                    "checksum": release.sha256,
                                    "size_bytes": release.size,
                                },
                            }
                        },
                    }
                }
                for release in releases
            ],
        }
    )


@pytest.fixture
def served(tmp_path: Path) -> Iterator[tuple[FwIndexServer, str, Path]]:
    """A local HTTP root holding `firmware.tar`;
    yields the server, its base URL and the root to add more files to."""
    root = tmp_path / "serve"
    root.mkdir()
    (root / "firmware.tar").write_bytes(_FIRMWARE_BYTES)
    with FwIndexServer(root, port=0, bind_ip="127.0.0.1") as server:
        yield server, f"http://127.0.0.1:{server.port}", root


# ── spec classification ───────────────────────────────────────────────────────


@pytest.mark.parametrize("spec", ["http://host/fw.tar", "https://host/a/fw.tar"])
def test_is_url_for_http_schemes(spec: str) -> None:
    assert firmware.is_url(spec) is True


@pytest.mark.parametrize("spec", ["fw.tar", "./fw.tar", "/abs/fw.tar", "host:fw.tar"])
def test_is_url_false_for_paths(spec: str) -> None:
    assert firmware.is_url(spec) is False


def test_obtain_uses_a_local_path_in_place(tmp_path: Path) -> None:
    local = tmp_path / "fw.tar"
    local.write_bytes(_FIRMWARE_BYTES)
    assert firmware.obtain(str(local), index_url="http://unused/").path == local


def test_obtain_aborts_on_a_missing_local_path(tmp_path: Path) -> None:
    with pytest.raises(Abort, match="image not found"):
        firmware.obtain(str(tmp_path / "missing.tar"), index_url="http://unused/")


def test_cache_dir_honours_xdg_cache_home(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path))
    assert firmware.cache_dir() == tmp_path / "deck" / "firmware"


# ── download ──────────────────────────────────────────────────────────────────


def test_download_verifies_the_checksum_and_keeps_the_tar(
    served: tuple[FwIndexServer, str, Path], tmp_path: Path
) -> None:
    _server, base_url, _root = served
    dest = tmp_path / "cache" / "firmware.tar"

    firmware.download_firmware(f"{base_url}/firmware.tar", dest, sha256=_SHA256, size=None)

    assert dest.read_bytes() == _FIRMWARE_BYTES
    assert not dest.with_name("firmware.tar.part").exists()


def test_download_drops_a_corrupt_tar(
    served: tuple[FwIndexServer, str, Path], tmp_path: Path
) -> None:
    _server, base_url, _root = served
    dest = tmp_path / "firmware.tar"

    with pytest.raises(Abort, match="checksum mismatch"):
        firmware.download_firmware(f"{base_url}/firmware.tar", dest, sha256="0" * 64, size=None)

    assert not dest.exists()
    assert not dest.with_name("firmware.tar.part").exists()


def test_download_aborts_when_fewer_bytes_arrive_than_expected(
    served: tuple[FwIndexServer, str, Path], tmp_path: Path
) -> None:
    _server, base_url, _root = served
    dest = tmp_path / "firmware.tar"

    with pytest.raises(Abort, match="expected"):
        firmware.download_firmware(
            f"{base_url}/firmware.tar", dest, sha256=None, size=len(_FIRMWARE_BYTES) + 1
        )

    assert not dest.exists()
    assert not dest.with_name("firmware.tar.part").exists()


def test_download_reuses_a_verified_cached_tar(
    served: tuple[FwIndexServer, str, Path], tmp_path: Path
) -> None:
    server, base_url, _root = served
    dest = tmp_path / "firmware.tar"
    dest.write_bytes(_FIRMWARE_BYTES)

    firmware.download_firmware(f"{base_url}/firmware.tar", dest, sha256=_SHA256, size=None)

    assert server.requests() == []


def test_download_without_a_checksum_always_fetches(
    served: tuple[FwIndexServer, str, Path], tmp_path: Path
) -> None:
    server, base_url, _root = served
    dest = tmp_path / "firmware.tar"
    dest.write_bytes(b"stale")

    firmware.download_firmware(f"{base_url}/firmware.tar", dest, sha256=None, size=None)

    assert dest.read_bytes() == _FIRMWARE_BYTES
    assert [record.path for record in server.requests()] == ["/firmware.tar"]


def test_download_aborts_on_http_error(
    served: tuple[FwIndexServer, str, Path], tmp_path: Path
) -> None:
    _server, base_url, _root = served
    dest = tmp_path / "missing.tar"

    with pytest.raises(Abort, match="HTTP 404"):
        firmware.download_firmware(f"{base_url}/missing.tar", dest, sha256=None, size=None)

    assert not dest.with_name("missing.tar.part").exists()


def test_download_aborts_on_unreachable_host(tmp_path: Path) -> None:
    with pytest.raises(Abort, match="fetch failed"):
        firmware.download_firmware(
            "http://127.0.0.1:1/fw.tar", tmp_path / "fw.tar", sha256=None, size=None
        )


def test_download_names_the_cached_tar_after_the_url(
    monkeypatch: pytest.MonkeyPatch, served: tuple[FwIndexServer, str, Path], tmp_path: Path
) -> None:
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path))
    _server, base_url, _root = served

    image = firmware.download(f"{base_url}/firmware.tar?token=x")

    assert image.path == tmp_path / "deck" / "firmware" / "firmware.tar"


def test_download_rejects_a_url_without_a_file_name() -> None:
    with pytest.raises(Abort, match="no file name"):
        firmware.download("http://host/releases/")


# ── release index ─────────────────────────────────────────────────────────────


def test_fetch_releases_reads_the_served_index(served: tuple[FwIndexServer, str, Path]) -> None:
    _server, base_url, root = served
    release = _release("2026-09-11-0-ff0d18a2-26.09", f"{base_url}/firmware.tar")
    (root / "index.v1.json").write_text(_index(release))

    assert firmware.fetch_releases(f"{base_url}/index.v1.json") == [release]


def test_fetch_releases_aborts_on_an_empty_index(served: tuple[FwIndexServer, str, Path]) -> None:
    _server, base_url, root = served
    (root / "index.v1.json").write_text(_index())

    with pytest.raises(Abort, match="no releases"):
        firmware.fetch_releases(f"{base_url}/index.v1.json")


def test_fetch_releases_aborts_on_a_foreign_document(
    served: tuple[FwIndexServer, str, Path],
) -> None:
    _server, base_url, root = served
    (root / "index.v1.json").write_text("<html>")

    with pytest.raises(Abort, match="cannot read the release index"):
        firmware.fetch_releases(f"{base_url}/index.v1.json")


def _shown_rows(monkeypatch: pytest.MonkeyPatch, pick: int) -> list[Sequence[Sequence[str]]]:
    """Stub `console.choose` to answer `pick`, collecting the rows it was shown."""
    shown: list[Sequence[Sequence[str]]] = []

    def choose(_question: str, rows: Sequence[Sequence[str]], *, columns: Sequence[str]) -> int:
        shown.append(rows)
        return pick

    monkeypatch.setattr(console, "choose", choose)
    return shown


def test_pick_release_offers_newest_first(monkeypatch: pytest.MonkeyPatch) -> None:
    shown = _shown_rows(monkeypatch, pick=0)
    older = _release("2026-03-04-0-8436f26b-26.02", "http://x/old.tar", is_major=True)
    newer = _release("2026-09-11-0-ff0d18a2-26.09", "http://x/new.tar")

    assert firmware.pick_release([older, newer]) == newer
    assert shown == [
        [
            (lit("26.09"), "2026-09-11", "[dim]2026-09-11-0-ff0d18a2-26.09[/dim]", ""),
            (
                lit("26.02"),
                "2026-03-04",
                "[dim]2026-03-04-0-8436f26b-26.02[/dim]",
                "[yellow]major[/yellow]",
            ),
        ]
    ]


def test_pick_release_shows_an_unparsable_version_verbatim(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    shown = _shown_rows(monkeypatch, pick=0)
    odd = Release(
        version="nightly-build-7",
        release_date="2026-09-11",
        is_major=False,
        url="http://x/fw.tar",
        sha256=None,
        size=None,
    )

    firmware.pick_release([odd])

    assert shown[0][0][0] == lit("nightly-build-7")


def test_pick_release_aborts_without_a_terminal(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(console, "choose", lambda *_args, **_kwargs: None)

    with pytest.raises(Abort, match="pass --image"):
        firmware.pick_release([_release("2026-09-11-0-ff0d18a2-26.09", "http://x/fw.tar")])


def test_obtain_from_the_index_downloads_the_picked_release(
    monkeypatch: pytest.MonkeyPatch, served: tuple[FwIndexServer, str, Path], tmp_path: Path
) -> None:
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path))
    monkeypatch.setattr(console, "choose", lambda *_args, **_kwargs: 0)
    _server, base_url, root = served
    release = _release("2026-09-11-0-ff0d18a2-26.09", f"{base_url}/firmware.tar")
    (root / "index.v1.json").write_text(_index(release))

    image = firmware.obtain(None, index_url=f"{base_url}/index.v1.json")

    assert image.path == tmp_path / "deck" / "firmware" / "firmware.tar"
    assert image.sha256 == _SHA256
