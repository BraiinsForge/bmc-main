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

"""Drive a firmware upgrade through the Deck's production gRPC API.

Re-run recipe: a successful run leaves the device on the image's version;
re-running the same image needs no manual prep — the anchor-ensure stage
rewrites /etc/bos_version below the image's release again.
"""

import shutil
import subprocess
import tempfile
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from bmc_tui import catalog, console, fw_index, nix
from bmc_tui.catalog import FirmwareCycle, StreamResult  # noqa: TC001
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.nix import Nix
from bmc_tui.procedures.upgrade_e2e import default_key_dir
from bmc_tui.server import default_serve_ip
from bmc_tui.stage import Abort, entrypoint

# Match bmc-upgrade/src/firmware.rs DOWNLOAD_IDLE_TIMEOUT.
_RETAIN_TRANSFER_GRACE = 120.0
# A flash + reboot resolves well within one 180 s poll lap; a boot id
# still unchanged after three full laps is not coming.
_BOOT_RESOLVE_DEADLINE = 600.0


@dataclass
class _Cleanup:
    failures: list[str]

    def run(self, label: str, action: Callable[[], object]) -> bool:
        try:
            action()
        except Exception as error:
            self.failures.append(f"{label}: {error}")
            return False
        return True


@dataclass
class _Resolution:
    restore: str
    verdict: BaseException | None
    provenance: bool | None = None
    retain: bool = False
    same_boot: bool = True
    device: Device | None = None


class _TransferTracker(Protocol):
    def active_transfers(self) -> int: ...


def _stream_diagnostic(
    dev: Device, cycle: catalog.FirmwareCycle, result: catalog.StreamResult
) -> str:
    diagnostic = (
        f"events={result.events!r}; status={result.status_code}: {result.status_message}; "
        f"stderr={result.stderr or '(empty)'}"
    )
    if any("finished" in event for event in result.events):
        diagnostic = (
            "device ran the package-upgrade path to completion instead of a firmware flash; "
            f"{diagnostic}"
        )
    verifying = any(
        event.get("firmwarePhase") == "FIRMWARE_UPGRADE_PHASE_VERIFYING" for event in result.events
    )
    if not verifying:
        diagnostic += f"; new bmc log={catalog.bmc_log_tail(dev, cycle) or '(empty)'}"
    return diagnostic


def _outcome_error(outcome: catalog.StreamOutcome, diagnostic: str) -> Abort | None:
    if outcome is catalog.StreamOutcome.PROVISIONAL_SUCCESS:
        return None
    label = {
        catalog.StreamOutcome.POSSIBLY_ACCEPTED: "firmware stream possibly accepted",
        catalog.StreamOutcome.REJECTED: "firmware stream rejected",
        catalog.StreamOutcome.TERMINAL_FAILURE: "firmware stream terminal failure",
    }[outcome]
    return Abort(f"{label}: {diagnostic}")


def _same_boot_is_known(dev: Device, cycle: catalog.FirmwareCycle) -> bool:
    try:
        catalog.verify_device_identity(dev, cycle)
        boot_id = dev.read("cat /proc/sys/kernel/random/boot_id")
        catalog.scan_bmc_pids(dev)
    except Exception:
        return False
    return boot_id == cycle.boot_id_before


def _recovery_hint(cycle: catalog.FirmwareCycle, packages: catalog.UpgradeCycle) -> str:
    server = packages.server
    group_pid = server.pid if server is not None else "unknown"
    return (
        "firmware state remains ambiguous; both host servers are still serving; "
        f"snapshots are retained at {cycle.snapshot_dir}; package-server group pid "
        f"is {group_pid}; interrupting kills the in-process index server"
    )


def _pinned_device(
    dev: Device,
    cycle: catalog.FirmwareCycle,
    make_device: Callable[[str], Device],
) -> Device:
    if cycle.pinned_host is None or cycle.pinned_host == dev.host:
        return dev
    return make_device(cycle.pinned_host)


def _retain_after_transfer_grace(
    error: BaseException,
    index: _TransferTracker,
    *,
    sleep: Callable[[float], None],
    clock: Callable[[], float],
) -> _Resolution:
    console.warn(
        f"{error}; keeping the firmware index server alive for active transfers "
        f"for up to {_RETAIN_TRANSFER_GRACE:g} seconds"
    )
    deadline = clock() + _RETAIN_TRANSFER_GRACE
    while index.active_transfers() > 0:
        remaining = deadline - clock()
        if remaining <= 0:
            console.warn("transfer grace expired; exiting")
            return _Resolution("retain", error, retain=True)
        sleep(min(1.0, remaining))
    console.warn("firmware index transfers quiesced; exiting")
    return _Resolution("retain", error, retain=True)


def _resolve_boot(  # noqa: PLR0912, PLR0913
    dev: Device,
    cycle: catalog.FirmwareCycle,
    *,
    packages: catalog.UpgradeCycle,
    index: fw_index.FwIndexServer,
    outcome: catalog.StreamOutcome,
    verdict: BaseException | None,
    make_device: Callable[[str], Device],
    sleep: Callable[[float], None],
    clock: Callable[[], float],
) -> _Resolution:
    deadline = clock() + _BOOT_RESOLVE_DEADLINE
    while True:
        # An active index transfer holds expiry open — the device is still pulling
        # the image; a dead transfer is released by the bmc-side idle timeout
        # that _RETAIN_TRANSFER_GRACE mirrors, so the wait stays bounded.
        if clock() >= deadline and index.active_transfers() == 0:
            return _retain_after_transfer_grace(
                Abort(
                    f"boot id did not change within {_BOOT_RESOLVE_DEADLINE:g} seconds; "
                    f"{_recovery_hint(cycle, packages)}"
                ),
                index,
                sleep=sleep,
                clock=clock,
            )
        try:
            try:
                changed = catalog.poll_boot_id_change(
                    dev,
                    cycle,
                    timeout=catalog.BOOT_POLL_TIMEOUT,
                    sleep=sleep,
                    clock=clock,
                )
                if changed:
                    catalog.pin_device_address(dev, cycle)
                    reboot_dev = _pinned_device(dev, cycle, make_device)
                    catalog.verify_device_identity(reboot_dev, cycle)
                    confirmed = catalog.poll_boot_id_change(
                        reboot_dev,
                        cycle,
                        timeout=catalog.BOOT_POLL_TIMEOUT,
                        sleep=sleep,
                        clock=clock,
                    )
                    if not confirmed:
                        raise Abort("boot-id change was not confirmed through the pinned address")
                    flashed = catalog.read_flashed_version(reboot_dev)
                    image = cycle.image_version
                    if image is None:
                        msg = "BUG: image version was not resolved before boot proof"
                        raise RuntimeError(msg)
                    if flashed.canonical == image.canonical:
                        provenance = index.completed("/firmware.tar")
                        if outcome is catalog.StreamOutcome.PROVISIONAL_SUCCESS and not provenance:
                            verdict = Abort(
                                "firmware fetch provenance is missing; index requests: "
                                f"{index.requests()!r}"
                            )
                        return _Resolution("success", verdict, provenance, device=reboot_dev)
                    return _Resolution(
                        "failure",
                        Abort(
                            f"device rebooted into {flashed.canonical}, expected {image.canonical}"
                        ),
                        same_boot=False,
                        device=reboot_dev,
                    )

                console.warn(_recovery_hint(cycle, packages))
            except catalog.DeviceIdentityError as identity_error:
                return _retain_after_transfer_grace(
                    identity_error,
                    index,
                    sleep=sleep,
                    clock=clock,
                )
            except KeyboardInterrupt:
                raise
            except BaseException as resolver_error:
                original = (
                    str(verdict) if verdict is not None else "firmware stream provisional success"
                )
                try:
                    console.warn(
                        f"{original}; boot resolution failed: {resolver_error}; "
                        f"{_recovery_hint(cycle, packages)}"
                    )
                except KeyboardInterrupt:
                    raise
                except BaseException:
                    pass
                sleep(1.0)
        except KeyboardInterrupt:
            raise
        except BaseException:
            sleep(1.0)
            continue


def _resolve_started(  # noqa: PLR0913
    dev: Device,
    stable_dev: Device,
    cycle: catalog.FirmwareCycle,
    *,
    packages: catalog.UpgradeCycle,
    index: fw_index.FwIndexServer,
    outcome: catalog.StreamOutcome,
    diagnostic: str,
    make_device: Callable[[str], Device],
    sleep: Callable[[float], None],
    clock: Callable[[], float],
) -> _Resolution:
    verdict = _outcome_error(outcome, diagnostic)
    if outcome is catalog.StreamOutcome.TERMINAL_FAILURE and _same_boot_is_known(stable_dev, cycle):
        return _Resolution("failure", verdict)
    try:
        return _resolve_boot(
            dev,
            cycle,
            packages=packages,
            index=index,
            outcome=outcome,
            verdict=verdict,
            make_device=make_device,
            sleep=sleep,
            clock=clock,
        )
    except KeyboardInterrupt:
        return _Resolution("retain", KeyboardInterrupt(), retain=True)


def _restore_failure(
    cleanup: _Cleanup,
    dev: Device,
    image: Image,
    cycle: catalog.FirmwareCycle,
    *,
    same_boot: bool,
) -> bool:
    if not same_boot:
        return cleanup.run(
            "restore config after changed boot",
            lambda: catalog.restore_after_success(dev, cycle),
        )

    if not cleanup.run("quiesce bmc", lambda: catalog.quiesce_bmc(dev)):
        return False

    def restore_file(name: str, snap: "catalog.FileSnapshot | None") -> None:
        if snap is None:
            cleanup.failures.append(f"{name}: snapshot is missing")
        else:
            cleanup.run(name, lambda: catalog.restore_remote_file(dev, snap))

    before = len(cleanup.failures)
    cleanup.run("restore servers.json", lambda: catalog.restore_servers_config(dev, cycle))
    restore_file("restore nix.conf", cycle.nix_conf_snapshot)
    restore_file("restore bos_version", cycle.bos_version_snapshot)
    opkg_keys = cycle.opkg_keys_snapshot
    if opkg_keys is None:
        cleanup.failures.append("restore opkg keys: snapshot is missing")
    else:
        cleanup.run("restore opkg keys", lambda: catalog.restore_remote_dir(dev, opkg_keys))
    restore_file("restore service script", cycle.init_script_snapshot)
    if cycle.upload_present:
        cleanup.run(
            "remove uploaded firmware",
            lambda: catalog.remove_uploaded_image(dev, image, cycle),
        )
    if len(cleanup.failures) != before:
        return False

    if not cleanup.run("start stock bmc", lambda: dev.run("service bmc-compositor start")):
        return False
    cleanup.run("verify stock bmc", lambda: catalog.verify_stock_service(dev))
    return not cleanup.failures


def _stop_hosts(
    cleanup: _Cleanup,
    packages: catalog.UpgradeCycle,
    index: fw_index.FwIndexServer | None,
    index_started: bool,
) -> bool:
    if packages.server is not None:
        cleanup.run("stop package server", lambda: catalog.stop_upgrade_server_group(packages))
    if index is not None and index_started:
        cleanup.run("stop firmware index", lambda: index.__exit__(None, None, None))
    return not cleanup.failures


def _combined_error(
    verdict: BaseException | None,
    failures: list[str],
    snapshot_dir: Path,
    packages: catalog.UpgradeCycle,
) -> Abort:
    original = str(verdict) if verdict is not None else "firmware upgrade cleanup failed"
    return Abort(
        f"{original}; cleanup failures: {'; '.join(failures)}; "
        f"snapshots retained at {snapshot_dir}; package-server group pid "
        f"{packages.server.pid if packages.server is not None else 'unknown'}"
    )


@dataclass
class E2eGrpcSysupgrade:
    device: str
    image: Path
    password: str = ""
    index_port: int = 8082
    packages_port: int = 8080
    packages_index_port: int = 8081
    stream_deadline: float = 900.0

    def run(  # noqa: PLR0912, PLR0913, PLR0915
        self,
        *,
        dev: Device | None = None,
        backend: Nix | None = None,
        make_index_server: "Callable[[Path], fw_index.FwIndexServer] | None" = None,
        stream: "Callable[[Device, FirmwareCycle], StreamResult] | None" = None,
        make_device: "Callable[[str], Device] | None" = None,
        sleep: Callable[[float], None] = time.sleep,
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        dev = dev or Device(self.device)
        backend = backend or nix.real()
        image = Image(self.image)
        plan = catalog.Deployment(
            attrs=[nix.Attr(name) for name in backend.list_packages()],
            prefix=catalog.package_prefix("release"),
        )
        packages = catalog.UpgradeCycle(
            password=self.password,
            port=self.packages_port,
            index_port=self.packages_index_port,
            key_dir=default_key_dir(),
        )
        console.header("End-to-end gRPC sysupgrade")
        dev.print()
        image.print()

        snapshot_dir = Path(tempfile.mkdtemp(prefix="e2e-firmware."))
        cycle = catalog.FirmwareCycle(
            password=self.password,
            index_port=self.index_port,
            stream_deadline=self.stream_deadline,
            snapshot_dir=snapshot_dir,
        )
        index: fw_index.FwIndexServer | None = None
        index_started = False
        resolution = _Resolution("none", None)
        cleanup = _Cleanup([])
        mutation_dev = dev
        cleanup_dev = dev
        device_factory = make_device or Device

        try:
            catalog.ensure_grpcurl()
            catalog.ensure_device_reachable(dev)
            catalog.ensure_nix_cli(backend, dev)
            catalog.validate_firmware_image(image, device_target=dev.target)
            catalog.require_nix_era(image)
            catalog.preflight_versions(dev, image, cycle)
            catalog.preflight_device(dev)
            catalog.resolve_packages(backend, plan)
            catalog.build_packages(backend, plan)
            catalog.grpc_login(dev, packages)
            catalog.require_auto_upgrade_disabled(dev, packages)
            catalog.snapshot_device_identity(dev, cycle)
            catalog.pin_device_address(dev, cycle)
            mutation_dev = self._pinned(dev, cycle, device_factory)
            cleanup_dev = mutation_dev
            catalog.verify_device_identity(mutation_dev, cycle)
            catalog.snapshot_upgrade_config(mutation_dev, cycle)
            catalog.snapshot_opkg_keys(mutation_dev, cycle)
            catalog.snapshot_bos_version(mutation_dev, cycle)
            catalog.snapshot_service_script(mutation_dev, cycle)
            # No memory-headroom gate here: unlike the SSH path (which flashes
            # with the full UI resident), StartUpgrade is driven by the bmc
            # application, which tears down the widgets — freeing bmc-wasm-host's
            # resident runtime — before it flashes. A pre-flight check measured
            # while the widgets are still up gates on RAM that is no longer
            # committed by flash time.
            cycle.mutation_started = True
            catalog.ensure_anchor_version(mutation_dev, cycle)
            cycle.upload_present = True
            catalog.upload_firmware(mutation_dev, image)
            catalog.trust_image_keys(mutation_dev, image)
            catalog.remove_uploaded_image(mutation_dev, image, cycle)
            catalog.start_upgrade_server(mutation_dev, plan, packages)
            catalog.register_upgrade_server(mutation_dev, packages)
            catalog.require_exclusive_package_server(mutation_dev)

            cycle.host = default_serve_ip(mutation_dev.host)
            serve_root = snapshot_dir / "serve"
            serve_root.mkdir()
            (serve_root / "firmware.tar").symlink_to(image.path.resolve())
            server_factory = make_index_server or (
                lambda root: fw_index.FwIndexServer(root, port=self.index_port)
            )
            index = server_factory(serve_root)
            base_url = f"http://{cycle.host}:{index.port}"
            running = cycle.running_version
            offered = cycle.image_version
            if running is None or offered is None:
                msg = "BUG: versions were not resolved before index construction"
                raise RuntimeError(msg)
            (serve_root / fw_index.INDEX_NAME).write_text(
                fw_index.index_document(
                    running=running,
                    image=offered,
                    anchor_url=base_url,
                    image_url=f"{base_url}/firmware.tar",
                    image_sha256=image.sha256,
                    image_size=image.size,
                )
            )
            index.__enter__()
            index_started = True

            catalog.point_bmc_at_index(mutation_dev, cycle)
            catalog.await_bmc_ready(mutation_dev, cycle)
            catalog.grpc_login(mutation_dev, cycle)
            try:
                catalog.check_for_firmware_upgrade(mutation_dev, image, cycle, index)
            except subprocess.CalledProcessError as error:
                raise Abort(
                    "firmware check RPC failed; a package-check failure points to "
                    f"{catalog.SERVERS_JSON} or the package-server log at "
                    f"{packages.log_path}: {error}"
                ) from None
            catalog.snapshot_boot_id(mutation_dev, cycle)
            result = (stream or catalog.run_firmware_stream)(mutation_dev, cycle)
            outcome = catalog.classify_stream(result)
            resolution = _resolve_started(
                dev,
                mutation_dev,
                cycle,
                packages=packages,
                index=index,
                outcome=outcome,
                diagnostic=_stream_diagnostic(mutation_dev, cycle, result),
                make_device=device_factory,
                sleep=sleep,
                clock=clock,
            )
        except BaseException as error:
            if resolution.retain:
                resolution.verdict = error
            elif cycle.started_upgrade and index is not None:
                resolution = _resolve_started(
                    dev,
                    mutation_dev,
                    cycle,
                    packages=packages,
                    index=index,
                    outcome=catalog.StreamOutcome.POSSIBLY_ACCEPTED,
                    diagnostic=str(error),
                    make_device=device_factory,
                    sleep=sleep,
                    clock=clock,
                )
            else:
                resolution = _Resolution(
                    "failure" if cycle.mutation_started else "none",
                    error,
                )
        finally:
            cleanup_dev = resolution.device or cleanup_dev
            restored = True
            if resolution.restore in {"failure", "success"}:
                try:
                    catalog.verify_device_identity(cleanup_dev, cycle)
                except catalog.DeviceIdentityError as identity_error:
                    if index is None:
                        cleanup.failures.append(
                            f"verify device identity before restore: {identity_error}"
                        )
                    else:
                        resolution = _retain_after_transfer_grace(
                            identity_error,
                            index,
                            sleep=sleep,
                            clock=clock,
                        )
                    restored = False
                except Exception as error:
                    cleanup.failures.append(f"verify device identity before restore: {error}")
                    restored = False
            if restored and resolution.restore == "failure":
                restored = _restore_failure(
                    cleanup,
                    cleanup_dev,
                    image,
                    cycle,
                    same_boot=resolution.same_boot,
                )
            elif restored and resolution.restore == "success":
                restored = cleanup.run(
                    "verify stock bmc after reboot",
                    lambda: catalog.verify_stock_service(cleanup_dev),
                )
                if restored:
                    restored = cleanup.run(
                        "restore config after success",
                        lambda: catalog.restore_after_success(cleanup_dev, cycle),
                    )

            if not resolution.retain and restored:
                hosts_stopped = _stop_hosts(
                    cleanup,
                    packages,
                    index,
                    index_started,
                )
                if hosts_stopped:
                    cleanup.run(
                        "remove snapshot directory",
                        lambda: shutil.rmtree(snapshot_dir),
                    )

        if cleanup.failures:
            raise _combined_error(
                resolution.verdict,
                cleanup.failures,
                snapshot_dir,
                packages,
            )
        if resolution.verdict is not None:
            raise resolution.verdict

    @staticmethod
    def _pinned(
        dev: Device,
        cycle: catalog.FirmwareCycle,
        make_device: Callable[[str], Device],
    ) -> Device:
        return _pinned_device(dev, cycle, make_device)


@entrypoint
def main(args: E2eGrpcSysupgrade) -> None:
    args.run()


if __name__ == "__main__":
    main()
