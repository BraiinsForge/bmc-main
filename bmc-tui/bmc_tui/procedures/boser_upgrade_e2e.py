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

"""Serve a Boser upgrade rig, drive one upgrade over Boser's REST API and verify the device."""

import shlex
import subprocess
import tempfile
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from bmc_tui import boser_index, catalog, console
from bmc_tui.bos_version import parse_bos_version
from bmc_tui.boser_rest import BoserRest
from bmc_tui.device import Device
from bmc_tui.fw_index import FwIndexServer
from bmc_tui.image import Image
from bmc_tui.procedures.upgrade_e2e import default_key_dir
from bmc_tui.stage import Abort, best_effort, entrypoint, require

_MAX_TCP_PORT = 65_535
_REBOOT_POLL_SECONDS = 5.0


@dataclass
class BoserUpgradeE2e:
    device: str  # address of the device's Boser web API
    package_index: Path  # complete package index; store paths must be realized locally
    running_version: str  # exact full BOS version reported by the device
    serve_ip: str  # this host's address reachable from the device
    ssh: str  # root SSH address of the same device, for the verification after the run
    image: Path | None = None  # omit to test packages without a firmware offer
    packages: list[str] = field(default_factory=list)  # install on top of the upgrade
    password: str = ""  # Boser root password
    port: int = 8080  # signed Nix cache
    index_port: int = 8081  # package index
    firmware_port: int = 8082  # BOS firmware index and tarball
    stream_deadline: float = 900.0  # seconds to reach a terminal state after start
    reboot_deadline: float = 600.0  # seconds until the new firmware serves Boser again

    def run(
        self,
        *,
        ask: Callable[[str], str] = input,
        clock: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], None] = time.sleep,
        make_device: Callable[[str], Device] = Device,
    ) -> None:
        ports = [self.port, self.index_port, self.firmware_port]
        require(
            len(set(ports)) == len(ports) and all(0 < port <= _MAX_TCP_PORT for port in ports),
            "choose three distinct TCP ports between 1 and 65535",
        )
        boser_index.validate_package_index(self.package_index)
        running = parse_bos_version(self.running_version)
        image = Image(self.image.resolve()) if self.image is not None else None
        base_url = f"http://{self.serve_ip}:{self.firmware_port}"
        document = boser_index.index_document(running=running, base_url=base_url, image=image)

        scratch = Path(".tmp/boser-e2e")
        scratch.mkdir(parents=True, exist_ok=True)
        workdir = Path(tempfile.mkdtemp(dir=scratch)).resolve()
        serve_root = workdir / "firmware"
        serve_root.mkdir()
        (serve_root / boser_index.INDEX_NAME).write_text(document)
        if image is not None:
            (serve_root / "firmware.tar").symlink_to(image.path)
        cycle = catalog.UpgradeCycle(
            password="",
            port=self.port,
            index_port=self.index_port,
            key_dir=default_key_dir(),
            host=self.serve_ip,
            log_path=workdir / "packages.log",
        )
        argv = catalog.upgrade_server_argv(
            host=self.serve_ip,
            port=self.port,
            index_port=self.index_port,
            key_dir=cycle.key_dir,
            built=[],
            firmware=(
                parse_bos_version(image.version).canonical
                if image is not None
                else running.canonical
            ),
        )
        argv += ["--base-index", str(self.package_index.resolve())]
        failed = False
        console.kv("rig artifacts", str(workdir))
        try:
            with FwIndexServer(serve_root, port=self.firmware_port):
                catalog.launch_upgrade_server(cycle, argv)
                instructions = operator_instructions(cycle, base_url)
                (workdir / "operator.txt").write_text(instructions)
                console.code(instructions, "sh")
                prompt = "Type ready once Boser serves with the printed registration: "
                while ask(prompt).strip() != "ready":
                    console.warn("The device is driven only after you type ready.")
                client = BoserRest(f"http://{self.device}", self.password)
                self._drive(client, make_device(self.ssh), ask=ask, clock=clock, sleep=sleep)
                prompt = "Keep serving; type stop only after the device is safe: "
                while ask(prompt).strip() != "stop":
                    console.warn("The servers are still running.")
        except BaseException:
            failed = True
            raise
        finally:
            console.warn("Stopping host servers does not prove a device upgrade has stopped.")
            if failed:
                best_effort(lambda: catalog.stop_upgrade_server_group(cycle))
            else:
                catalog.stop_upgrade_server_group(cycle)

    def _drive(
        self,
        client: BoserRest,
        dev: Device,
        *,
        ask: Callable[[str], str],
        clock: Callable[[], float],
        sleep: Callable[[float], None],
    ) -> None:
        client.login()
        for package in client.installable_packages():
            console.kv("installable", f"{package['name']} {package['version']}")
        offer = client.check(self.packages)
        for line in check_summary(offer):
            console.kv("check", line)
        if offer["offer"] is None:
            console.warn("no upgrade offered; nothing to start")
            return
        offer_id = offer["offer"]["id"]
        if ask(f"Type yes to start offer {offer_id}: ").strip() != "yes":
            console.warn("offer left unstarted")
            return
        before = _Snapshot.take(dev, offer)
        client.start(offer_id)
        console.ok(f"started {offer_id}")
        deadline = clock() + self.stream_deadline
        terminal: dict[str, Any] | None = None
        for event in client.events(deadline=deadline, clock=clock):
            console.kv("state", event_summary(event))
            if event.get("id") not in (None, offer_id):
                raise Abort(f"the stream reports a different execution: {event}")
            if event["state"] == "FAILED":
                raise Abort(f"upgrade failed in {event['phase']}: {event['reason']}")
            terminal = event
        if terminal is None:
            msg = "BUG: the event stream returned without a terminal event"
            raise RuntimeError(msg)
        if terminal["state"] == "REBOOTING":
            deadline = clock() + self.reboot_deadline
            boot_id = _await_reboot(dev, before, deadline=deadline, clock=clock, sleep=sleep)
            _await_boser(client, deadline=deadline, clock=clock, sleep=sleep)
            console.ok(f"the device rebooted and Boser is back; boot id {boot_id}")
        else:
            require(
                offer["firmware"] is None, "a firmware offer must end in REBOOTING, not COMPLETED"
            )
            require(
                catalog.read_boot_id(dev) == before.boot_id,
                "the device rebooted during an upgrade that reported COMPLETED",
            )
        _verify_firmware(dev, offer["firmware"])
        _verify_packages(dev, offer["packages"], before)
        again = client.check(self.packages)
        require(again["offer"] is None, f"the same check still offers {again['offer']}")
        console.ok("the device reached a terminal state and the verification passed")


@dataclass(frozen=True)
class _Snapshot:
    """The device before start: what the run must change and what it must leave alone."""

    boot_id: str
    generation: int | None  # None without a package store: nothing to verify in the profile

    @staticmethod
    def take(dev: Device, offer: dict[str, Any]) -> "_Snapshot":
        capability = offer["package_capability"]
        if capability["status"] == "UNHEALTHY":
            raise Abort(
                f"the package store is unhealthy: {capability['reason']}; repair it before the run"
            )
        ready = capability["status"] == "READY"
        return _Snapshot(
            boot_id=catalog.read_boot_id(dev),
            generation=catalog.current_generation(dev) if ready else None,
        )


def _await_reboot(
    dev: Device,
    before: _Snapshot,
    *,
    deadline: float,
    clock: Callable[[], float],
    sleep: Callable[[float], None],
) -> str:
    while clock() < deadline:
        try:
            boot_id = catalog.read_boot_id(dev)
        except (subprocess.CalledProcessError, OSError):
            pass
        else:
            if boot_id != before.boot_id:
                return boot_id
        sleep(min(_REBOOT_POLL_SECONDS, max(deadline - clock(), 0.0)))
    raise Abort("the device did not come back with a new boot id within --reboot-deadline")


def _await_boser(
    client: BoserRest,
    *,
    deadline: float,
    clock: Callable[[], float],
    sleep: Callable[[float], None],
) -> None:
    """Wait for the rebooted device's Boser, which starts fresh and must report no execution."""
    while True:
        try:
            client.login()
            state = client.state()
        except Abort as error:
            if clock() >= deadline:
                raise Abort(f"Boser did not come back within --reboot-deadline: {error}") from None
            sleep(min(_REBOOT_POLL_SECONDS, max(deadline - clock(), 0.0)))
            continue
        require(state["state"] == "NONE", f"Boser reports an execution after the reboot: {state}")
        return


def _verify_firmware(dev: Device, firmware: dict[str, Any] | None) -> None:
    if firmware is None:
        return
    flashed = catalog.read_flashed_version(dev)
    offered = parse_bos_version(firmware["version"])
    require(
        flashed.canonical == offered.canonical,
        f"/etc/bos_version is {flashed.canonical}, the offer promised {offered.canonical}",
    )


def _verify_packages(dev: Device, plan: dict[str, Any] | None, before: _Snapshot) -> None:
    if before.generation is None:
        require(plan is None, "packages were offered on a device without a package store")
        return
    generation = catalog.current_generation(dev)
    if plan is None:
        require(
            generation == before.generation,
            f"the profile moved to generation {generation} without a package change",
        )
        return
    require(
        generation > before.generation,
        f"expected a newer generation than {before.generation}, the profile points at {generation}",
    )
    installed = catalog.read_manifest_packages(dev)
    for change in plan["changes"]:
        entry = installed.get(change["name"])
        version = entry.get("version") if isinstance(entry, dict) else None
        require(
            version == change["version_to"],
            f"{change['name']}: the manifest has {version or 'nothing'}, "
            f"the offer promised {change['version_to'] or 'removal'}",
        )


def check_summary(body: dict[str, Any]) -> list[str]:
    lines = [f"package capability {body['package_capability']['status']}"]
    firmware = body["firmware"]
    if firmware is not None:
        lines.append(f"firmware {firmware['version']} ({firmware['file_size_bytes']} bytes)")
    plan = body["packages"]
    if plan is not None:
        lines.extend(
            f"package {change['name']} {change['version_from']} -> {change['version_to']}"
            for change in plan["changes"]
        )
        lines.append(f"packages download {plan['download_size_bytes']} bytes")
    offer = body["offer"]
    if offer is None:
        lines.append("no offer")
    else:
        lines.append(f"offer {offer['id']} {offer['kind']} {offer['disruption']}")
    return lines


def event_summary(event: dict[str, Any]) -> str:
    """`STATE`, `STATE/PHASE` or `STATE/PHASE/downloaded_bytes`, mirroring Boser's own tests."""
    key = str(event.get("state", "?"))
    if "phase" in event:
        key += f"/{event['phase']}"
    if "download" in event:
        key += f"/{event['download']['downloaded_bytes']}"
    return key


def operator_instructions(cycle: catalog.UpgradeCycle, firmware_url: str) -> str:
    register = (
        f"{catalog.upgrade_server_registration_command(cycle)} "
        f"--factory-base-url {shlex.quote(cycle.index_url)}"
    )
    return (
        "# Manual device preparation; the host runner executes none of these.\n"
        "# Snapshot /etc/nix-upgrade/servers.json and /etc/nix/nix.conf first.\n"
        "# Pause Boser autoupgrades and BMC maintenance; stop ordinary Boser.\n"
        "# Preserve a recovery path and run only one Boser process.\n"
        f"{register}\n"
        f"BOS_INDEX_URL={shlex.quote(firmware_url)} /path/to/boser-openwrt --log-to-file\n"
        "# Keep Boser alive; an SSH disconnect can terminate a foreground daemon.\n"
        "# Then type ready here: the runner logs in, checks, starts the offer you\n"
        "# confirm, follows the state stream to a terminal state and verifies\n"
        "# the device over SSH; a disagreement with the offer fails the run.\n"
        "# After the run: restore both config snapshots and normal services.\n"
        "# Only then stop the host rig.\n"
    )


@entrypoint
def main(args: BoserUpgradeE2e) -> None:
    args.run()


if __name__ == "__main__":
    main()
