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

"""Restore-state tests for the firmware gRPC upgrade procedure."""

import io
import json
import os
import signal
import socket
import subprocess
import tarfile
import time
from collections.abc import Callable, Generator, Iterable
from dataclasses import dataclass
from http import HTTPStatus
from pathlib import Path
from typing import NoReturn, cast, override

import pytest

from bmc_tui import catalog, fw_index
from bmc_tui.bos_version import parse_bos_version
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.json_value import Json
from bmc_tui.procedures import e2e_grpc_sysupgrade
from bmc_tui.stage import Abort
from tests import stubs
from tests.test_catalog import _Nix

_classify_stream = catalog.classify_stream
_check_for_firmware_upgrade = catalog.check_for_firmware_upgrade
_require_fresh_firmware_staging = catalog.require_fresh_firmware_staging


_BOOT_ID = "cat /proc/sys/kernel/random/boot_id"
_BOARD = json.dumps({"board_name": "deck", "release": {"target": "stm32mp15/ii3"}})


class DeviceExec:
    """The device's side of the commands the procedure sends through `Device` itself.

    `replies` maps a command substring to its stdout, or to the error running it raises.
    """

    def __init__(self, events: list[str]) -> None:
        self.events: list[str] = events
        self.replies: dict[str, str | BaseException] = {
            _BOOT_ID: "boot-a",
            "ubus call system board": _BOARD,
        }

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        command = argv[-1]
        if command == "service bmc-compositor start":
            self.events.append("start stock")
        reply = next((reply for key, reply in self.replies.items() if key in command), "")
        if isinstance(reply, BaseException):
            raise reply
        return subprocess.CompletedProcess(argv, 0, stdout=reply, stderr="")

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        raise AssertionError(f"BUG: the run streamed to the device: {argv} ({chunks})")

    def stream_output(self, argv: list[str], on_line: Callable[[str], None]) -> int:
        raise AssertionError(f"BUG: the run streamed from the device: {argv} ({on_line})")


def fake_device(events: list[str], host: str = "deck") -> Device:
    return Device(host, backend=DeviceExec(events))


class _NoSsh:
    """An Exec for a run that must abort before it reaches the device."""

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        raise AssertionError(f"BUG: the run reached ssh: {argv}")

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        raise AssertionError(f"BUG: the run reached ssh: {argv} ({chunks})")

    def stream_output(self, argv: list[str], on_line: Callable[[str], None]) -> int:
        raise AssertionError(f"BUG: the run reached ssh: {argv} ({on_line})")


class IndexServer(fw_index.FwIndexServer):
    """A real index server whose transfer record the test scripts."""

    def __init__(self, root: Path, *, provenance: bool = True) -> None:
        super().__init__(root, port=0, bind_ip="127.0.0.1")
        self.root: Path = root
        self.provenance: bool = provenance
        self.entered: bool = False
        self.stopped: bool = False
        self.index_text: str | None = None
        self.transfers: int = 0

    def release(self) -> None:
        """Stop a server the run left serving, so its thread does not outlive the test,
        and close one it never entered, so its socket does not either."""
        if not self.entered:
            self._httpd.server_close()
        elif not self.stopped:
            super().__exit__()

    @override
    def __enter__(self) -> "IndexServer":
        _ = super().__enter__()
        self.entered = True
        index_path = next(self.root.rglob("index.v1.json"), None)
        self.index_text = index_path.read_text() if index_path is not None else None
        return self

    @override
    def __exit__(self, *_exc: object) -> None:
        super().__exit__()
        self.stopped = True

    @override
    def completed(self, path: str) -> bool:
        return (path == "/firmware.tar" and self.provenance) or path == "/index.v1.json"

    @override
    def requests(self) -> list[fw_index.RequestRecord]:
        record = fw_index.RequestRecord(
            path="/firmware.tar",
            status=HTTPStatus.OK,
            bytes_attempted=0,
            bytes_written=0,
            complete=self.provenance,
            error=None,
        )
        return [record]

    @override
    def active_transfers(self) -> int:
        return self.transfers


def _image(path: Path) -> Image:
    image = path / "firmware.tar"
    with tarfile.open(image, "w") as archive:
        for name, data in {
            "COMMAND": b'UPGRADE_FW_VERSION="2025-07-01-0-0badc0de-25.07"\n',
            "rootfs.img": b"rootfs",
            "bmc-nix-cli": b"cli",
            "servers.json.default": b"{}",
        }.items():
            info = tarfile.TarInfo(f"sysupgrade-stm32mp15_ii3-emmc/{name}")
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))
    return Image(image)


type Stream = Callable[[Device, catalog.FirmwareCycle], catalog.StreamResult]


@dataclass
class State:
    """What the stubbed stages saw, and the knobs tests turn on them."""

    cycle: catalog.FirmwareCycle | None = None
    outcome: catalog.StreamOutcome = catalog.StreamOutcome.PROVISIONAL_SUCCESS
    firmwares: list[str] | None = None
    core_store_path: str = "/nix/store/abc-bmc-bmc-core"

    @property
    def run_cycle(self) -> catalog.FirmwareCycle:
        assert self.cycle is not None, "the run never reached preflight_versions"
        return self.cycle


@dataclass
class Harness:
    procedure: e2e_grpc_sysupgrade.E2eGrpcSysupgrade
    dev: Device
    remote: DeviceExec
    backend: _Nix
    server: IndexServer
    state: State
    events: list[str]
    snapshot: Path
    stream: Stream


@pytest.fixture
def harness(  # noqa: PLR0915
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> Generator[Harness]:
    events: list[str] = []
    snapshot = tmp_path / "snapshot"
    snapshot.mkdir()
    server = IndexServer(snapshot)
    state = State()

    monkeypatch.setattr("tempfile.mkdtemp", stubs.returning(str(snapshot)))
    monkeypatch.setattr(
        "bmc_tui.procedures.e2e_grpc_sysupgrade.default_serve_ip",
        stubs.returning("192.0.2.1"),
    )
    monkeypatch.setattr(catalog, "ensure_grpcurl", lambda: None)
    monkeypatch.setattr(catalog, "ensure_device_reachable", stubs.returning(None))
    monkeypatch.setattr(catalog, "validate_firmware_image", stubs.returning(None))
    monkeypatch.setattr(catalog, "require_nix_era", stubs.returning(None))
    monkeypatch.setattr(catalog, "preflight_device", stubs.returning(None))
    monkeypatch.setattr(catalog, "require_fresh_firmware_staging", stubs.returning(None))
    monkeypatch.setattr(catalog, "ensure_nix_cli", stubs.recording(events, "ensure cli"))
    monkeypatch.setattr(catalog, "resolve_packages", stubs.recording(events, "resolve"))
    monkeypatch.setattr(catalog, "build_packages", stubs.recording(events, "build"))

    def versions(_dev: object, image: Image, cycle: catalog.FirmwareCycle) -> None:
        state.cycle = cycle
        cycle.running_version = parse_bos_version("2025-06-15-0-acde0123-25.06")
        cycle.image_version = parse_bos_version(image.version)

    monkeypatch.setattr(catalog, "preflight_versions", versions)

    def manifest_packages(_dev: object) -> dict[str, dict[str, str]]:
        return {"core": {"store_path": state.core_store_path}}

    monkeypatch.setattr(catalog, "read_manifest_packages", manifest_packages)

    def snapshot_upgrade(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.servers_snapshot = catalog.FileSnapshot(catalog.SERVERS_JSON, None)
        cycle.nix_conf_snapshot = catalog.FileSnapshot(catalog._NIX_CONF, None)
        events.append("snapshot config")

    def snapshot_keys(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.opkg_keys_snapshot = catalog.DirSnapshot("/etc/opkg/keys", None)
        events.append("snapshot keys")

    def snapshot_bos(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.bos_version_snapshot = catalog.FileSnapshot(catalog._BOS_VERSION, None)
        events.append("snapshot bos_version")

    monkeypatch.setattr(catalog, "snapshot_upgrade_config", snapshot_upgrade)
    monkeypatch.setattr(catalog, "snapshot_opkg_keys", snapshot_keys)
    monkeypatch.setattr(catalog, "snapshot_bos_version", snapshot_bos)
    monkeypatch.setattr(catalog, "ensure_anchor_version", stubs.recording(events, "anchor"))
    monkeypatch.setattr(catalog, "upload_firmware", stubs.recording(events, "upload"))
    monkeypatch.setattr(catalog, "trust_image_keys", stubs.recording(events, "trust"))

    def remove(_dev: object, _image: Image, cycle: catalog.FirmwareCycle) -> None:
        events.append("remove upload")
        cycle.upload_present = False

    monkeypatch.setattr(catalog, "remove_uploaded_image", remove)

    def start_server(
        _dev: object,
        _plan: object,
        cycle: catalog.UpgradeCycle,
        *,
        firmwares: list[str],
    ) -> None:
        image, running = state.run_cycle.image_version, state.run_cycle.running_version
        assert image is not None and running is not None
        assert image.canonical != running.canonical
        state.firmwares = firmwares
        cycle.server = _StartedProcess()
        events.append("start host")

    monkeypatch.setattr(catalog, "start_upgrade_server", start_server)
    monkeypatch.setattr(catalog, "register_upgrade_server", stubs.recording(events, "register"))
    monkeypatch.setattr(
        catalog,
        "require_exclusive_package_server",
        stubs.recording(events, "restrict servers"),
    )
    monkeypatch.setattr(catalog, "stop_upgrade_server_group", stubs.recording(events, "stop host"))

    def login(_dev: object, cycle: catalog.GrpcSession) -> None:
        cycle.cookie = "session_id=test"
        events.append("login")

    monkeypatch.setattr(catalog, "grpc_login", login)
    monkeypatch.setattr(catalog, "require_auto_upgrade_disabled", stubs.returning(None))

    def snapshot_identity(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.device_identity = "aa:bb:cc:dd:ee:ff"

    def pin_address(dev: Device, cycle: catalog.FirmwareCycle) -> None:
        cycle.pinned_host = dev.host

    monkeypatch.setattr(catalog, "snapshot_device_identity", snapshot_identity)
    monkeypatch.setattr(catalog, "pin_device_address", pin_address)
    monkeypatch.setattr(catalog, "verify_device_identity", stubs.returning(None))

    def snapshot_service(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.init_script_snapshot = catalog.FileSnapshot(catalog._INIT_SCRIPT, None)
        events.append("snapshot service script")

    monkeypatch.setattr(catalog, "snapshot_service_script", snapshot_service)
    monkeypatch.setattr(catalog, "point_bmc_at_index", stubs.recording(events, "point"))
    monkeypatch.setattr(catalog, "await_bmc_ready", stubs.recording(events, "await"))

    def check(
        _dev: object,
        _image: Image,
        cycle: catalog.FirmwareCycle,
        _index: object,
        *,
        require_package_progress: bool = True,
    ) -> None:
        assert require_package_progress
        cycle.upgrade_id = "upgrade-1"
        events.append("check")

    monkeypatch.setattr(catalog, "check_for_firmware_upgrade", check)

    def boot_id(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.boot_id_before = "boot-a"
        events.append("boot id")

    monkeypatch.setattr(catalog, "snapshot_boot_id", boot_id)

    def classify(_result: object) -> catalog.StreamOutcome:
        return state.outcome

    monkeypatch.setattr(catalog, "classify_stream", classify)
    monkeypatch.setattr(catalog, "poll_boot_id_change", stubs.returning(True))
    monkeypatch.setattr(
        catalog,
        "read_flashed_version",
        stubs.returning(parse_bos_version("2025-07-01-0-0badc0de-25.07")),
    )
    monkeypatch.setattr(catalog, "verify_stock_service", stubs.recording(events, "verify"))
    monkeypatch.setattr(
        catalog, "restore_after_success", stubs.recording(events, "restore success")
    )
    monkeypatch.setattr(catalog, "quiesce_bmc", stubs.recording(events, "quiesce"))
    monkeypatch.setattr(
        catalog, "restore_servers_config", stubs.recording(events, "restore servers")
    )

    def restore_file(_dev: object, snap: catalog.FileSnapshot) -> None:
        if snap.remote_path == catalog._NIX_CONF:
            events.append("restore nix")
        elif snap.remote_path == catalog._BOS_VERSION:
            events.append("restore bos_version")
        elif snap.remote_path == catalog._INIT_SCRIPT:
            events.append("restore service script")
        else:
            events.append("restore file")

    monkeypatch.setattr(catalog, "restore_remote_file", restore_file)
    monkeypatch.setattr(catalog, "restore_remote_dir", stubs.recording(events, "restore keys"))
    monkeypatch.setattr(catalog, "bmc_log_tail", stubs.returning("bmc-tail"))

    def no_pids(_dev: object) -> list[int]:
        return []

    monkeypatch.setattr(catalog, "scan_bmc_pids", no_pids)

    def run_stream(_dev: object, cycle: catalog.FirmwareCycle) -> catalog.StreamResult:
        cycle.started_upgrade = True
        return catalog.StreamResult(
            [
                {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_VERIFYING"},
                {"packagePhase": "PACKAGE_UPGRADE_PHASE_REALIZING"},
                {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_APPLYING"},
            ],
            1,
            "Unavailable",
            "stream diagnostic",
            "stderr",
        )

    procedure = e2e_grpc_sysupgrade.E2eGrpcSysupgrade(device="deck", image=_image(tmp_path).path)
    remote = DeviceExec(events)
    yield Harness(
        procedure=procedure,
        dev=Device("deck", backend=remote),
        remote=remote,
        backend=_Nix(packages=("core",)),
        server=server,
        state=state,
        events=events,
        snapshot=snapshot,
        stream=run_stream,
    )
    server.release()


def _run(
    harness: Harness,
    *,
    stream: Stream | None = None,
    make_device: Callable[[str], Device] | None = None,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> None:
    harness.procedure.run(
        dev=harness.dev,
        backend=harness.backend,
        make_index_server=stubs.returning(harness.server),
        stream=harness.stream if stream is None else stream,
        make_device=make_device,
        sleep=sleep,
        clock=clock,
    )


def test_matching_staging_marker_aborts_before_device_preparation(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(catalog, "require_fresh_firmware_staging", _require_fresh_firmware_staging)
    harness.remote.replies[catalog._NIX_PREPARED_MARKER] = Image(harness.procedure.image).version
    with pytest.raises(Abort, match="reboot the device"):
        _run(harness)
    assert not harness.state.run_cycle.mutation_started
    assert not harness.state.run_cycle.started_upgrade
    assert "snapshot config" not in harness.events
    assert not harness.snapshot.exists()


def test_empty_package_offer_restores_configuration_without_starting_upgrade(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    image = Image(harness.procedure.image)
    monkeypatch.setattr(catalog, "check_for_firmware_upgrade", _check_for_firmware_upgrade)

    def grpcurl(*_args: object, **_kwargs: object) -> dict[str, object]:
        return {
            "upgradeId": "firmware-only",
            "firmware": {
                "version": image.version,
                "hash": image.sha256,
                "fileSizeBytes": str(image.size),
            },
            "disruption": "UPGRADE_DISRUPTION_REBOOT",
        }

    monkeypatch.setattr(catalog, "_grpcurl", grpcurl)
    with pytest.raises(Abort, match="no package changes"):
        _run(harness)
    assert not harness.state.run_cycle.started_upgrade
    assert "restore bos_version" in harness.events
    assert "restore service script" in harness.events
    assert "restore servers" in harness.events
    assert harness.server.stopped
    assert not harness.snapshot.exists()


def test_allow_empty_plan_runs_firmware_only_upgrade(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    image = Image(harness.procedure.image)
    harness.procedure.allow_empty_plan = True
    monkeypatch.setattr(catalog, "check_for_firmware_upgrade", _check_for_firmware_upgrade)

    def grpcurl(*_args: object, **_kwargs: object) -> dict[str, object]:
        return {
            "upgradeId": "firmware-only",
            "firmware": {
                "version": image.version,
                "hash": image.sha256,
                "fileSizeBytes": str(image.size),
            },
            "disruption": "UPGRADE_DISRUPTION_REBOOT",
        }

    def staging_gate(*_args: object) -> NoReturn:
        pytest.fail("staging gate must be disabled")

    monkeypatch.setattr(catalog, "_grpcurl", grpcurl)
    monkeypatch.setattr(catalog, "require_fresh_firmware_staging", staging_gate)

    def stream(_dev: object, cycle: catalog.FirmwareCycle) -> catalog.StreamResult:
        cycle.started_upgrade = True
        return catalog.StreamResult(
            [
                {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_VERIFYING"},
                {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_APPLYING"},
            ],
            0,
            None,
            None,
            "",
        )

    _run(harness, stream=stream)
    assert harness.state.run_cycle.started_upgrade
    assert "restore success" in harness.events and "stop host" in harness.events
    assert harness.server.stopped and not harness.snapshot.exists()


def test_registration_failure_restores_in_mandated_order_and_never_streams(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(catalog, "register_upgrade_server", stubs.raising(Abort("register")))
    streamed = False

    def stream(*_args: object) -> catalog.StreamResult:
        nonlocal streamed
        streamed = True
        raise AssertionError

    with pytest.raises(Abort, match="register"):
        _run(harness, stream=stream)
    assert not streamed
    assert harness.events.index("quiesce") < harness.events.index("restore servers")
    assert harness.events.index("restore keys") < harness.events.index("start stock")
    assert "restore bos_version" in harness.events


def test_package_server_start_failure_stops_stored_process(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    def fail_after_start(
        _dev: object, _plan: object, cycle: catalog.UpgradeCycle, *, firmwares: list[str]
    ) -> None:
        cycle.server = _StartedProcess()
        raise Abort("package server startup")

    monkeypatch.setattr(catalog, "start_upgrade_server", fail_after_start)

    with pytest.raises(Abort, match="package server startup"):
        _run(harness)
    assert "stop host" in harness.events


def test_failed_upload_removes_partial_tar(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(catalog, "upload_firmware", stubs.raising(Abort("upload")))
    with pytest.raises(Abort, match="upload"):
        _run(harness)
    assert "remove upload" in harness.events


@pytest.mark.parametrize("stage", ["preflight_device", "snapshot_upgrade_config"])
def test_pre_mutation_failure_touches_no_device(
    harness: Harness, monkeypatch: pytest.MonkeyPatch, stage: str
) -> None:
    monkeypatch.setattr(catalog, stage, stubs.raising(Abort(stage)))
    with pytest.raises(Abort, match=stage):
        _run(harness)
    assert not {"quiesce", "restore servers", "restore nix", "restore keys"} & set(harness.events)
    assert not harness.snapshot.exists()


def test_possibly_accepted_matching_reboot_restores_but_aborts(harness: Harness) -> None:
    harness.state.outcome = catalog.StreamOutcome.POSSIBLY_ACCEPTED
    with pytest.raises(Abort, match="stream diagnostic"):
        _run(harness)
    assert "restore success" in harness.events
    assert "stop host" in harness.events
    assert not harness.snapshot.exists()


def test_spawn_failure_uses_immediate_restore(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    def poll(*_args: object, **_kwargs: object) -> NoReturn:
        pytest.fail("boot resolver called")

    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)

    def stream(_dev: object, cycle: catalog.FirmwareCycle) -> catalog.StreamResult:
        cycle.started_upgrade = False
        raise OSError("spawn")

    with pytest.raises(OSError, match="spawn"):
        _run(harness, stream=stream)
    assert "quiesce" in harness.events


def test_restore_failure_retains_hosts_and_snapshot_and_reports_both(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(catalog, "register_upgrade_server", stubs.raising(Abort("original")))
    monkeypatch.setattr(
        catalog,
        "restore_servers_config",
        stubs.raising(Abort("push failed")),
    )
    with pytest.raises(Abort) as error:
        _run(harness)
    assert "original" in error.value.hint and "push failed" in error.value.hint
    assert "start stock" not in harness.events and "stop host" not in harness.events
    assert harness.snapshot.exists()


def test_pre_verifying_error_includes_bmc_log(harness: Harness) -> None:
    harness.state.outcome = catalog.StreamOutcome.POSSIBLY_ACCEPTED

    def stream(_dev: object, cycle: catalog.FirmwareCycle) -> catalog.StreamResult:
        cycle.started_upgrade = True
        return catalog.StreamResult([], 1, "Unavailable", None, "")

    with pytest.raises(Abort) as error:
        _run(harness, stream=stream)
    assert "bmc-tail" in error.value.hint


def test_provisional_success_matching_reboot_succeeds(harness: Harness) -> None:
    _run(harness)
    assert "restore success" in harness.events
    assert "restore bos_version" not in harness.events
    assert not harness.snapshot.exists()
    ordered = ("ensure cli", "resolve", "snapshot bos_version", "anchor", "upload")
    indices = [harness.events.index(event) for event in ordered]
    assert indices == sorted(indices)


def test_index_anchor_entry_uses_rewritten_running_version(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    def versions(_dev: object, image: Image, cycle: catalog.FirmwareCycle) -> None:
        harness.state.cycle = cycle
        cycle.running_version = parse_bos_version("2025-06-20-0-acde0123-25.07")
        cycle.image_version = parse_bos_version(image.version)

    def anchor(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        assert cycle.running_version is not None and cycle.image_version is not None
        cycle.running_version = catalog.anchored_version(cycle.running_version, cycle.image_version)

    monkeypatch.setattr(catalog, "preflight_versions", versions)
    monkeypatch.setattr(catalog, "ensure_anchor_version", anchor)
    _run(harness)
    assert harness.server.index_text is not None
    assert "2025-06-20-0-acde0123-25.06" in harness.server.index_text
    assert "2025-06-20-0-acde0123-25.07" not in harness.server.index_text


def test_production_26_09_core_is_fed_the_target_index_under_its_anchored_version(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Its bmc resolves the feed by /etc/bos_version, which the anchor rewrote;
    # without that entry it finds no index and the upgrade check fails.
    def anchor(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        assert cycle.running_version is not None and cycle.image_version is not None
        cycle.running_version = catalog.anchored_version(cycle.running_version, cycle.image_version)

    monkeypatch.setattr(catalog, "ensure_anchor_version", anchor)
    harness.state.core_store_path = e2e_grpc_sysupgrade._PRODUCTION_26_09_CORE
    _run(harness)
    cycle = harness.state.run_cycle
    assert cycle.image_version is not None and cycle.running_version is not None
    assert harness.state.firmwares == [
        cycle.image_version.canonical,
        cycle.running_version.canonical,
    ]


def test_any_other_core_is_fed_the_target_entry_only(harness: Harness) -> None:
    # A running-firmware entry would hide a bmc that wrongly scopes the feed to it.
    _run(harness)
    image = harness.state.run_cycle.image_version
    assert image is not None
    assert harness.state.firmwares == [image.canonical]


def test_resolution_error_retries_until_boot_resolves(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    polls = 0
    reads = 0
    resolutions = 0
    sleeps: list[float] = []
    resolve_started = cast("Callable[..., object]", e2e_grpc_sysupgrade._resolve_started)

    def resolve(*args: object, **kwargs: object) -> object:
        nonlocal resolutions
        resolutions += 1
        return resolve_started(*args, **kwargs)

    def poll(*_args: object, **_kwargs: object) -> bool:
        nonlocal polls
        polls += 1
        return True

    def read(_dev: object) -> object:
        nonlocal reads
        reads += 1
        if reads <= 2:
            raise OSError("version read failed")
        return parse_bos_version("2025-07-01-0-0badc0de-25.07")

    monkeypatch.setattr(e2e_grpc_sysupgrade, "_resolve_started", resolve)
    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)
    monkeypatch.setattr(catalog, "read_flashed_version", read)
    _run(harness, sleep=sleeps.append)
    assert resolutions == 1
    assert polls == 6 and reads == 3
    assert sleeps == [1.0, 1.0]
    assert "restore success" in harness.events


def test_resolution_warning_error_stays_in_boot_resolver(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    reads = 0
    resolutions = 0
    sleeps: list[float] = []
    resolve_started = cast("Callable[..., object]", e2e_grpc_sysupgrade._resolve_started)

    def resolve(*args: object, **kwargs: object) -> object:
        nonlocal resolutions
        resolutions += 1
        return resolve_started(*args, **kwargs)

    def read(_dev: object) -> object:
        nonlocal reads
        reads += 1
        if reads == 1:
            raise OSError("version read failed")
        return parse_bos_version("2025-07-01-0-0badc0de-25.07")

    def warn(_message: str) -> None:
        raise BrokenPipeError("stdout closed")

    monkeypatch.setattr(e2e_grpc_sysupgrade, "_resolve_started", resolve)
    monkeypatch.setattr(catalog, "read_flashed_version", read)
    monkeypatch.setattr("bmc_tui.console.warn", warn)
    _run(harness, sleep=sleeps.append)
    assert resolutions == 1
    assert reads == 2 and sleeps == [1.0]
    assert "restore success" in harness.events


def test_resolver_paces_before_retry_when_inner_sleep_fails(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    polls = 0
    sleeps: list[float] = []
    sleep_failures = 0

    def poll(*_args: object, **_kwargs: object) -> bool:
        nonlocal polls
        polls += 1
        if polls == 1:
            raise OSError("poll transport failed")
        return True

    def sleep(duration: float) -> None:
        nonlocal sleep_failures
        if sleep_failures == 0:
            sleep_failures += 1
            raise RuntimeError("sleep interrupted")
        sleeps.append(duration)

    def read(_dev: object) -> object:
        return parse_bos_version("2025-07-01-0-0badc0de-25.07")

    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)
    monkeypatch.setattr(catalog, "read_flashed_version", read)
    _run(harness, sleep=sleep)
    assert sleeps == [1.0]
    assert "restore success" in harness.events


def test_resolution_error_interrupt_retains_servers_and_reports_recovery(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    harness.state.outcome = catalog.StreamOutcome.POSSIBLY_ACCEPTED
    reports: list[str] = []
    sleeps: list[float] = []

    monkeypatch.setattr(catalog, "poll_boot_id_change", stubs.returning(True))
    monkeypatch.setattr(
        catalog,
        "read_flashed_version",
        stubs.raising(OSError("version read failed")),
    )
    monkeypatch.setattr("bmc_tui.console.warn", reports.append)

    def interrupt(delay: float) -> None:
        sleeps.append(delay)
        raise KeyboardInterrupt

    with pytest.raises(KeyboardInterrupt):
        _run(harness, sleep=interrupt)
    assert sleeps == [1.0]
    assert any(
        "firmware stream possibly accepted" in report
        and "version read failed" in report
        and str(harness.snapshot) in report
        and "4321" in report
        and "interrupting kills the in-process index server" in report
        for report in reports
    )
    assert "stop host" not in harness.events and not harness.server.stopped
    assert harness.snapshot.exists()


def test_finished_event_names_package_upgrade_path(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(catalog, "classify_stream", _classify_stream)

    def stream(_dev: object, cycle: catalog.FirmwareCycle) -> catalog.StreamResult:
        cycle.started_upgrade = True
        return catalog.StreamResult([{"finished": {}}], 0, None, None, "")

    with pytest.raises(Abort) as error:
        _run(harness, stream=stream)
    assert "package-upgrade path" in error.value.hint
    assert "instead of a firmware flash" in error.value.hint
    assert "events=[{'finished': {}}]" in error.value.hint


@pytest.mark.parametrize("status", [None, "Unavailable"])
@pytest.mark.parametrize("package_progress", ["missing", "before_verify", "during_apply"])
def test_firmware_progress_requires_sysupgrade_events_even_after_successful_reboot(
    harness: Harness,
    monkeypatch: pytest.MonkeyPatch,
    status: str | None,
    package_progress: str,
) -> None:
    monkeypatch.setattr(catalog, "classify_stream", _classify_stream)
    events: list[dict[str, Json]] = [
        {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_DOWNLOADING"},
        {"download": {"downloadedBytes": "1"}},
    ]
    package_event: dict[str, Json] = {"packagePhase": "PACKAGE_UPGRADE_PHASE_REALIZING"}
    if package_progress == "before_verify":
        events.append(package_event)
    events.append({"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_VERIFYING"})
    if package_progress == "during_apply":
        events.append(package_event)
    if status is None:
        events.append({"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_APPLYING"})

    def stream(_dev: object, cycle: catalog.FirmwareCycle) -> catalog.StreamResult:
        cycle.started_upgrade = True
        return catalog.StreamResult(events, 0 if status is None else 1, status, None, "")

    if package_progress == "during_apply":
        _run(harness, stream=stream)
    else:
        with pytest.raises(Abort, match=r"package REALIZING.*firmware VERIFYING"):
            _run(harness, stream=stream)
    assert "restore success" in harness.events and "stop host" in harness.events
    assert harness.server.stopped and not harness.snapshot.exists()


def test_provisional_success_without_provenance_cleans_then_aborts(
    harness: Harness,
) -> None:
    harness.server.provenance = False
    with pytest.raises(Abort, match="provenance"):
        _run(harness)
    assert "restore success" in harness.events and "stop host" in harness.events
    assert not harness.snapshot.exists()


@pytest.mark.parametrize("gate", ["unreachable", "changed"])
def test_terminal_failure_without_same_boot_routes_to_boot_resolver(
    harness: Harness, monkeypatch: pytest.MonkeyPatch, gate: str
) -> None:
    harness.state.outcome = catalog.StreamOutcome.TERMINAL_FAILURE
    polls = 0

    def poll(*_args: object, **_kwargs: object) -> bool:
        nonlocal polls
        polls += 1
        return True

    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)
    harness.remote.replies[_BOOT_ID] = OSError() if gate == "unreachable" else "boot-b"
    with pytest.raises(Abort, match="stream diagnostic"):
        _run(harness)
    assert polls == 2


def test_provisional_expiry_keeps_serving_until_operator_interrupt(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    polls = 0

    def poll(*_args: object, **_kwargs: object) -> bool:
        nonlocal polls
        polls += 1
        if polls > 1:
            raise KeyboardInterrupt
        return False

    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)
    with pytest.raises(KeyboardInterrupt):
        _run(harness)
    assert not harness.server.stopped and "stop host" not in harness.events
    assert harness.snapshot.exists()


def test_wrong_version_reboot_restores_then_aborts(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        catalog,
        "read_flashed_version",
        stubs.returning(parse_bos_version("2025-08-01-0-deadbeef-25.08")),
    )
    with pytest.raises(Abort, match=r"25\.08"):
        _run(harness)
    assert "restore success" in harness.events
    assert "restore bos_version" not in harness.events
    assert "restore keys" not in harness.events
    assert "restore service script" not in harness.events


def test_post_reboot_operations_use_new_numeric_pin(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    pins = iter(["192.0.2.10", "192.0.2.11"])
    polls: list[str] = []
    versions: list[str] = []
    restores: list[str] = []

    def pin(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.pinned_host = next(pins)

    def poll(dev: Device, *_args: object, **_kwargs: object) -> bool:
        polls.append(dev.host)
        return True

    def read_version(dev: Device) -> object:
        versions.append(dev.host)
        return parse_bos_version("2025-07-01-0-0badc0de-25.07")

    monkeypatch.setattr(catalog, "pin_device_address", pin)
    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)

    def verify(dev: Device) -> None:
        restores.append(f"verify:{dev.host}")

    def restore(dev: Device, _cycle: object) -> None:
        restores.append(dev.host)

    def make_device(host: str) -> Device:
        return fake_device(harness.events, host)

    monkeypatch.setattr(catalog, "read_flashed_version", read_version)
    monkeypatch.setattr(catalog, "verify_stock_service", verify)
    monkeypatch.setattr(catalog, "restore_after_success", restore)
    _run(harness, make_device=make_device)
    assert polls == ["deck", "192.0.2.11"]
    assert versions == ["192.0.2.11"]
    assert restores == ["verify:192.0.2.11", "192.0.2.11"]


def test_same_boot_cleanup_uses_original_numeric_pin(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    harness.state.outcome = catalog.StreamOutcome.TERMINAL_FAILURE
    restored: list[str] = []

    def pin(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.pinned_host = "192.0.2.10"

    def restore(dev: Device, _cycle: object) -> None:
        restored.append(dev.host)

    def make_device(host: str) -> Device:
        return fake_device(harness.events, host)

    monkeypatch.setattr(catalog, "pin_device_address", pin)
    monkeypatch.setattr(catalog, "restore_servers_config", restore)
    with pytest.raises(Abort, match="stream diagnostic"):
        _run(harness, make_device=make_device)
    assert restored == ["192.0.2.10"]


@pytest.mark.parametrize(
    "version",
    ["2025-07-01-0-0badc0de-25.07", "2025-08-01-0-deadbeef-25.08"],
    ids=["matching-version", "mismatched-version"],
)
def test_rebound_hostname_retains_without_mutating_other_device(
    harness: Harness,
    monkeypatch: pytest.MonkeyPatch,
    version: str,
) -> None:
    checks = 0
    version_reads = 0

    def verify(_dev: object, _cycle: object) -> None:
        nonlocal checks
        checks += 1
        if checks > 1:
            raise catalog.DeviceIdentityError("device identity changed")

    def read_version(_dev: object) -> object:
        nonlocal version_reads
        version_reads += 1
        return parse_bos_version(version)

    monkeypatch.setattr(catalog, "verify_device_identity", verify)
    monkeypatch.setattr(catalog, "read_flashed_version", read_version)
    with pytest.raises(catalog.DeviceIdentityError, match="device identity changed"):
        _run(harness)
    assert version_reads == 0
    assert not {
        "quiesce",
        "restore servers",
        "restore nix",
        "restore keys",
        "restore service script",
        "restore success",
        "start stock",
        "stop host",
    } & set(harness.events)
    assert harness.snapshot.exists()


@pytest.mark.parametrize(
    ("active", "times", "expected"),
    [
        ([1, 1, 0], [0.0, 0.0, 1.0], "transfers quiesced"),
        ([1], [0.0, 120.0], "grace expired"),
    ],
    ids=["transfer-finishes", "grace-expires"],
)
def test_identity_mismatch_waits_for_transfer_quiescence_or_grace(
    monkeypatch: pytest.MonkeyPatch,
    active: list[int],
    times: list[float],
    expected: str,
) -> None:
    reports: list[str] = []
    sleeps: list[float] = []

    class ActiveServer:
        def active_transfers(self) -> int:
            return active.pop(0) if len(active) > 1 else active[0]

    server = ActiveServer()
    clock = iter(times)
    monkeypatch.setattr("bmc_tui.console.warn", reports.append)
    resolution = e2e_grpc_sysupgrade._retain_after_transfer_grace(
        catalog.DeviceIdentityError("device identity changed"),
        server,
        sleep=sleeps.append,
        clock=lambda: next(clock),
    )
    assert resolution.retain is True
    assert expected in reports[-1]
    assert all(delay <= 1.0 for delay in sleeps)


def test_missing_image_does_not_allocate_snapshot_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    snapshot = tmp_path / "snapshot"

    def allocate(**_kwargs: object) -> str:
        snapshot.mkdir()
        return str(snapshot)

    monkeypatch.setattr("tempfile.mkdtemp", allocate)
    procedure = e2e_grpc_sysupgrade.E2eGrpcSysupgrade(device="deck", image=tmp_path / "missing.tar")
    with pytest.raises(FileNotFoundError):
        procedure.run(dev=Device("deck", backend=_NoSsh()))
    assert not snapshot.exists()


def test_terminal_failure_same_boot_restores_and_aborts(harness: Harness) -> None:
    harness.state.outcome = catalog.StreamOutcome.TERMINAL_FAILURE
    with pytest.raises(Abort, match="stream diagnostic"):
        _run(harness)
    assert harness.events.index("quiesce") < harness.events.index("restore servers")
    assert "restore bos_version" in harness.events
    assert "stop host" in harness.events


def test_failure_restore_reinstates_service_script(harness: Harness) -> None:
    harness.state.outcome = catalog.StreamOutcome.TERMINAL_FAILURE
    with pytest.raises(Abort):
        _run(harness)
    assert "quiesce" in harness.events
    assert "restore service script" in harness.events
    assert harness.events.index("restore service script") < harness.events.index("start stock")


def test_rejected_uses_boot_resolver(harness: Harness, monkeypatch: pytest.MonkeyPatch) -> None:
    harness.state.outcome = catalog.StreamOutcome.REJECTED
    polls = 0

    def poll(*_args: object, **_kwargs: object) -> bool:
        nonlocal polls
        polls += 1
        return True

    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)
    with pytest.raises(Abort, match="stream diagnostic"):
        _run(harness)
    assert polls == 2


def test_possibly_accepted_expiry_repeats_recovery_and_retains_state(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    harness.state.outcome = catalog.StreamOutcome.POSSIBLY_ACCEPTED
    polls = 0
    reports: list[str] = []

    def poll(*_args: object, **_kwargs: object) -> bool:
        nonlocal polls
        polls += 1
        if polls > 2:
            raise KeyboardInterrupt
        return False

    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)
    monkeypatch.setattr("bmc_tui.console.warn", reports.append)
    with pytest.raises(KeyboardInterrupt):
        _run(harness)
    assert polls > 1 and harness.snapshot.exists() and not harness.server.stopped
    assert any(str(harness.snapshot) in report for report in reports)


def test_boot_resolution_deadline_expiry_retains_state(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    harness.state.outcome = catalog.StreamOutcome.POSSIBLY_ACCEPTED
    polls = 0
    reports: list[str] = []
    now = 0.0

    def poll(*_args: object, **_kwargs: object) -> bool:
        nonlocal polls, now
        polls += 1
        now += 301.0
        return False

    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)
    monkeypatch.setattr("bmc_tui.console.warn", reports.append)
    with pytest.raises(Abort, match="boot id did not change"):
        _run(harness, clock=lambda: now)
    assert polls == 2
    assert harness.snapshot.exists() and not harness.server.stopped


def test_boot_resolution_deadline_defers_to_active_transfer(
    harness: Harness, monkeypatch: pytest.MonkeyPatch
) -> None:
    harness.state.outcome = catalog.StreamOutcome.POSSIBLY_ACCEPTED
    harness.server.transfers = 1
    polls = 0
    now = 0.0

    def poll(*_args: object, **_kwargs: object) -> bool:
        nonlocal polls, now
        polls += 1
        now += 301.0
        if polls == 4:
            harness.server.transfers = 0
        return False

    def advance(seconds: float) -> None:
        nonlocal now
        now += seconds

    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)
    monkeypatch.setattr("bmc_tui.console.warn", stubs.returning(None))
    with pytest.raises(Abort, match="boot id did not change"):
        _run(harness, sleep=advance, clock=lambda: now)
    assert polls == 4, "expiry must wait for the in-flight download to quiesce"
    assert harness.snapshot.exists() and not harness.server.stopped


class _Process:
    pid = 4321

    def __init__(self) -> None:
        self.waits = 0

    def poll(self) -> None:
        return None

    def wait(self, timeout: float | None = None) -> int:
        self.waits += 1
        if self.waits == 1:
            raise subprocess.TimeoutExpired("server", cast("float", timeout))
        return 0

    def terminate(self) -> None:
        raise AssertionError("BUG: the group shutdown signals the group, not the leader")

    def kill(self) -> None:
        raise AssertionError("BUG: the group shutdown signals the group, not the leader")


class _StartedProcess:
    """A server handle only ever read for its pid: the harness stubs every shutdown."""

    pid: int = 4321

    def poll(self) -> int | None:
        raise AssertionError("BUG: the harness stubs the server shutdown")

    def wait(self, timeout: float | None = None) -> int:
        raise AssertionError(f"BUG: the harness stubs the server shutdown (timeout {timeout})")

    def terminate(self) -> None:
        raise AssertionError("BUG: the harness stubs the server shutdown")

    def kill(self) -> None:
        raise AssertionError("BUG: the harness stubs the server shutdown")


class _Socket:
    def __init__(self, checked: list[int]) -> None:
        self.checked = checked

    def __enter__(self) -> "_Socket":
        return self

    def __exit__(self, *_exc: object) -> None:
        pass

    def setsockopt(self, *_args: object) -> None:
        pass

    def bind(self, address: tuple[str, int]) -> None:
        self.checked.append(address[1])


def test_package_server_group_shutdown_escalates_and_verifies_ports(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    process = _Process()
    cycle = catalog.UpgradeCycle(
        "",
        8080,
        8081,
        tmp_path,
        host="127.0.0.1",
        server=process,
    )
    signals: list[tuple[int, signal.Signals]] = []
    checked: list[int] = []

    def killpg(pid: int, sig: signal.Signals) -> None:
        signals.append((pid, sig))

    def make_socket(*_args: object) -> _Socket:
        return _Socket(checked)

    monkeypatch.setattr(os, "killpg", killpg)
    monkeypatch.setattr(socket, "socket", make_socket)
    catalog.stop_upgrade_server_group(cycle)
    assert signals == [(4321, signal.SIGTERM), (4321, signal.SIGKILL)]
    assert checked == [8080, 8081]
