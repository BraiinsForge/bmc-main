"""Unit tests for the procedure command-line surfaces."""

from pathlib import Path

import tyro

from bmc_tui.procedures.deploy import Deploy
from bmc_tui.procedures.sysupgrade import Sysupgrade


def test_deploy_parses_args() -> None:
    cmd = tyro.cli(Deploy, args=["--device", "h", "--packages", ".#deck-packages.core"])
    assert cmd.device == "h"
    assert cmd.packages == [".#deck-packages.core"]


def test_deploy_defaults_to_empty_package_set() -> None:
    assert tyro.cli(Deploy, args=["--device", "h"]).packages == []


def test_sysupgrade_parses_args() -> None:
    cmd = tyro.cli(Sysupgrade, args=["--device", "h", "--image", "fw.tar"])
    assert cmd.device == "h"
    assert cmd.image == Path("fw.tar")
    assert cmd.force is False
    assert cmd.yes is False
