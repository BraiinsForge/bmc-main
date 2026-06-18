"""Initialise a clean Deck's nix store from the init tarball.

Bootstraps a device so `deck deploy` can run against it: bind-mounts
/mnt/data/nix at /nix, streams and extracts the init tarball over /, then
activates the first profile generation. The port of the retired
scripts/nix-init.sh.
"""

from dataclasses import dataclass

from bmc_tui import catalog, console, nix
from bmc_tui.device import Device
from bmc_tui.stage import dry_run, entrypoint


@dataclass
class Init:
    device: str  # IP or host of the target Deck
    dry_run: bool = False  # run read-only checks; log mutations without executing

    def run(self) -> None:
        if self.dry_run:
            dry_run.set(True)
        dev = Device(self.device)
        backend = nix.real()
        plan = catalog.Provisioning()

        console.header("Initialise device")
        dev.print()

        catalog.ensure_device_reachable(dev)
        catalog.ensure_store_absent(dev)
        catalog.mount_nix_store(dev)
        catalog.build_init_tarball(backend, plan)
        catalog.stream_init_tarball(dev, plan)
        catalog.activate_profile(dev)


@entrypoint
def main(args: Init) -> None:
    args.run()


if __name__ == "__main__":
    main()
