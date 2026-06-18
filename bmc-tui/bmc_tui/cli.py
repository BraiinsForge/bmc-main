"""Single `deck` entry point: dispatch to a procedure subcommand."""

from bmc_tui.procedures.deploy import Deploy
from bmc_tui.procedures.init import Init
from bmc_tui.procedures.sysupgrade import Sysupgrade
from bmc_tui.stage import entrypoint


@entrypoint
def main(command: Init | Deploy | Sysupgrade) -> None:
    command.run()


if __name__ == "__main__":
    main()
