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

"""End-to-end sysupgrade test against a real Deck: the firmware COMMAND's
Nix branches exercised against a fully local package rig.

Scenario A clears the store and flashes image A — COMMAND takes the init
branch, downloading the factory tarball from the rig. Scenario B drops a
preservation marker and flashes image B — COMMAND takes the upgrade
branch, resolving feed → index → rig cache and staging a next generation.
Registration re-runs before each flash: it is idempotent, and keeps the
rig authoritative regardless of what the previous flash left behind.
"""

import tempfile
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from bmc_tui import catalog, console, nix, rig
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.nix import Nix
from bmc_tui.server import default_serve_ip
from bmc_tui.stage import best_effort, dry_run, entrypoint


@dataclass
class E2eSysupgrade:
    device: str  # IP or host of the target Deck
    image_a: Path  # baseline firmware tar; scenario A flashes it over a cleared store
    image_b: Path  # target firmware tar; scenario B upgrades A's store with it
    serve_ip: str | None = None  # device-facing rig address (default: auto-detected)
    serve_port: int = 8083  # rig HTTP port
    scenario: Literal["init", "upgrade", "full"] = "full"
    yes: bool = False  # skip the confirm prompts (cleardown + each flash)
    dry_run: bool = False  # run read-only checks; log mutations without executing

    def run(  # noqa: PLR0915  one ordered scenario owns setup, mutation, and restore
        self,
        dev: Device | None = None,
        backend: Nix | None = None,
        make_device: Callable[[str], Device] = Device,
        make_server: Callable[[Path], rig.RigServer] | None = None,
    ) -> None:
        if self.dry_run:
            dry_run.set(True)
        dev = dev or Device(self.device)
        backend = backend or nix.real()
        state = catalog.E2eRun(image_a=Image(self.image_a), image_b=Image(self.image_b))
        prov = catalog.Provisioning()

        console.header("Sysupgrade e2e")
        dev.print()
        state.image_a.print()
        state.image_b.print()

        catalog.ensure_device_reachable(dev)
        catalog.capture_server_registry(dev, prov)
        catalog.capture_nix_conf(dev, prov)
        catalog.validate_firmware_image(state.image_a, device_target=dev.target)
        catalog.validate_firmware_image(state.image_b, device_target=dev.target)
        catalog.validate_e2e_inputs(state)
        catalog.build_e2e_artifacts(backend, state)
        catalog.build_nix_cli(backend, prov)

        serve_ip = self.serve_ip or default_serve_ip(dev.host)
        server_factory = make_server or (lambda root: rig.RigServer(root, port=self.serve_port))
        # The workdir holds the rig cache and the private signing key;
        # neither may outlive the run.
        with tempfile.TemporaryDirectory(
            prefix="sysupgrade-e2e.", ignore_cleanup_errors=True
        ) as tmp:
            workdir = Path(tmp)
            serve_root = workdir / "serve"
            serve_root.mkdir()
            with server_factory(serve_root) as server:
                base_url = f"http://{serve_ip}:{server.port}"
                catalog.assemble_rig(backend, state, workdir=workdir, base_url=base_url)
                failed = False
                try:
                    catalog.preflight_rig(dev, state)
                    if self.scenario in ("init", "full"):
                        self._scenario_a(dev, prov, state, make_device)
                    if self.scenario in ("upgrade", "full"):
                        self._scenario_b(dev, prov, state, make_device)
                except BaseException:
                    failed = True
                    raise
                finally:
                    if state.device_mutated:
                        # A failure between the cleardown's quiesce and the
                        # reboot leaves the mDNS name dead — clean up through
                        # the pinned numeric handle whenever one exists.
                        cleanup = self._pinned(dev, state, make_device)
                        best_effort(lambda: catalog.cleanup_e2e_marker(cleanup))
                        if failed:
                            best_effort(lambda: catalog.restore_server_registry(cleanup, prov))
                            best_effort(lambda: catalog.restore_nix_conf(cleanup, prov))
                        else:
                            # No primary failure to preserve: leaving the rig
                            # registration behind must fail the run, not
                            # degrade to a log line.
                            catalog.restore_server_registry(cleanup, prov)
                            catalog.restore_nix_conf(cleanup, prov)
                        best_effort(lambda: catalog.sweep_uploaded_images(cleanup, state))
                        best_effort(lambda: catalog.cleanup_remote_artifacts(cleanup, prov))
                        best_effort(lambda: catalog.start_compositor(cleanup))

    def _scenario_a(
        self,
        dev: Device,
        prov: catalog.Provisioning,
        state: catalog.E2eRun,
        make_device: Callable[[str], Device],
    ) -> None:
        console.header("Scenario A — init path")
        state.device_mutated = True
        catalog.stop_compositor(dev)
        catalog.push_nix_cli(dev, prov)
        catalog.register_rig(dev, state)
        catalog.ensure_memory(
            dev, state.image_a.size + state.image_a.rootfs_size + catalog.FLASH_HEADROOM
        )
        catalog.pin_device_address(dev, state)
        pinned = self._pinned(dev, state, make_device)
        catalog.upload_firmware(dev, state.image_a)
        catalog.require_uploaded(pinned, state.image_a)
        catalog.trust_image_keys(pinned, state.image_a)
        catalog.clear_nix_store(pinned, assume_yes=self.yes)
        catalog.flash_e2e(pinned, state.image_a, assume_yes=self.yes)
        catalog.wait_for_device(dev)
        catalog.verify_initialized(dev, state)

    def _scenario_b(
        self,
        dev: Device,
        prov: catalog.Provisioning,
        state: catalog.E2eRun,
        make_device: Callable[[str], Device],
    ) -> None:
        console.header("Scenario B — upgrade path")
        catalog.require_lineage(dev, state)
        catalog.require_initialized_store(dev)
        catalog.ensure_bump_absent(dev, state)
        # Push only after the read-only preconditions — a failed precondition
        # must leave the device untouched. On a full run the reboot into
        # image A cleared /tmp, so scenario B pushes again regardless.
        state.device_mutated = True
        catalog.stop_compositor(dev)
        catalog.push_nix_cli(dev, prov)
        catalog.register_rig(dev, state)
        catalog.drop_e2e_marker(dev)
        catalog.record_generation(dev, state)
        catalog.ensure_memory(
            dev, state.image_b.size + state.image_b.rootfs_size + catalog.FLASH_HEADROOM
        )
        catalog.pin_device_address(dev, state)
        pinned = self._pinned(dev, state, make_device)
        catalog.upload_firmware(dev, state.image_b)
        catalog.require_uploaded(pinned, state.image_b)
        catalog.trust_image_keys(pinned, state.image_b)
        catalog.flash_e2e(pinned, state.image_b, assume_yes=self.yes)
        catalog.wait_for_device(dev)
        catalog.verify_upgraded(dev, state)

    @staticmethod
    def _pinned(dev: Device, state: catalog.E2eRun, make_device: Callable[[str], Device]) -> Device:
        if state.pinned_host is None or state.pinned_host == dev.host:
            return dev
        return make_device(state.pinned_host)


@entrypoint
def main(args: E2eSysupgrade) -> None:
    args.run()


if __name__ == "__main__":
    main()
