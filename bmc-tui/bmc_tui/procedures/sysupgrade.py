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

"""Flash a firmware image to a Deck and verify the result.

The Nix packages are deployed by a separate procedure; run that first when a
firmware bump also needs new widgets (it avoids the post-upgrade /nix unmount).
"""

from dataclasses import dataclass
from pathlib import Path

from bmc_tui import catalog, console
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.stage import best_effort, dry_run, entrypoint, require

# sysupgrade stages the tar in /tmp (tmpfs) and pivots
# to a ramdisk, so it needs RAM beyond the tar: a ~45 MB tar
# was observed needing >70 MB free, hence +20 MB.
FLASH_HEADROOM = 20 * 1024 * 1024


@dataclass
class Sysupgrade:
    device: str  # IP or host of the target Deck
    image: Path  # path to the firmware sysupgrade .tar
    force: bool = False  # pass -F to sysupgrade (override the device's compat check)
    yes: bool = False  # skip the confirm prompt before the irreversible flash
    skip_nix: bool = False  # set BOS_NIX_SKIP=1 to skip Nix staging (already-initialized store)
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

        # Before the headroom check, not only after the flash: a run killed
        # mid-flight never reaches its own cleanup, and its tar is counted
        # against the RAM this one needs.
        best_effort(lambda: catalog.cleanup_firmware(dev, image))
        catalog.ensure_memory(dev, image.size + FLASH_HEADROOM)

        # The upload is inside the cleanup scope: an interrupted transfer
        # or a failed checksum leaves a partial tar in tmpfs, which is exactly
        # what starves the next run's headroom check.
        try:
            catalog.upload_firmware(dev, image)
            catalog.sysupgrade(
                dev, image, force=self.force, assume_yes=self.yes, skip_nix=self.skip_nix
            )
            catalog.wait_for_device(dev)
            catalog.verify_post_upgrade(dev, expect=image.version)
        finally:
            best_effort(lambda: catalog.cleanup_firmware(dev, image))


@entrypoint
def main(args: Sysupgrade) -> None:
    args.run()


if __name__ == "__main__":
    main()
