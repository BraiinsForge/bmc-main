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

"""Unit tests for the procedure command-line surfaces."""

from pathlib import Path

import pytest
import tyro

from bmc_tui import cli
from bmc_tui.procedures.deploy import Deploy
from bmc_tui.procedures.e2e_grpc_sysupgrade import E2eGrpcSysupgrade
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
    cmd = tyro.cli(Deploy, args=["--device", "h"])
    assert cmd.packages == []
    assert not hasattr(cmd, "no_restart")


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


def test_e2e_grpc_sysupgrade_parses_args() -> None:
    cmd = tyro.cli(
        E2eGrpcSysupgrade,
        args=["--device", "h", "--image", "firmware.tar"],
    )
    assert cmd.device == "h"
    assert cmd.image == Path("firmware.tar")
    assert cmd.password == ""
    assert cmd.index_port == 8082
    assert cmd.packages_port == 8080
    assert cmd.packages_index_port == 8081
    assert cmd.stream_deadline == 900.0


def test_register_server_is_a_deck_subcommand(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit):
        cli.main(["register-server", "--help"])
    assert "register-server" in capsys.readouterr().out
