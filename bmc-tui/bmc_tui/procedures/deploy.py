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

"""Build and deploy Deck nix packages into a device's bmc profile.

With no --packages, deploys `core`, `bmc-nix-cli` plus every widget (discovered
from the nix-owned `category` metadata), so the wasm host versions stay in
lockstep.
The device must already be initialised (nix-init) — it needs only the store and
its own nix-store binary, not a full nix.
"""

from dataclasses import dataclass, field
from typing import Literal

from bmc_tui import catalog, console, nix
from bmc_tui.device import Device
from bmc_tui.stage import dry_run, entrypoint


@dataclass
class Deploy:
    device: str  # IP or host of the target Deck
    # flake attrs; empty → core, bmc-nix-cli + all widgets
    packages: list[str] = field(default_factory=list)
    profile: Literal["release", "debug"] = "release"  # debug → profiling build (mesh::profile)
    dry_run: bool = False  # build + probe for real; log device mutations without executing
    max_jobs: int | None = None  # nix --max-jobs for the build; None → use nix's own config

    def run(self) -> None:
        if self.dry_run:
            dry_run.set(True)
        dev = Device(self.device)
        backend = nix.real(max_jobs=self.max_jobs)
        plan = catalog.Deployment(
            attrs=[nix.Attr(p) for p in self.packages], prefix=catalog.package_prefix(self.profile)
        )

        console.header("Deploy packages")
        dev.print()

        catalog.ensure_device_reachable(dev)
        catalog.ensure_nix_cli(backend, dev)
        catalog.resolve_packages(backend, plan)
        catalog.build_packages(backend, plan)
        catalog.copy_closures(backend, dev, plan)
        catalog.remove_legacy_flip_clock(dev, plan)
        old_pid = catalog.compositor_pid(dev)
        catalog.register_packages(dev, plan)
        catalog.clear_upgrade_servers(dev)
        catalog.await_package_activation(dev, old_pid=old_pid)


@entrypoint
def main(args: Deploy) -> None:
    args.run()


if __name__ == "__main__":
    main()
