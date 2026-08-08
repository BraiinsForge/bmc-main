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
import os
import signal
import subprocess
import tarfile
from pathlib import Path
from types import SimpleNamespace
from typing import TYPE_CHECKING, Any, cast

import pytest

from bmc_tui import catalog
from bmc_tui.bos_version import parse_bos_version
from bmc_tui.image import Image
from bmc_tui.procedures import e2e_grpc_sysupgrade
from bmc_tui.stage import Abort

if TYPE_CHECKING:
    from collections.abc import Callable

    from bmc_tui.device import Device
    from bmc_tui.nix import Nix

_classify_stream = catalog.classify_stream


class _Device:
    target = "stm32mp15/ii3"

    def __init__(self, events: list[str], host: str = "deck") -> None:
        self.events = events
        self.host = host

    def print(self) -> None:
        pass

    def read(self, _command: str) -> str:
        return "boot-a"

    def run(self, command: str) -> str:
        if command == "service bmc-compositor start":
            self.events.append("start stock")
        return ""


class _Nix:
    def list_packages(self) -> list[str]:
        return ["core"]


class _Server:
    def __init__(self, root: Path, *, provenance: bool = True) -> None:
        self.root = root
        self.port = 8082
        self.provenance = provenance
        self.stopped = False
        self.index_text: str | None = None
        self.transfers = 0

    def __enter__(self) -> "_Server":
        index_path = next(self.root.rglob("index.v1.json"), None)
        self.index_text = index_path.read_text() if index_path is not None else None
        return self

    def __exit__(self, *_exc: object) -> None:
        self.stopped = True

    def completed(self, path: str) -> bool:
        return (path == "/firmware.tar" and self.provenance) or path == "/index.v1.json"

    def requests(self) -> list[object]:
        return [SimpleNamespace(path="/firmware.tar", complete=self.provenance)]

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


@pytest.fixture
def harness(  # noqa: PLR0915
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> SimpleNamespace:
    events: list[str] = []
    snapshot = tmp_path / "snapshot"
    snapshot.mkdir()
    server = _Server(snapshot)
    state = SimpleNamespace(cycle=None, outcome=catalog.StreamOutcome.PROVISIONAL_SUCCESS)

    monkeypatch.setattr("tempfile.mkdtemp", lambda **_kwargs: str(snapshot))
    monkeypatch.setattr(
        "bmc_tui.procedures.e2e_grpc_sysupgrade.default_serve_ip",
        lambda _host: "192.0.2.1",
    )
    monkeypatch.setattr(catalog, "ensure_grpcurl", lambda: None)
    monkeypatch.setattr(catalog, "ensure_device_reachable", lambda _dev: None)
    monkeypatch.setattr(catalog, "validate_firmware_image", lambda *_args, **_kwargs: None)
    monkeypatch.setattr(catalog, "require_nix_era", lambda _image: None)
    monkeypatch.setattr(catalog, "preflight_device", lambda _dev: None)
    monkeypatch.setattr(catalog, "ensure_nix_cli", lambda *_args: events.append("ensure cli"))
    monkeypatch.setattr(catalog, "resolve_packages", lambda *_args: events.append("resolve"))
    monkeypatch.setattr(catalog, "build_packages", lambda *_args: events.append("build"))

    def versions(_dev: object, image: Image, cycle: catalog.FirmwareCycle) -> None:
        state.cycle = cycle
        cycle.running_version = parse_bos_version("2025-06-15-0-acde0123-25.06")
        cycle.image_version = parse_bos_version(image.version)

    monkeypatch.setattr(catalog, "preflight_versions", versions)

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
    monkeypatch.setattr(catalog, "ensure_anchor_version", lambda *_args: events.append("anchor"))
    monkeypatch.setattr(catalog, "upload_firmware", lambda *_args: events.append("upload"))
    monkeypatch.setattr(catalog, "trust_image_keys", lambda *_args: events.append("trust"))

    def remove(_dev: object, _image: Image, cycle: catalog.FirmwareCycle) -> None:
        events.append("remove upload")
        cycle.upload_present = False

    monkeypatch.setattr(catalog, "remove_uploaded_image", remove)

    def start_server(_dev: object, _plan: object, cycle: catalog.UpgradeCycle) -> None:
        cycle.server = cast("subprocess.Popen[bytes]", SimpleNamespace(pid=4321))
        events.append("start host")

    monkeypatch.setattr(catalog, "start_upgrade_server", start_server)
    monkeypatch.setattr(
        catalog, "register_upgrade_server", lambda *_args: events.append("register")
    )
    monkeypatch.setattr(
        catalog,
        "require_exclusive_package_server",
        lambda *_args: events.append("restrict servers"),
    )
    monkeypatch.setattr(
        catalog, "stop_upgrade_server_group", lambda *_args: events.append("stop host")
    )

    def login(_dev: object, cycle: catalog.GrpcSession) -> None:
        cycle.cookie = "session_id=test"
        events.append("login")

    monkeypatch.setattr(catalog, "grpc_login", login)
    monkeypatch.setattr(catalog, "require_auto_upgrade_disabled", lambda *_args: None)
    monkeypatch.setattr(
        catalog,
        "snapshot_device_identity",
        lambda _dev, cycle: setattr(cycle, "device_identity", "aa:bb:cc:dd:ee:ff"),
    )
    monkeypatch.setattr(
        catalog,
        "pin_device_address",
        lambda dev, cycle: setattr(cycle, "pinned_host", dev.host),
    )
    monkeypatch.setattr(catalog, "verify_device_identity", lambda *_args: None)

    def snapshot_service(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.init_script_snapshot = catalog.FileSnapshot(catalog._INIT_SCRIPT, None)
        events.append("snapshot service script")

    monkeypatch.setattr(catalog, "snapshot_service_script", snapshot_service)
    monkeypatch.setattr(catalog, "point_bmc_at_index", lambda *_args: events.append("point"))
    monkeypatch.setattr(catalog, "await_bmc_ready", lambda *_args: events.append("await"))

    def check(_dev: object, _image: Image, cycle: catalog.FirmwareCycle, _index: object) -> None:
        cycle.upgrade_id = "upgrade-1"
        events.append("check")

    monkeypatch.setattr(catalog, "check_for_firmware_upgrade", check)

    def boot_id(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.boot_id_before = "boot-a"
        events.append("boot id")

    monkeypatch.setattr(catalog, "snapshot_boot_id", boot_id)
    monkeypatch.setattr(catalog, "classify_stream", lambda _result: state.outcome)
    monkeypatch.setattr(catalog, "poll_boot_id_change", lambda *_args, **_kwargs: True)
    monkeypatch.setattr(
        catalog,
        "read_flashed_version",
        lambda _dev: parse_bos_version("2025-07-01-0-0badc0de-25.07"),
    )
    monkeypatch.setattr(catalog, "verify_stock_service", lambda _dev: events.append("verify"))
    monkeypatch.setattr(
        catalog, "restore_after_success", lambda *_args: events.append("restore success")
    )
    monkeypatch.setattr(catalog, "quiesce_bmc", lambda _dev: events.append("quiesce"))
    monkeypatch.setattr(
        catalog, "restore_servers_config", lambda *_args: events.append("restore servers")
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
    monkeypatch.setattr(catalog, "restore_remote_dir", lambda *_args: events.append("restore keys"))
    monkeypatch.setattr(catalog, "bmc_log_tail", lambda *_args: "bmc-tail")
    monkeypatch.setattr(catalog, "scan_bmc_pids", lambda _dev: [])

    def run_stream(_dev: object, cycle: catalog.FirmwareCycle) -> catalog.StreamResult:
        cycle.started_upgrade = True
        return catalog.StreamResult([], 1, "Unavailable", "stream diagnostic", "stderr")

    procedure = e2e_grpc_sysupgrade.E2eGrpcSysupgrade(device="deck", image=_image(tmp_path).path)
    return SimpleNamespace(
        procedure=procedure,
        dev=_Device(events),
        backend=_Nix(),
        server=server,
        state=state,
        events=events,
        snapshot=snapshot,
        stream=run_stream,
    )


def _run(harness: SimpleNamespace, **kwargs: object) -> None:
    harness.procedure.run(
        dev=harness.dev,
        backend=harness.backend,
        make_index_server=lambda _root: harness.server,
        stream=kwargs.pop("stream", harness.stream),
        **kwargs,
    )


def test_registration_failure_restores_in_mandated_order_and_never_streams(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        catalog, "register_upgrade_server", lambda *_args: (_ for _ in ()).throw(Abort("register"))
    )
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
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    def fail_after_start(_dev: object, _plan: object, cycle: catalog.UpgradeCycle) -> None:
        cycle.server = cast("subprocess.Popen[bytes]", SimpleNamespace(pid=4321))
        raise Abort("package server startup")

    monkeypatch.setattr(catalog, "start_upgrade_server", fail_after_start)

    with pytest.raises(Abort, match="package server startup"):
        _run(harness)
    assert "stop host" in harness.events


def test_failed_upload_removes_partial_tar(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        catalog, "upload_firmware", lambda *_args: (_ for _ in ()).throw(Abort("upload"))
    )
    with pytest.raises(Abort, match="upload"):
        _run(harness)
    assert "remove upload" in harness.events


@pytest.mark.parametrize("stage", ["preflight_device", "snapshot_upgrade_config"])
def test_pre_mutation_failure_touches_no_device(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch, stage: str
) -> None:
    monkeypatch.setattr(catalog, stage, lambda *_args: (_ for _ in ()).throw(Abort(stage)))
    with pytest.raises(Abort, match=stage):
        _run(harness)
    assert not {"quiesce", "restore servers", "restore nix", "restore keys"} & set(harness.events)
    assert not harness.snapshot.exists()


def test_possibly_accepted_matching_reboot_restores_but_aborts(harness: SimpleNamespace) -> None:
    harness.state.outcome = catalog.StreamOutcome.POSSIBLY_ACCEPTED
    with pytest.raises(Abort, match="stream diagnostic"):
        _run(harness)
    assert "restore success" in harness.events
    assert "stop host" in harness.events
    assert not harness.snapshot.exists()


def test_spawn_failure_uses_immediate_restore(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        catalog,
        "poll_boot_id_change",
        lambda *_args, **_kwargs: pytest.fail("boot resolver called"),
    )

    def stream(_dev: object, cycle: catalog.FirmwareCycle) -> catalog.StreamResult:
        cycle.started_upgrade = False
        raise OSError("spawn")

    with pytest.raises(OSError, match="spawn"):
        _run(harness, stream=stream)
    assert "quiesce" in harness.events


def test_restore_failure_retains_hosts_and_snapshot_and_reports_both(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        catalog, "register_upgrade_server", lambda *_args: (_ for _ in ()).throw(Abort("original"))
    )
    monkeypatch.setattr(
        catalog,
        "restore_servers_config",
        lambda *_args: (_ for _ in ()).throw(Abort("push failed")),
    )
    with pytest.raises(Abort) as error:
        _run(harness)
    assert "original" in error.value.hint and "push failed" in error.value.hint
    assert "start stock" not in harness.events and "stop host" not in harness.events
    assert harness.snapshot.exists()


def test_pre_verifying_error_includes_bmc_log(harness: SimpleNamespace) -> None:
    harness.state.outcome = catalog.StreamOutcome.POSSIBLY_ACCEPTED
    with pytest.raises(Abort) as error:
        _run(harness)
    assert "bmc-tail" in error.value.hint


def test_provisional_success_matching_reboot_succeeds(harness: SimpleNamespace) -> None:
    _run(harness)
    assert "restore success" in harness.events
    assert "restore bos_version" not in harness.events
    assert not harness.snapshot.exists()
    ordered = ("ensure cli", "resolve", "snapshot bos_version", "anchor", "upload")
    indices = [harness.events.index(event) for event in ordered]
    assert indices == sorted(indices)


def test_index_anchor_entry_uses_rewritten_running_version(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
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


def test_resolution_error_retries_until_boot_resolves(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    polls = 0
    reads = 0
    resolutions = 0
    sleeps: list[float] = []
    resolve_started = cast("Callable[..., object]", e2e_grpc_sysupgrade._resolve_started)

    def resolve(*args: Any, **kwargs: Any) -> object:
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
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    reads = 0
    resolutions = 0
    sleeps: list[float] = []
    resolve_started = cast("Callable[..., object]", e2e_grpc_sysupgrade._resolve_started)

    def resolve(*args: Any, **kwargs: Any) -> object:
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
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
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
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    harness.state.outcome = catalog.StreamOutcome.POSSIBLY_ACCEPTED
    reports: list[str] = []
    sleeps: list[float] = []

    monkeypatch.setattr(catalog, "poll_boot_id_change", lambda *_args, **_kwargs: True)
    monkeypatch.setattr(
        catalog,
        "read_flashed_version",
        lambda _dev: (_ for _ in ()).throw(OSError("version read failed")),
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
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
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


def test_provisional_success_without_provenance_cleans_then_aborts(
    harness: SimpleNamespace,
) -> None:
    harness.server.provenance = False
    with pytest.raises(Abort, match="provenance"):
        _run(harness)
    assert "restore success" in harness.events and "stop host" in harness.events
    assert not harness.snapshot.exists()


@pytest.mark.parametrize("gate", ["unreachable", "changed"])
def test_terminal_failure_without_same_boot_routes_to_boot_resolver(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch, gate: str
) -> None:
    harness.state.outcome = catalog.StreamOutcome.TERMINAL_FAILURE
    polls = 0

    def poll(*_args: object, **_kwargs: object) -> bool:
        nonlocal polls
        polls += 1
        return True

    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)
    if gate == "unreachable":
        monkeypatch.setattr(harness.dev, "read", lambda _cmd: (_ for _ in ()).throw(OSError()))
    else:
        monkeypatch.setattr(harness.dev, "read", lambda _cmd: "boot-b")
    with pytest.raises(Abort, match="stream diagnostic"):
        _run(harness)
    assert polls == 2


def test_provisional_expiry_keeps_serving_until_operator_interrupt(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
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
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        catalog,
        "read_flashed_version",
        lambda _dev: parse_bos_version("2025-08-01-0-deadbeef-25.08"),
    )
    with pytest.raises(Abort, match=r"25\.08"):
        _run(harness)
    assert "restore success" in harness.events
    assert "restore bos_version" not in harness.events
    assert "restore keys" not in harness.events
    assert "restore service script" not in harness.events


def test_post_reboot_operations_use_new_numeric_pin(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    pins = iter(["192.0.2.10", "192.0.2.11"])
    polls: list[str] = []
    versions: list[str] = []
    restores: list[str] = []

    def pin(_dev: object, cycle: catalog.FirmwareCycle) -> None:
        cycle.pinned_host = next(pins)

    def poll(dev: _Device, *_args: object, **_kwargs: object) -> bool:
        polls.append(dev.host)
        return True

    def read_version(dev: _Device) -> object:
        versions.append(dev.host)
        return parse_bos_version("2025-07-01-0-0badc0de-25.07")

    monkeypatch.setattr(catalog, "pin_device_address", pin)
    monkeypatch.setattr(catalog, "poll_boot_id_change", poll)
    monkeypatch.setattr(catalog, "read_flashed_version", read_version)
    monkeypatch.setattr(
        catalog, "verify_stock_service", lambda dev: restores.append(f"verify:{dev.host}")
    )
    monkeypatch.setattr(
        catalog, "restore_after_success", lambda dev, _cycle: restores.append(dev.host)
    )
    _run(harness, make_device=lambda host: _Device(harness.events, host))
    assert polls == ["deck", "192.0.2.11"]
    assert versions == ["192.0.2.11"]
    assert restores == ["verify:192.0.2.11", "192.0.2.11"]


def test_same_boot_cleanup_uses_original_numeric_pin(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    harness.state.outcome = catalog.StreamOutcome.TERMINAL_FAILURE
    restored: list[str] = []
    monkeypatch.setattr(
        catalog,
        "pin_device_address",
        lambda _dev, cycle: setattr(cycle, "pinned_host", "192.0.2.10"),
    )
    monkeypatch.setattr(
        catalog, "restore_servers_config", lambda dev, _cycle: restored.append(dev.host)
    )
    with pytest.raises(Abort, match="stream diagnostic"):
        _run(harness, make_device=lambda host: _Device(harness.events, host))
    assert restored == ["192.0.2.10"]


@pytest.mark.parametrize(
    "version",
    ["2025-07-01-0-0badc0de-25.07", "2025-08-01-0-deadbeef-25.08"],
    ids=["matching-version", "mismatched-version"],
)
def test_rebound_hostname_retains_without_mutating_other_device(
    harness: SimpleNamespace,
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
        procedure.run(dev=cast("Device", _Device([])), backend=cast("Nix", _Nix()))
    assert not snapshot.exists()


def test_terminal_failure_same_boot_restores_and_aborts(harness: SimpleNamespace) -> None:
    harness.state.outcome = catalog.StreamOutcome.TERMINAL_FAILURE
    with pytest.raises(Abort, match="stream diagnostic"):
        _run(harness)
    assert harness.events.index("quiesce") < harness.events.index("restore servers")
    assert "restore bos_version" in harness.events
    assert "stop host" in harness.events


def test_failure_restore_reinstates_service_script(harness: SimpleNamespace) -> None:
    harness.state.outcome = catalog.StreamOutcome.TERMINAL_FAILURE
    with pytest.raises(Abort):
        _run(harness)
    assert "quiesce" in harness.events
    assert "restore service script" in harness.events
    assert harness.events.index("restore service script") < harness.events.index("start stock")


def test_rejected_uses_boot_resolver(
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
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
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
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
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
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
    harness: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
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
    monkeypatch.setattr("bmc_tui.console.warn", lambda *_args: None)
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
        server=cast("subprocess.Popen[bytes]", process),
    )
    signals: list[tuple[int, signal.Signals]] = []
    checked: list[int] = []
    monkeypatch.setattr(os, "killpg", lambda pid, sig: signals.append((pid, sig)))
    monkeypatch.setattr(catalog.socket, "socket", lambda *_args: _Socket(checked))
    catalog.stop_upgrade_server_group(cycle)
    assert signals == [(4321, signal.SIGTERM), (4321, signal.SIGKILL)]
    assert checked == [8080, 8081]
