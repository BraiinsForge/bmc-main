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

"""Initialise a clean Deck's nix store from the init tarball.

Bootstraps a device so `deck deploy` can run against it: pushes the
statically built bmc-nix-cli and the init tarball, runs
`bmc-nix-cli init --tarball` — the same code path the firmware
sysupgrade uses — then mounts /nix and activates the first profile
generation.
"""

from dataclasses import dataclass

from bmc_tui import catalog, console, nix
from bmc_tui.device import Device
from bmc_tui.nix import Nix
from bmc_tui.stage import dry_run, entrypoint


@dataclass
class Init:
    device: str  # IP or host of the target Deck
    # Keep in sync with the Init and PrepareDataPartition defaults in bmc-nix/src/bin/cli.rs.
    data_partition: str = "/dev/mmcblk0p4"
    dry_run: bool = False  # run read-only checks; log mutations without executing

    def run(self, dev: Device | None = None, backend: Nix | None = None) -> None:
        if self.dry_run:
            dry_run.set(True)
        dev = dev or Device(self.device)
        backend = backend or nix.real()
        plan = catalog.Provisioning()

        console.header("Initialise device")
        dev.print()

        catalog.ensure_device_reachable(dev)
        catalog.build_init_tarball(backend, plan)
        catalog.build_nix_cli(backend, plan)
        try:
            catalog.push_nix_cli(dev, plan)
            catalog.prepare_data_partition(dev, self.data_partition)
            catalog.ensure_store_absent(dev)
            catalog.push_init_tarball(dev, plan)
            catalog.run_cli_init(dev, plan, self.data_partition)
            catalog.activate_profile(dev, plan)
        finally:
            catalog.cleanup_remote_artifacts(dev, plan)


@entrypoint
def main(args: Init) -> None:
    args.run()


if __name__ == "__main__":
    main()
