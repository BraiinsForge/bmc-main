"""Build and deploy Deck nix packages into a device's bmc profile.

With no --packages, deploys `core` plus every widget (discovered from the
nix-owned `category` metadata), so the wasm host versions stay in lockstep.
The device must already be initialised (nix-init) — it needs only the store and
its own nix-store binary, not a full nix.
"""

from dataclasses import dataclass, field

from bmc_tui import catalog, console, nix
from bmc_tui.device import Device
from bmc_tui.stage import dry_run, entrypoint


@dataclass
class Args:
    device: str  # IP or host of the target Deck
    packages: list[str] = field(default_factory=list)  # flake attrs; empty → core + all widgets
    dry_run: bool = False  # build + probe for real; log device mutations without executing


@entrypoint
def main(args: Args) -> None:
    if args.dry_run:
        dry_run.set(True)
    dev = Device(args.device)
    backend = nix.real()
    plan = catalog.Deployment(attrs=args.packages)

    console.header("Deploy packages")
    dev.print()

    catalog.ensure_device_reachable(dev)
    catalog.ensure_nix_cli(backend, dev)
    catalog.resolve_packages(backend, plan)
    catalog.build_packages(backend, plan)
    catalog.copy_closures(backend, dev, plan)
    catalog.register_packages(dev, plan)


if __name__ == "__main__":
    main()
