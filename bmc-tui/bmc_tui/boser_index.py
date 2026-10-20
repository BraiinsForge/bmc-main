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

"""BOS v2 firmware fixtures for BMM101's upgrade console."""

import json
from pathlib import Path
from typing import NotRequired, TypedDict

import msgspec

from bmc_tui import catalog
from bmc_tui.bos_version import BosVersion, parse_bos_version
from bmc_tui.fw_index import AssetObject, release_uuid
from bmc_tui.image import Image
from bmc_tui.stage import Abort, require

INDEX_NAME = "index.v2.json"
_BMM101_EMMC_ASSET = "sysupgrade_emmc_stm32mp157c_ii2_bmm1"
_BMM101_EMMC_TARGET = "sysupgrade-stm32mp15_ii2-emmc"
_ASSET_KEYS = (
    "transitional_am1_s9",
    "transitional_am2_s17",
    "transitional_am3_aml",
    "transitional_am3_bbb",
    "transitional_cvitek_bm1_am2",
    "transitional_zynq_bm3_am2",
    "sysupgrade_nand_am1_s9",
    "sysupgrade_nand_am2_s17",
    "sysupgrade_nand_am3_aml",
    "sysupgrade_nand_am3_bbb",
    "sysupgrade_nand_zynq_bm3_am2",
    "sysupgrade_sd_am1_s9",
    "sysupgrade_sd_am2_s17",
    "sysupgrade_sd_am3_bbb",
    "sysupgrade_sd_stm32mp157c_ii1_am2",
    "sysupgrade_sd_stm32mp157c_ii2_bmm1",
    "sysupgrade_emmc_cvitek_bm1_am2",
    "sysupgrade_emmc_stm32mp15_ii1_am2",
    "sysupgrade_emmc_stm32mp157c_ii2_bmm1",
)


class ReleaseMetadata(TypedDict):
    bos_version: str
    is_major: bool
    is_silent: bool
    release_date: str
    description: str
    assets: dict[str, AssetObject | None]


class ReleaseDocument(TypedDict):
    uuid: str
    metadata_version: str
    metadata: ReleaseMetadata


class IndexDocument(TypedDict):
    type: str
    status: str
    title: str
    version: str
    releases: list[ReleaseDocument]


def index_document(*, running: BosVersion, base_url: str, image: Image | None) -> str:
    releases = [_release(running, _BMM101_EMMC_ASSET, {"url": f"{base_url}/anchor.tar"})]
    if image is not None:
        catalog.validate_firmware_image(image, device_target="stm32mp15_ii2")
        require(
            image.sysupgrade_dir == _BMM101_EMMC_TARGET,
            "image is not a BMM101 eMMC sysupgrade",
        )
        target = parse_bos_version(image.version)
        require(target.version > running.version, "image must have a later BOS release version")
        releases.append(
            _release(
                target,
                _BMM101_EMMC_ASSET,
                {
                    "url": f"{base_url}/firmware.tar",
                    "integrity": {"checksum": image.sha256, "size_bytes": image.size},
                },
            )
        )
    document: IndexDocument = {
        "type": "bos",
        "status": "Active",
        "title": "BDK-787 BMM101 upgrade test",
        "version": "v2",
        "releases": releases,
    }
    return json.dumps(document, indent=2)


def _release(version: BosVersion, key: str, asset: AssetObject) -> ReleaseDocument:
    assets: dict[str, AssetObject | None] = {asset_key: None for asset_key in _ASSET_KEYS}
    assets[key] = asset
    return {
        "uuid": release_uuid(version.canonical),
        "metadata_version": "v2",
        "metadata": {
            "bos_version": version.canonical,
            "is_major": False,
            "is_silent": False,
            "release_date": version.release_date,
            "description": "BDK-787 controlled upgrade",
            "assets": assets,
        },
    }


class _IndexedPackage(TypedDict):
    store_path: str


class _PackageIndex(TypedDict):
    version: int
    packages: list[_IndexedPackage]
    indexes: NotRequired[list[msgspec.Raw]]


def validate_package_index(path: Path) -> None:
    try:
        document = msgspec.json.decode(path.read_bytes(), type=_PackageIndex)
    except msgspec.DecodeError as error:
        raise Abort(f"package index is malformed: {error}") from None
    require(document["version"] == 1, "package index must use version 1")
    require(not document.get("indexes"), "flatten child indexes before using the E2E rig")
    packages = document["packages"]
    require(bool(packages), "package index must contain packages")
    for package in packages:
        store_path = package["store_path"]
        require(bool(store_path), "package needs a store_path")
        require(
            Path(store_path).parent == Path("/nix/store") and Path(store_path).is_dir(),
            f"package store path is not realized locally: {store_path}",
        )
