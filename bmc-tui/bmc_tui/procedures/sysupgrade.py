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

# sysupgrade stages the tar in /tmp (tmpfs) and pivots to a ramdisk, so it needs
# RAM beyond the tar: a ~45 MB tar was observed needing >70 MB free, hence +20 MB.
_FLASH_HEADROOM = 20 * 1024 * 1024


@dataclass
class Sysupgrade:
    device: str  # IP or host of the target Deck
    image: Path  # path to the firmware sysupgrade .tar
    force: bool = False  # pass -F to sysupgrade (override the device's compat check)
    yes: bool = False  # skip the confirm prompt before the irreversible flash
    dry_run: bool = False  # run read-only checks; log mutations without executing

    def run(self) -> None:
        if self.dry_run:
            dry_run.set(True)
        dev = Device(self.device)
        image = Image(self.image)
        require(image.path.is_file(), f"image not found: {console.lit(image.path)}")

        console.header("Firmware update")
        dev.print()
        image.print()

        catalog.ensure_device_reachable(dev)
        catalog.validate_firmware_image(image, device_target=dev.target)
        catalog.ensure_memory(dev, image.size + _FLASH_HEADROOM)
        catalog.upload_firmware(dev, image)
        catalog.sysupgrade(dev, image, force=self.force, assume_yes=self.yes)
        catalog.wait_for_device(dev)
        catalog.verify_post_upgrade(dev, expect=image.version)


@entrypoint
def main(args: Sysupgrade) -> None:
    args.run()


if __name__ == "__main__":
    main()
