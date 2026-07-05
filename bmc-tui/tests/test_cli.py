"""Unit tests for the procedure command-line surfaces."""

from pathlib import Path

import tyro

from bmc_tui.procedures.deploy import Deploy
from bmc_tui.procedures.init import Init
from bmc_tui.procedures.sysupgrade import Sysupgrade
from bmc_tui.procedures.upgrade_e2e import UpgradeE2e


def test_init_parses_args() -> None:
    cmd = tyro.cli(Init, args=["--device", "h"])
    assert cmd.device == "h"
    assert cmd.dry_run is False


def test_deploy_parses_args() -> None:
    cmd = tyro.cli(Deploy, args=["--device", "h", "--packages", ".#deck-packages.core"])
    assert cmd.device == "h"
    assert cmd.packages == [".#deck-packages.core"]


def test_deploy_defaults_to_empty_package_set() -> None:
    assert tyro.cli(Deploy, args=["--device", "h"]).packages == []


def test_upgrade_e2e_parses_args() -> None:
    cmd = tyro.cli(UpgradeE2e, args=["--device", "h"])
    assert cmd.device == "h"
    assert cmd.packages == []
    assert cmd.password == ""
    assert cmd.port == 8080
    assert cmd.index_port == 8081


def test_sysupgrade_parses_args() -> None:
    cmd = tyro.cli(Sysupgrade, args=["--device", "h", "--image", "fw.tar"])
    assert cmd.device == "h"
    assert cmd.image == Path("fw.tar")
    assert cmd.force is False
    assert cmd.yes is False
