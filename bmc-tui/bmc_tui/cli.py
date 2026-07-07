"""Single `deck` entry point: dispatch to a procedure subcommand."""

from bmc_tui.procedures.deploy import Deploy
from bmc_tui.procedures.init import Init
from bmc_tui.procedures.install_widget_e2e import InstallWidgetE2e
from bmc_tui.procedures.sysupgrade import Sysupgrade
from bmc_tui.procedures.upgrade_e2e import UpgradeE2e
from bmc_tui.stage import entrypoint


@entrypoint
def main(command: Init | Deploy | Sysupgrade | UpgradeE2e | InstallWidgetE2e) -> None:
    command.run()


if __name__ == "__main__":
    main()
