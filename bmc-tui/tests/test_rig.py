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

"""Unit tests for the e2e package rig."""

import json
import urllib.error
import urllib.request
from pathlib import Path

import pytest

from bmc_tui import rig
from bmc_tui.nix import Attr, Built, Pkg, StorePath

_PROFILE = "/nix/var/nix/gcroots/profiles/bmc"


def _variant(tmp_path: Path, version: str, store_paths: list[str]) -> rig.Variant:
    index = tmp_path / f"index-{version}"
    index.mkdir()
    packages = [
        {"name": f"pkg{i}", "version": "1.0", "store_path": p} for i, p in enumerate(store_paths)
    ]
    (index / rig.INDEX_NAME).write_text(
        json.dumps({"version": 1, "indexes": [], "caches": [], "packages": packages})
    )
    tarball_dir = tmp_path / f"tarball-{version}"
    tarball_dir.mkdir()
    tarball = tarball_dir / f"nix-{version}.tar.gz"
    tarball.write_bytes(b"tar-bytes-" + version.encode())
    return rig.Variant(bos_version=version, profile_path=_PROFILE, index=index, tarball=tarball)


def _get(url: str) -> bytes:
    with urllib.request.urlopen(url) as response:
        return response.read()


def test_feed_document_links_each_variant(tmp_path: Path) -> None:
    a = _variant(tmp_path, "va", ["/nix/store/a"])
    b = _variant(tmp_path, "vb", ["/nix/store/b"])
    doc = json.loads(rig.feed_document([a, b], "http://10.0.0.1:8083"))
    assert doc["version"] == 1
    assert doc["entries"] == [
        {
            "bos_version": "va",
            "download_url": "http://10.0.0.1:8083/tarballs/nix-va.tar.gz",
            "profile_path": _PROFILE,
            "index_url": f"http://10.0.0.1:8083/index/va/{rig.INDEX_NAME}",
        },
        {
            "bos_version": "vb",
            "download_url": "http://10.0.0.1:8083/tarballs/nix-vb.tar.gz",
            "profile_path": _PROFILE,
            "index_url": f"http://10.0.0.1:8083/index/vb/{rig.INDEX_NAME}",
        },
    ]


def test_index_store_paths_and_package_lookup(tmp_path: Path) -> None:
    v = _variant(tmp_path, "va", ["/nix/store/one", "/nix/store/two"])
    assert rig.index_store_paths(v.index) == ["/nix/store/one", "/nix/store/two"]
    assert rig.package_store_path(v.index, "pkg1") == "/nix/store/two"
    assert rig.package_store_path(v.index, "absent") is None


def test_serve_root_serves_feed_index_and_tarball(tmp_path: Path) -> None:
    a = _variant(tmp_path, "va", ["/nix/store/a"])
    root = tmp_path / "serve"
    rig.write_serve_root(root, [a], "http://placeholder")
    with rig.RigServer(root, port=0, bind_ip="127.0.0.1") as server:
        base = f"http://127.0.0.1:{server.port}"
        feed = json.loads(_get(f"{base}/{rig.FEED_NAME}"))
        assert feed["entries"][0]["bos_version"] == "va"
        assert json.loads(_get(f"{base}/index/va/{rig.INDEX_NAME}"))["packages"]
        assert _get(f"{base}/tarballs/nix-va.tar.gz") == b"tar-bytes-va"


def test_write_serve_root_is_rerunnable(tmp_path: Path) -> None:
    a = _variant(tmp_path, "va", ["/nix/store/a"])
    root = tmp_path / "serve"
    rig.write_serve_root(root, [a], "http://placeholder")
    rig.write_serve_root(root, [a], "http://placeholder")
    assert (root / "tarballs" / "nix-va.tar.gz").read_bytes() == b"tar-bytes-va"


def test_rig_server_serves_content_added_after_start(tmp_path: Path) -> None:
    root = tmp_path / "serve"
    root.mkdir()
    with rig.RigServer(root, port=0, bind_ip="127.0.0.1") as server:
        a = _variant(tmp_path, "va", ["/nix/store/a"])
        rig.write_serve_root(root, [a], f"http://127.0.0.1:{server.port}")
        feed = json.loads(_get(f"http://127.0.0.1:{server.port}/{rig.FEED_NAME}"))
        assert feed["entries"][0]["bos_version"] == "va"


def test_make_cache_signs_the_union_of_index_paths(tmp_path: Path) -> None:
    copied: list[tuple[list[StorePath], Path, Path]] = []

    class _Nix:
        def generate_cache_key(self, name: str, secret: Path) -> str:
            secret.write_text("sk")
            return f"{name}:PUB"

        def copy_signed(self, store_paths: list[StorePath], cache: Path, secret: Path) -> None:
            copied.append((store_paths, cache, secret))

        def discover_widgets(self) -> list[str]:
            return []

        def list_packages(self) -> list[str]:
            return []

        def resolve(self, attr: Attr) -> Pkg:
            raise NotImplementedError

        def build(self, pkgs: list[Pkg]) -> list[Built]:
            return []

        def build_out(self, attr: Attr) -> StorePath:
            raise NotImplementedError

        def out_path(self, attr: Attr) -> StorePath:
            raise NotImplementedError

        def build_file(self, file: str, attrs: list[Attr], args: dict[str, str]) -> list[StorePath]:
            raise NotImplementedError

        def copy(self, store_paths: list[StorePath], dest: str) -> None:
            return None

    a = _variant(tmp_path, "va", ["/nix/store/shared", "/nix/store/a-only"])
    b = _variant(tmp_path, "vb", ["/nix/store/shared", "/nix/store/b-only"])
    secret = tmp_path / "key.secret"
    public = rig.make_cache(_Nix(), secret, tmp_path / "cache", [a, b])
    assert public == "sysupgrade-e2e-1:PUB"
    assert secret.read_text() == "sk"
    assert copied == [
        (
            ["/nix/store/a-only", "/nix/store/b-only", "/nix/store/shared"],
            tmp_path / "cache",
            secret,
        )
    ]


def _served(tmp_path: Path) -> Path:
    root = tmp_path / "serve"
    rig.write_serve_root(root, [_variant(tmp_path, "va", ["/nix/store/a"])], "http://placeholder")
    return root


def test_rig_server_head_reports_size_without_a_body(tmp_path: Path) -> None:
    with rig.RigServer(_served(tmp_path), port=0, bind_ip="127.0.0.1") as server:
        url = f"http://127.0.0.1:{server.port}/tarballs/nix-va.tar.gz"
        with urllib.request.urlopen(urllib.request.Request(url, method="HEAD")) as response:
            assert response.status == 200
            assert int(response.headers["Content-Length"]) == len(b"tar-bytes-va")
            assert response.read() == b""


def test_rig_server_404s_unknown_paths_and_directories(tmp_path: Path) -> None:
    """Directories are not listed; every rig consumer fetches an exact path."""
    with rig.RigServer(_served(tmp_path), port=0, bind_ip="127.0.0.1") as server:
        base = f"http://127.0.0.1:{server.port}"
        for path in ("/nope.json", "/tarballs/"):
            with pytest.raises(urllib.error.HTTPError) as caught:
                _get(f"{base}{path}")
            assert caught.value.status == 404


def test_rig_server_port_before_start_is_a_bug(tmp_path: Path) -> None:
    with pytest.raises(RuntimeError, match="before it was started"):
        _ = rig.RigServer(tmp_path, port=0).port
