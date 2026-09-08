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

"""Serve an interactive Boser upgrade rig without contacting the device."""

import shlex
import tempfile
from dataclasses import dataclass
from pathlib import Path

from bmc_tui import boser_index, catalog, console
from bmc_tui.bos_version import parse_bos_version
from bmc_tui.fw_index import FwIndexServer
from bmc_tui.image import Image
from bmc_tui.procedures.upgrade_e2e import default_key_dir
from bmc_tui.stage import best_effort, entrypoint, require

_MAX_TCP_PORT = 65_535


@dataclass
class BoserUpgradeE2e:
    package_index: Path  # complete package index; store paths must be realized locally
    running_version: str  # exact full BOS version reported by the device
    serve_ip: str  # this host's address reachable from the device
    image: Path | None = None  # omit to test packages without a firmware offer
    port: int = 8080  # signed Nix cache
    index_port: int = 8081  # package index
    firmware_port: int = 8082  # BOS firmware index and tarball

    def run(self) -> None:
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
                prompt = "Keep serving; type stop only after the device is safe: "
                while input(prompt).strip() != "stop":
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


def operator_instructions(cycle: catalog.UpgradeCycle, firmware_url: str) -> str:
    register = (
        f"{catalog.upgrade_server_registration_command(cycle)} "
        f"--factory-base-url {shlex.quote(cycle.index_url)}"
    )
    return (
        "# Manual device steps; the host runner executes none of these.\n"
        "# Snapshot /etc/nix-upgrade/servers.json and /etc/nix/nix.conf first.\n"
        "# Pause Boser autoupgrades and BMC maintenance; stop ordinary Boser.\n"
        "# Preserve a recovery path and run only one Boser process.\n"
        f"{register}\n"
        f"BOS_INDEX_URL={shlex.quote(firmware_url)} BOSER_UPGRADE_CONSOLE=1 "
        "/path/to/boser-openwrt --log-to-file\n"
        "# In the console: catalog, check [package ...], start <offer-id>, yes.\n"
        "# Watch output automatically reports execution state.\n"
        "# Keep Boser alive; an SSH disconnect can terminate a foreground daemon.\n"
        "# After confirmed completion: restore both config snapshots and normal services.\n"
        "# Only then stop the host rig. This runner never certifies device completion.\n"
    )


@entrypoint
def main(args: BoserUpgradeE2e) -> None:
    args.run()


if __name__ == "__main__":
    main()
