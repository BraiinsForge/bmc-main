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

"""Unit tests for finding the widget recipes and running a logged step."""

import sys
from pathlib import Path

import pytest
from bmc_tui.stage import Abort

from intl_sweep.__main__ import run_logged, widget_recipes


def recipe(root: Path, relative: str) -> Path:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    _ = path.write_text("", encoding="utf-8")
    return path


def test_every_recipe_is_named_for_its_widget_and_given_whole(tmp_path: Path) -> None:
    clock = recipe(tmp_path, "clock/gallery/clock.capture.toml")
    driver = recipe(tmp_path, "formula-1/gallery/driver.capture.toml")
    miner = recipe(tmp_path, "lib/miner-info/gallery/miner-info.capture.toml")

    assert widget_recipes(tmp_path) == [
        ("clock-clock", clock.resolve()),
        ("formula-1-driver", driver.resolve()),
        ("miner-info-miner-info", miner.resolve()),
    ]


def test_a_recipe_inside_a_build_tree_is_left_out(tmp_path: Path) -> None:
    _ = recipe(tmp_path, "clock/target/gallery/stale.capture.toml")

    assert widget_recipes(tmp_path) == []


def test_a_step_keeps_its_whole_output_and_a_failure_points_at_it(tmp_path: Path) -> None:
    log = tmp_path / "logs" / "step.log"
    failing = [sys.executable, "-c", "print('built'); raise SystemExit(3)"]

    with pytest.raises(Abort, match=r"exit 3"):
        run_logged(failing, log, "a step")

    assert log.read_text(encoding="utf-8") == "built\n"
