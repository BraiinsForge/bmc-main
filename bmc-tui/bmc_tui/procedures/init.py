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
            catalog.prepare_data_partition(dev)
            catalog.ensure_store_absent(dev)
            catalog.push_init_tarball(dev, plan)
            catalog.run_cli_init(dev, plan)
            catalog.activate_profile(dev, plan)
        finally:
            catalog.cleanup_remote_artifacts(dev, plan)


@entrypoint
def main(args: Init) -> None:
    args.run()


if __name__ == "__main__":
    main()
