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

"""The oldest firmware the BMC application supports on each product."""

import shlex

from rich.markup import escape

from bmc_tui import console
from bmc_tui.bos_version import VersionName, parse_bos_version
from bmc_tui.device import Device
from bmc_tui.stage import Abort, require, stage

PLATFORM_PATH = "/etc/bos_platform"
VERSION_PATH = "/etc/bos_version"

# Keyed by the platform file's content; a platform not listed is not checked.
MINIMUM_FIRMWARE: dict[str, VersionName] = {
    "stm32mp157c-ii3-bmc1": VersionName(year=26, month=7, patch=None),  # BMC100
    "stm32mp157c-ii2-bmm1": VersionName(year=26, month=9, patch=None),  # BMM101
}


def _read_file(dev: Device, path: str) -> str | None:
    """The file's content, or None when it is missing, unreadable or empty."""
    return dev.read(f"cat {shlex.quote(path)} 2>/dev/null || true") or None


@stage("Firmware supported")
def ensure_supported_firmware(dev: Device) -> str:
    platform = _read_file(dev, PLATFORM_PATH)
    if platform is None:
        return "unknown platform, not checked"
    minimum = MINIMUM_FIRMWARE.get(platform)
    if minimum is None:
        return f"{console.lit(platform)} not checked"
    raw = _read_file(dev, VERSION_PATH)
    if raw is None:
        msg = f"cannot read the firmware version: {console.lit(VERSION_PATH)} is missing or empty"
        raise Abort(msg)
    try:
        installed = parse_bos_version(raw).version
    except ValueError as error:
        details = escape(str(error))
        msg = f"cannot parse the firmware version in {console.lit(VERSION_PATH)}: {details}"
        raise Abort(msg) from None
    require(
        installed >= minimum,
        f"firmware {console.lit(installed)} is older than {console.lit(minimum)}, the oldest "
        + "this BMC application supports; upgrade the firmware first, in the device's web UI "
        + "or with `deck sysupgrade` (a BMM101 needs its image passed with `--image`)",
    )
    return console.lit(installed)
