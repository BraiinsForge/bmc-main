"""Drive a full package-upgrade cycle against a Deck, end to end.

Builds the requested packages locally, serves them from this machine via
`nix run .#upgrade-server`, registers the server on the device, then drives
CheckForUpgrade → StartUpgrade over gRPC (grpcurl) and asserts the bmc
profile advanced to a new generation. Dev-only harness: the served build
must differ from the installed profile or no upgrade is offered. The
server registration persists on the device after the run.
"""

import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

from bmc_tui import catalog, console, nix
from bmc_tui.device import Device
from bmc_tui.stage import entrypoint


def _default_key_dir() -> Path:
    """The upgrade-server script's own default keypair location."""
    state_home = os.environ.get("XDG_STATE_HOME", "")
    base = Path(state_home) if state_home else Path.home() / ".local" / "state"
    return base / "bmc-upgrade-server"


@dataclass
class UpgradeE2e:
    device: str  # IP or host of the target Deck
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
        plan = catalog.Deployment(attrs=attrs, prefix=catalog.package_prefix(self.profile))
        cycle = catalog.UpgradeCycle(
            password=self.password,
            port=self.port,
            index_port=self.index_port,
            key_dir=_default_key_dir(),
        )

        console.header("End-to-end package upgrade")
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
            catalog.grpc_login(dev, cycle)
            catalog.check_for_upgrade(dev, cycle)
            catalog.run_upgrade(dev, cycle)
            catalog.verify_profile_advanced(dev, plan, cycle)
        finally:
            catalog.stop_upgrade_server(cycle)


@entrypoint
def main(args: UpgradeE2e) -> None:
    args.run()


if __name__ == "__main__":
    main()
