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

"""The rig stays local until the operator is ready, then drives Boser over REST."""

import hashlib
import io
import json
import subprocess
import tarfile
import urllib.request
from collections.abc import Callable, Iterable
from pathlib import Path

import pytest
import tyro

from bmc_tui import boser_index, catalog, cli
from bmc_tui.bos_version import parse_bos_version
from bmc_tui.device import Device
from bmc_tui.fw_index import FwIndexServer
from bmc_tui.image import Image
from bmc_tui.procedures.boser_upgrade_e2e import (
    BoserUpgradeE2e,
    check_summary,
    event_summary,
    phase_label,
)
from bmc_tui.stage import Abort
from tests import boser_stub
from tests.boser_stub import BoserStub, Script

RUNNING = "2026-08-01-0-acde0123-26.08-plus"
TARGET = "2026-09-01-0-0badc0de-26.09-plus"
STORE = "/nix/store/test-package"
CHECK_FIRMWARE_ONLY = {
    **boser_stub.CHECK_FIRMWARE,
    "offer": {"id": boser_stub.OFFER_ID, "kind": "FIRMWARE", "disruption": "REBOOT"},
    "packages": None,
}


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


def _rig(
    monkeypatch: pytest.MonkeyPatch, events: list[str], *, fail_start: bool = False
) -> list[str]:
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
    return arguments


PROFILE = "/nix/var/nix/gcroots/profiles/bmc"


class _DeviceScript:
    """Scripted SSH answers for the verification: tests mutate the fields to play the device."""

    def __init__(self) -> None:
        self.boot_id = "boot-1"
        self.generation = 1
        self.manifest: dict = {"packages": {"core": {"version": "1.0.0"}}}
        self.bos_version = RUNNING
        self.reachable = True
        self.commands: list[str] = []

    def make(self, host: str) -> Device:
        assert host == "dev"
        return Device(host, backend=self)

    def run(self, argv: list[str]) -> subprocess.CompletedProcess[str]:
        command = argv[-1]
        self.commands.append(command)
        if not self.reachable:
            raise subprocess.CalledProcessError(255, argv)
        answers = {
            "cat /proc/sys/kernel/random/boot_id": self.boot_id,
            f"readlink {PROFILE}/current": f"{PROFILE}/{self.generation}-link",
            f"cat {PROFILE}/current/manifest": json.dumps(self.manifest),
            "cat /etc/bos_version": self.bos_version,
        }
        return subprocess.CompletedProcess(argv, 0, stdout=answers[command] + "\n", stderr="")

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        raise AssertionError("the runner never uploads")

    def stream_output(self, argv: list[str], on_line: Callable[[str], None]) -> int:
        raise AssertionError("the runner never streams a command")

    def install_weather(self) -> None:
        self.generation += 2  # leftover generations make the new number skip ahead
        self.manifest["packages"]["weather"] = {"version": "1.2.0"}

    def flash(self) -> None:
        self.boot_id = "boot-2"
        self.bos_version = TARGET
        self.install_weather()


class _Clock:
    """Time that only advances when the runner sleeps."""

    def __init__(self) -> None:
        self.now = 0.0

    def sleep(self, seconds: float) -> None:
        self.now += seconds


def _answers(*replies: str) -> Callable[[str], str]:
    queue = iter(replies)

    def ask(prompt: str) -> str:
        reply = next(queue)
        if reply == "ready":
            assert prompt.startswith("Type ready")
        return reply

    return ask


@pytest.mark.parametrize("image_version", [None, TARGET, TARGET.replace("0badc0de", "0BADC0DE")])
def test_runner_drives_the_offer_over_rest_after_the_operator_is_ready(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, image_version: str | None
) -> None:
    path = _package_index(tmp_path, monkeypatch)
    monkeypatch.chdir(tmp_path)
    events: list[str] = []
    arguments = _rig(monkeypatch, events)
    image = _image(tmp_path, version=image_version).path if image_version is not None else None
    device = _DeviceScript()
    script = Script(password="secret", before_terminal=device.install_weather)
    with BoserStub(script) as stub:
        runner = BoserUpgradeE2e(
            stub.address,
            path,
            RUNNING,
            "127.0.0.1",
            "dev",
            image=image,
            packages=["weather"],
            password="secret",
        )
        runner.run(ask=_answers("nope", "ready", "yes", "stop"), make_device=device.make)
    instructions = next((tmp_path / ".tmp/boser-e2e").glob("*/operator.txt")).read_text()
    assert "BOS_INDEX_URL=http://127.0.0.1:8082 /path/to/boser-openwrt" in instructions
    assert "BOSER_UPGRADE_CONSOLE" not in instructions
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
    assert [path for _m, path, _t, _b in script.requests] == [
        "/api/v1/auth/login",
        "/api/v1/upgrade/packages/installable",
        "/api/v1/upgrade/check",
        "/api/v1/upgrade/start",
        "/api/v1/upgrade/state/events",
        "/api/v1/upgrade/check",
    ], "the device is contacted only after the operator typed ready"
    assert script.requests[2][3] == {"packages": ["weather"]}
    assert device.commands == [
        "cat /proc/sys/kernel/random/boot_id",
        f"readlink {PROFILE}/current",
        "cat /proc/sys/kernel/random/boot_id",
        f"readlink {PROFILE}/current",
        f"cat {PROFILE}/current/manifest",
    ], "the profile is snapshotted before start and compared after COMPLETED"


def test_host_startup_failure_stops_the_rig_before_touching_the_device(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = _package_index(tmp_path, monkeypatch)
    monkeypatch.chdir(tmp_path)
    events: list[str] = []
    _rig(monkeypatch, events, fail_start=True)
    script = Script()
    with BoserStub(script) as stub:
        runner = BoserUpgradeE2e(stub.address, path, RUNNING, "127.0.0.1", "dev")
        with pytest.raises(Abort, match="scripted startup failure"):
            runner.run(ask=_answers("ready", "stop"))
    assert events == ["firmware-start", "packages-start", "firmware-stop", "packages-stop"]
    assert script.requests == []


@pytest.mark.parametrize(
    ("script", "replies", "started"),
    [
        (Script(check=dict(boser_stub.CHECK_NOTHING)), ("ready", "stop"), False),
        (Script(), ("ready", "no", "stop"), False),
    ],
)
def test_nothing_starts_without_an_offer_and_a_yes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    script: Script,
    replies: tuple[str, ...],
    started: bool,
) -> None:
    path = _package_index(tmp_path, monkeypatch)
    monkeypatch.chdir(tmp_path)
    _rig(monkeypatch, [])
    with BoserStub(script) as stub:
        BoserUpgradeE2e(stub.address, path, RUNNING, "127.0.0.1", "dev").run(ask=_answers(*replies))
    assert ("/api/v1/upgrade/start" in [p for _m, p, _t, _b in script.requests]) is started


def test_failed_run_aborts_with_the_device_reason(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = _package_index(tmp_path, monkeypatch)
    monkeypatch.chdir(tmp_path)
    events: list[str] = []
    _rig(monkeypatch, events)
    script = Script(events=[boser_stub.RUNNING, boser_stub.FAILED])
    with BoserStub(script) as stub:
        runner = BoserUpgradeE2e(stub.address, path, RUNNING, "127.0.0.1", "dev")
        with pytest.raises(
            Abort, match="failed in PACKAGES/ACTIVATING: activation script exited 1"
        ):
            runner.run(ask=_answers("ready", "yes"), make_device=_DeviceScript().make)
    assert events[-2:] == ["firmware-stop", "packages-stop"]


def test_rebooting_run_waits_for_the_new_boot_and_checks_the_firmware(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = _package_index(tmp_path, monkeypatch)
    monkeypatch.chdir(tmp_path)
    _rig(monkeypatch, [])
    device = _DeviceScript()
    clock = _Clock()
    script = Script(
        check=dict(boser_stub.CHECK_FIRMWARE), events=[boser_stub.RUNNING, boser_stub.REBOOTING]
    )
    script.before_terminal = device.flash
    with BoserStub(script) as stub:
        runner = BoserUpgradeE2e(stub.address, path, RUNNING, "127.0.0.1", "dev")
        runner.run(
            ask=_answers("ready", "yes", "stop"),
            clock=lambda: clock.now,
            sleep=clock.sleep,
            make_device=device.make,
        )
    assert [p for _m, p, _t, _b in script.requests][-4:] == [
        "/api/v1/upgrade/state/events",
        "/api/v1/auth/login",
        "/api/v1/upgrade/state",
        "/api/v1/upgrade/check",
    ], "after the reboot the runner logs into the fresh Boser before it re-checks"
    assert "cat /etc/bos_version" in device.commands


def _completed_with_wrong_generation(device: _DeviceScript) -> None:
    device.manifest["packages"]["weather"] = {"version": "1.2.0"}


def _completed_with_wrong_version(device: _DeviceScript) -> None:
    device.generation += 1
    device.manifest["packages"]["weather"] = {"version": "1.1.0"}


def _completed_but_rebooted(device: _DeviceScript) -> None:
    device.install_weather()
    device.boot_id = "boot-2"


def _rebooting_without_a_flash(device: _DeviceScript) -> None:
    device.install_weather()
    device.boot_id = "boot-2"


def _flash_without_a_package_change(device: _DeviceScript) -> None:
    device.boot_id = "boot-2"
    device.bos_version = TARGET


def _flash_moving_the_profile(device: _DeviceScript) -> None:
    _flash_without_a_package_change(device)
    device.generation += 1


@pytest.mark.parametrize(
    ("script", "play", "message"),
    [
        (Script(), _completed_with_wrong_generation, "expected a newer generation than 1"),
        (Script(), _completed_with_wrong_version, "weather: the manifest has 1.1.0"),
        (Script(), _completed_but_rebooted, "rebooted during an upgrade that reported COMPLETED"),
        (
            Script(recheck=dict(boser_stub.CHECK_OFFER)),
            _DeviceScript.install_weather,
            "still offers",
        ),
        (
            Script(check=dict(boser_stub.CHECK_FIRMWARE), events=[boser_stub.REBOOTING]),
            _DeviceScript.install_weather,
            "did not come back with a new boot id",
        ),
        (
            Script(check=dict(boser_stub.CHECK_FIRMWARE), events=[boser_stub.REBOOTING]),
            _rebooting_without_a_flash,
            "/etc/bos_version is 2026-08-01-0-acde0123-26.08-plus",
        ),
        (
            Script(
                check=dict(boser_stub.CHECK_FIRMWARE),
                events=[boser_stub.REBOOTING],
                state=dict(boser_stub.RUNNING),
            ),
            _DeviceScript.flash,
            "reports an execution after the reboot",
        ),
        (
            Script(check=dict(CHECK_FIRMWARE_ONLY), events=[boser_stub.REBOOTING]),
            _flash_moving_the_profile,
            "moved to generation 2 without a package change",
        ),
        (
            Script(check={**boser_stub.CHECK_OFFER, "package_capability": {"status": "ABSENT"}}),
            _DeviceScript.install_weather,
            "packages were offered on a device without a package store",
        ),
        (
            Script(
                check={
                    **boser_stub.CHECK_FIRMWARE,
                    "package_capability": {"status": "UNHEALTHY", "reason": "nix.conf is gone"},
                }
            ),
            _DeviceScript.flash,
            "package store is unhealthy: nix.conf is gone",
        ),
    ],
)
def test_a_terminal_state_the_device_does_not_back_up_fails_the_run(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    script: Script,
    play: Callable[[_DeviceScript], None],
    message: str,
) -> None:
    path = _package_index(tmp_path, monkeypatch)
    monkeypatch.chdir(tmp_path)
    _rig(monkeypatch, [])
    device = _DeviceScript()
    clock = _Clock()
    script.before_terminal = lambda: play(device)
    with BoserStub(script) as stub:
        runner = BoserUpgradeE2e(stub.address, path, RUNNING, "127.0.0.1", "dev")
        with pytest.raises(Abort, match=message):
            runner.run(
                ask=_answers("ready", "yes"),
                clock=lambda: clock.now,
                sleep=clock.sleep,
                make_device=device.make,
            )


def test_the_runner_waits_through_a_device_that_is_down_while_it_reboots(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = _package_index(tmp_path, monkeypatch)
    monkeypatch.chdir(tmp_path)
    _rig(monkeypatch, [])
    device = _DeviceScript()
    clock = _Clock()
    script = Script(
        check=dict(boser_stub.CHECK_FIRMWARE), events=[boser_stub.RUNNING, boser_stub.REBOOTING]
    )

    def go_down() -> None:
        device.flash()
        device.reachable = False
        script.password = "still booting"

    def sleep(seconds: float) -> None:
        clock.sleep(seconds)
        device.reachable = device.reachable or clock.now >= 10.0
        if clock.now >= 20.0:
            script.password = ""

    script.before_terminal = go_down
    with BoserStub(script) as stub:
        runner = BoserUpgradeE2e(stub.address, path, RUNNING, "127.0.0.1", "dev")
        runner.run(
            ask=_answers("ready", "yes", "stop"),
            clock=lambda: clock.now,
            sleep=sleep,
            make_device=device.make,
        )
    logins = [p for _m, p, _t, _b in script.requests].count("/api/v1/auth/login")
    assert logins > 2, "the runner retries the login until Boser is back"
    assert clock.now >= 20.0, "the run waited for both the SSH and the Boser side"


def test_a_shared_reboot_deadline_bounds_the_ssh_and_boser_waits_together(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = _package_index(tmp_path, monkeypatch)
    monkeypatch.chdir(tmp_path)
    _rig(monkeypatch, [])
    device = _DeviceScript()
    clock = _Clock()
    script = Script(check=dict(boser_stub.CHECK_FIRMWARE), events=[boser_stub.REBOOTING])

    def flash_slowly() -> None:
        device.reachable = False
        script.password = "still booting"

    def sleep(seconds: float) -> None:
        clock.sleep(seconds)
        if clock.now >= 40.0 and not device.reachable:
            device.flash()
            device.reachable = True

    script.before_terminal = flash_slowly
    with BoserStub(script) as stub:
        runner = BoserUpgradeE2e(
            stub.address, path, RUNNING, "127.0.0.1", "dev", reboot_deadline=60.0
        )
        with pytest.raises(Abort, match="Boser did not come back within --reboot-deadline"):
            runner.run(
                ask=_answers("ready", "yes"),
                clock=lambda: clock.now,
                sleep=sleep,
                make_device=device.make,
            )
    assert clock.now <= 65.0, "the Boser wait gets only what the reboot wait left"


def test_a_firmware_only_offer_must_leave_the_profile_alone(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = _package_index(tmp_path, monkeypatch)
    monkeypatch.chdir(tmp_path)
    _rig(monkeypatch, [])
    device = _DeviceScript()
    clock = _Clock()
    script = Script(check=dict(CHECK_FIRMWARE_ONLY), events=[boser_stub.REBOOTING])
    script.before_terminal = lambda: _flash_without_a_package_change(device)
    with BoserStub(script) as stub:
        runner = BoserUpgradeE2e(stub.address, path, RUNNING, "127.0.0.1", "dev")
        runner.run(
            ask=_answers("ready", "yes", "stop"),
            clock=lambda: clock.now,
            sleep=clock.sleep,
            make_device=device.make,
        )
    assert device.commands.count(f"readlink {PROFILE}/current") == 2, (
        "the generation is read before and after so a moved profile cannot hide"
    )


def test_cli_requires_the_ssh_address() -> None:
    with pytest.raises(SystemExit):
        tyro.cli(
            BoserUpgradeE2e,
            args=[
                "--device",
                "10.0.0.1",
                "--package-index",
                "index.json",
                "--running-version",
                RUNNING,
                "--serve-ip",
                "127.0.0.1",
            ],
        )


def test_phase_label_joins_stage_and_step() -> None:
    assert phase_label({"stage": "PREPARING"}) == "PREPARING"
    assert phase_label({"stage": "PACKAGES", "step": "REALIZING"}) == "PACKAGES/REALIZING"


def test_summaries_read_like_the_console_did() -> None:
    assert check_summary(boser_stub.CHECK_OFFER) == [
        "package capability READY",
        "package weather None -> 1.2.0",
        "packages download 1048576 bytes",
        f"offer {boser_stub.OFFER_ID} PACKAGES NONE",
    ]
    assert check_summary(boser_stub.CHECK_NOTHING) == ["package capability READY", "no offer"]
    assert [event_summary(e) for e in (boser_stub.RUNNING, boser_stub.REALIZING)] == [
        "RUNNING/PREPARING",
        "RUNNING/PACKAGES/REALIZING/524288",
    ]


def test_boser_rig_is_available_as_subcommand(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit):
        cli.main(["boser-upgrade-e2e", "--help"])
    assert "--ssh" in capsys.readouterr().out
