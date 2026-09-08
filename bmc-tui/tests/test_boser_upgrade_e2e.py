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

"""The rig must stay local and must not turn a package check into a firmware offer."""

import hashlib
import io
import json
import tarfile
import urllib.request
from pathlib import Path

import pytest

from bmc_tui import boser_index, catalog, cli
from bmc_tui.bos_version import parse_bos_version
from bmc_tui.fw_index import FwIndexServer
from bmc_tui.image import Image
from bmc_tui.procedures.boser_upgrade_e2e import BoserUpgradeE2e
from bmc_tui.stage import Abort

RUNNING = "2026-08-01-0-acde0123-26.08-plus"
TARGET = "2026-09-01-0-0badc0de-26.09-plus"
STORE = "/nix/store/test-package"


def _image(tmp_path: Path, *, board: str = "stm32mp15_ii2-emmc", version: str = TARGET) -> Image:
    path = tmp_path / "firmware.tar"
    with tarfile.open(path, "w") as archive:
        for name, data in {
            "COMMAND": f'UPGRADE_FW_VERSION="{version}"\n'.encode(),
            "rootfs.img": b"test rootfs",
        }.items():
            member = tarfile.TarInfo(f"sysupgrade-{board}/{name}")
            member.size = len(data)
            archive.addfile(member, io.BytesIO(data))
    return Image(path)


def _document(image: Image | None = None) -> dict:
    return json.loads(
        boser_index.index_document(
            running=parse_bos_version(RUNNING),
            base_url="http://127.0.0.1:8082",
            image=image,
        )
    )


def test_package_only_index_anchors_exact_running_version() -> None:
    doc = _document()
    assert (doc["type"], doc["version"], doc["status"]) == ("bos", "v2", "Active")
    assert len(doc["releases"]) == 1, "no later firmware must compete with package upgrades"
    release = doc["releases"][0]
    assert release["metadata_version"] == "v2"
    metadata = release["metadata"]
    assert metadata["bos_version"] == RUNNING
    assert metadata["is_major"] is False
    assert metadata["is_silent"] is False
    assert metadata["release_date"] == "2026-08-01"
    assert metadata["assets"]["sysupgrade_emmc_stm32mp157c_ii2_bmm1"] == {
        "url": "http://127.0.0.1:8082/anchor.tar"
    }
    assert sum(asset is not None for asset in metadata["assets"].values()) == 1


def test_firmware_offer_follows_anchor_with_real_integrity(tmp_path: Path) -> None:
    image = _image(tmp_path)
    doc = _document(image)
    assert [r["metadata"]["bos_version"] for r in doc["releases"]] == [RUNNING, TARGET]
    asset = doc["releases"][1]["metadata"]["assets"]["sysupgrade_emmc_stm32mp157c_ii2_bmm1"]
    assert asset["integrity"] == {
        "checksum": hashlib.sha256(image.path.read_bytes()).hexdigest(),
        "size_bytes": image.path.stat().st_size,
    }


@pytest.mark.parametrize("board", ["stm32mp15_ii3-emmc", "stm32mp15_ii2-sd"])
def test_non_bmm101_emmc_image_is_rejected(tmp_path: Path, board: str) -> None:
    with pytest.raises(Abort):
        _document(_image(tmp_path, board=board))


def test_same_release_with_different_hash_is_not_a_firmware_upgrade(tmp_path: Path) -> None:
    with pytest.raises(Abort, match="later BOS release"):
        _document(_image(tmp_path, version="2026-08-02-0-ffffffff-26.08-plus"))


def _package_index(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, **extra: object) -> Path:
    path = tmp_path / "packages.json"
    path.write_text(
        json.dumps(
            {
                "version": 1,
                "indexes": [],
                "caches": [],
                "packages": [
                    {
                        "name": "core",
                        "store_path": STORE,
                        "version": "1.0.0",
                        "metadata": {"bmc_version": "2.4.0", "custom": {"value": 42}},
                    }
                ],
                **extra,
            }
        )
    )
    real_is_dir = Path.is_dir
    monkeypatch.setattr(Path, "is_dir", lambda p: str(p) == STORE or real_is_dir(p))
    return path


def test_package_metadata_is_not_rewritten(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    path = _package_index(tmp_path, monkeypatch)
    original = path.read_bytes()
    boser_index.validate_package_index(path)
    assert path.read_bytes() == original


def test_child_indexes_cannot_escape_to_production(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = _package_index(tmp_path, monkeypatch, indexes=["https://production.test"])
    with pytest.raises(Abort, match="flatten child indexes"):
        boser_index.validate_package_index(path)


def test_missing_store_path_fails_before_download(tmp_path: Path) -> None:
    path = tmp_path / "packages.json"
    path.write_text(
        json.dumps(
            {"version": 1, "packages": [{"store_path": "/nix/store/bdk-787-missing-fixture"}]}
        )
    )
    with pytest.raises(Abort, match="not realized locally"):
        boser_index.validate_package_index(path)


def test_actual_http_server_serves_v2_and_exact_image(tmp_path: Path) -> None:
    image = _image(tmp_path)
    (tmp_path / boser_index.INDEX_NAME).write_text(json.dumps(_document(image)))
    with FwIndexServer(tmp_path, port=0, bind_ip="127.0.0.1") as server:
        for name in [boser_index.INDEX_NAME, "firmware.tar"]:
            with urllib.request.urlopen(f"http://127.0.0.1:{server.port}/{name}") as response:
                assert response.read() == (tmp_path / name).read_bytes()


@pytest.mark.parametrize("fail_start", [False, True])
@pytest.mark.parametrize("image_version", [None, TARGET, TARGET.replace("0badc0de", "0BADC0DE")])
def test_runner_owns_servers_but_never_contacts_device(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, fail_start: bool, image_version: str | None
) -> None:
    path = _package_index(tmp_path, monkeypatch)
    monkeypatch.chdir(tmp_path)
    events: list[str] = []
    arguments: list[str] = []

    class FirmwareServer:
        def __init__(self, *_args: object, **_kwargs: object) -> None:
            pass

        def __enter__(self):
            events.append("firmware-start")
            return self

        def __exit__(self, *_args: object) -> None:
            events.append("firmware-stop")

    def launch(cycle: catalog.UpgradeCycle, argv: list[str]) -> None:
        events.append("packages-start")
        arguments.extend(argv)
        if fail_start:
            raise Abort("scripted startup failure")
        cycle.cache_public_key = "dev-upgrade:TEST"

    monkeypatch.setattr("bmc_tui.procedures.boser_upgrade_e2e.FwIndexServer", FirmwareServer)
    monkeypatch.setattr(catalog, "launch_upgrade_server", launch)
    monkeypatch.setattr(
        catalog, "stop_upgrade_server_group", lambda _cycle: events.append("packages-stop")
    )
    monkeypatch.setattr("builtins.input", lambda _prompt: "stop")
    image = _image(tmp_path, version=image_version).path if image_version is not None else None
    runner = BoserUpgradeE2e(path, RUNNING, "127.0.0.1", image=image)
    if fail_start:
        with pytest.raises(Abort, match="scripted startup failure"):
            runner.run()
    else:
        runner.run()
        instructions = next((tmp_path / ".tmp/boser-e2e").glob("*/operator.txt")).read_text()
        assert "BOS_INDEX_URL=http://127.0.0.1:8082 BOSER_UPGRADE_CONSOLE=1" in instructions
        assert "register-server --exclusive" in instructions
        assert "--factory-base-url http://127.0.0.1:8081" in instructions
        assert "/nix/var/nix/gcroots/profiles/bmc/current/bin/bmc-nix-cli" in instructions
        assert "/run/current-profile/bin/bmc-nix-cli" not in instructions
        assert "Snapshot" in instructions and "restore" in instructions
    assert events == ["firmware-start", "packages-start", "firmware-stop", "packages-stop"]
    assert arguments[arguments.index("--base-index") + 1] == str(path)
    assert arguments[arguments.index("--firmware") + 1] == (
        TARGET if image_version is not None else RUNNING
    ), "the package feed must match the advertised firmware, not the raw image version"


def test_boser_rig_is_available_as_subcommand(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit):
        cli.main(["boser-upgrade-e2e", "--help"])
    assert "--package-index" in capsys.readouterr().out
