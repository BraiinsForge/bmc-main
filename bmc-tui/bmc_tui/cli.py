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

"""Single `deck` entry point: dispatch to a procedure subcommand."""

from bmc_tui.procedures.deploy import Deploy
from bmc_tui.procedures.e2e_sysupgrade import E2eSysupgrade
from bmc_tui.procedures.image_formats import ImageFormats
from bmc_tui.procedures.init import Init
from bmc_tui.procedures.install_widget_e2e import InstallWidgetE2e
from bmc_tui.procedures.sysupgrade import Sysupgrade
from bmc_tui.procedures.upgrade_e2e import UpgradeE2e
from bmc_tui.stage import entrypoint


@entrypoint
def main(
    command: Init
    | Deploy
    | Sysupgrade
    | UpgradeE2e
    | InstallWidgetE2e
    | E2eSysupgrade
    | ImageFormats,
) -> None:
    command.run()


if __name__ == "__main__":
    main()
