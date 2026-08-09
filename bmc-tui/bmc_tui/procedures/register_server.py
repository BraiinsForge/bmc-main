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

"""Restore or register upgrade servers on a device after a deploy cleared them."""

from dataclasses import dataclass

from bmc_tui import catalog, console
from bmc_tui.device import Device
from bmc_tui.stage import entrypoint


@dataclass
class RegisterServer:
    device: str  # IP or host of the target Deck
    url: str | None = None  # override the default entry's feed/index URL
    id: str | None = None  # override or select the entry id
    key: str | None = None  # override the entry's index public key

    def run(self) -> None:
        dev = Device(self.device)
        console.header("Register upgrade servers")
        dev.print()
        catalog.ensure_device_reachable(dev)
        catalog.register_default_servers(dev, url=self.url, entry_id=self.id, key=self.key)


@entrypoint
def main(args: RegisterServer) -> None:
    args.run()


if __name__ == "__main__":
    main()
