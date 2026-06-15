"""Flash a firmware image to a Deck and verify the result.

The Nix packages are deployed by a separate procedure; run that first when a
firmware bump also needs new widgets (it avoids the post-upgrade /nix unmount).
"""

from dataclasses import dataclass
from pathlib import Path

from bmc_tui import catalog, console
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.stage import dry_run, entrypoint, require


@dataclass
class Args:
    device: str  # IP or host of the target Deck
    image: Path  # path to the firmware sysupgrade .tar
    force: bool = False  # pass -F to sysupgrade (override the device's compat check)
    dry_run: bool = False  # run read-only checks; log mutations without executing


@entrypoint
def main(args: Args) -> None:
    if args.dry_run:
        dry_run.set(True)
    dev = Device(args.device)
    image = Image(args.image)
    require(image.path.is_file(), f"image not found: {console.lit(image.path)}")

    console.header("Firmware update")
    dev.print()
    image.print()

    catalog.ensure_device_reachable(dev)
    catalog.ensure_nix_conf(dev)
    catalog.validate_firmware_image(image, device_target=dev.target)
    catalog.ensure_free_space(dev, "/mnt/data", image.size)
    catalog.upload_firmware(dev, image)
    catalog.sysupgrade(dev, image, force=args.force)
    catalog.wait_for_device(dev)
    catalog.verify_post_upgrade(dev, expect=image.version)


if __name__ == "__main__":
    main()
