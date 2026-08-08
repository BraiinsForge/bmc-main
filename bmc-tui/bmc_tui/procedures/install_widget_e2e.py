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

"""Install a widget on a Deck end to end via remove-then-reinstall.

The plain upgrade e2e serves the full package set, so nothing is
installable by construction. This procedure creates the gap on purpose:
remove a widget, then discover + install it back over gRPC. Self-cleaning
— the reinstall returns the device to its baseline. Dev-only harness: the
served build must contain the removed widget for the install to be offered.
The server registration persists on the device after the run.
"""

from dataclasses import dataclass, field
from typing import Literal

from bmc_tui import catalog, console, nix
from bmc_tui.device import Device
from bmc_tui.procedures.upgrade_e2e import default_key_dir
from bmc_tui.stage import entrypoint


@dataclass
class InstallWidgetE2e:
    device: str  # IP or host of the target Deck
    widget: str = "widget-blockheight"  # leaf widget to remove then reinstall
    packages: list[str] = field(default_factory=list)  # flake attrs; empty → every deck package
    profile: Literal["release", "debug"] = "release"  # debug → profiling build (mesh::profile)
    password: str = ""  # device web password; empty when none is set
    port: int = 8080  # binary-cache port served on this machine
    index_port: int = 8081  # package-index port served on this machine
    max_jobs: int | None = None  # nix --max-jobs for the build; None → use nix's own config

    def run(self) -> None:
        dev = Device(self.device)
        backend = nix.real(max_jobs=self.max_jobs)
        # Serve the full package set by default: any system-installed package
        # absent from every consulted index aborts the device's upgrade check.
        attrs = self.packages or backend.list_packages()
        plan = catalog.Deployment(
            attrs=[nix.Attr(a) for a in attrs], prefix=catalog.package_prefix(self.profile)
        )
        cycle = catalog.UpgradeCycle(
            password=self.password,
            port=self.port,
            index_port=self.index_port,
            key_dir=default_key_dir(),
        )

        console.header("End-to-end widget install")
        dev.print()

        catalog.ensure_grpcurl()
        catalog.ensure_device_reachable(dev)
        catalog.ensure_nix_cli(backend, dev)
        catalog.resolve_packages(backend, plan)
        catalog.build_packages(backend, plan)
        catalog.snapshot_profile(dev, cycle)
        try:
            catalog.start_upgrade_server(dev, plan, cycle)
            catalog.register_upgrade_server(dev, cycle)
            catalog.require_exclusive_package_server(dev)
            catalog.grpc_login(dev, cycle)
            catalog.remove_package(dev, cycle, self.widget)
            catalog.list_installable_widgets(dev, cycle, self.widget)
            catalog.check_for_install(dev, cycle, self.widget)
            catalog.run_upgrade(dev, cycle)
            catalog.verify_widget_installed(dev, cycle, self.widget)
        finally:
            catalog.stop_upgrade_server(cycle)


@entrypoint
def main(args: InstallWidgetE2e) -> None:
    args.run()


if __name__ == "__main__":
    main()
