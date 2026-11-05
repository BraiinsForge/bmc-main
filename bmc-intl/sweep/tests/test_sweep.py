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

"""Unit tests for finding the widget recipes, running a logged step and telling a stage's steps."""

import re
import sys
from collections.abc import Generator, Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import Self

import pytest
from bmc_tui import console
from bmc_tui.stage import Abort
from rich.text import Text
from web_probe.device import Interrupted, SceneLeft

from intl_sweep.__main__ import (
    EXIT_INTERRUPTED,
    _walk_web,
    run_logged,
    steps_of,
    web,
    widget_recipes,
    widgets,
)


class Said:
    """What a stage's steps left in the output."""

    def __init__(self, monkeypatch: pytest.MonkeyPatch) -> None:
        self.ended: list[str] = []
        self.failed: list[str] = []
        self.live: list[str] = []
        monkeypatch.setattr(console, "ok", self.ended.append)
        monkeypatch.setattr(console, "error", self.failed.append)


def test_each_step_stays_behind_with_its_duration(monkeypatch: pytest.MonkeyPatch) -> None:
    said = Said(monkeypatch)

    with steps_of(said.live.append) as steps:
        steps.start("logging in")
        steps.start("walking")

    assert said.live == ["logging in", "walking"]
    assert len(said.ended) == 2
    assert re.fullmatch(r"logging in \(\d+\.\d s\)", Text.from_markup(said.ended[0]).plain)
    assert re.fullmatch(r"walking \(\d+\.\d s\)", Text.from_markup(said.ended[1]).plain)


def test_a_steps_value_is_plain_on_the_spinner_and_styled_in_the_output(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    said = Said(monkeypatch)

    with steps_of(said.live.append) as steps:
        steps.start("switching to", "en-XA")

    assert said.live == ["switching to en-XA"]
    assert said.ended[0].startswith(f"switching to {console.lit('en-XA')} (")


def test_a_step_that_fails_is_not_told_as_ended(monkeypatch: pytest.MonkeyPatch) -> None:
    said = Said(monkeypatch)

    with pytest.raises(RuntimeError, match="the walk failed"), steps_of(said.live.append) as steps:
        steps.start("logging in")
        steps.start("walking")
        raise RuntimeError("the walk failed")

    assert [line.split(" (")[0] for line in said.ended] == ["logging in"]
    assert [line.split(" (")[0] for line in said.failed] == ["walking"]


def test_an_interrupted_step_is_not_told_as_ended(monkeypatch: pytest.MonkeyPatch) -> None:
    said = Said(monkeypatch)

    with pytest.raises(KeyboardInterrupt), steps_of(said.live.append) as steps:
        steps.start("walking")
        raise KeyboardInterrupt

    assert said.ended == []
    assert [line.split(" (")[0] for line in said.failed] == ["walking"]


def test_each_widget_recipe_is_told_and_a_failed_one_costs_only_itself(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    said = Said(monkeypatch)
    recipes = [(name, Path(f"{name}.capture.toml")) for name in ("clock", "weather", "ticker")]

    def listed(_root: Path) -> list[tuple[str, Path]]:
        return recipes

    def captured(_recipe: str, _out: Path, _log: Path, phase: str) -> None:
        if phase.startswith("weather"):
            raise Abort("weather failed")

    monkeypatch.setattr("intl_sweep.__main__.widget_recipes", listed)
    monkeypatch.setattr("intl_sweep.__main__.capture", captured)

    with pytest.raises(Abort, match="1 of 3 recipes failed: weather"):
        _ = widgets()

    told = [Text.from_markup(line).plain.split(" (")[0] for line in said.ended]
    assert told == ["capturing clock", "capturing ticker"]
    assert Text.from_markup(said.failed[0]).plain.startswith("capturing weather (")
    assert "weather failed" in said.failed


def walking_raises(monkeypatch: pytest.MonkeyPatch, error: Exception) -> None:
    def walked(_url: str, _password: str) -> str:
        raise error

    monkeypatch.setattr("intl_sweep.__main__._walk_web", walked)


def test_an_interrupt_with_its_context_exits_as_interrupted_and_says_what_happened(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    said = Said(monkeypatch)
    what = "the walk was interrupted while deleting /display/7"
    walking_raises(monkeypatch, Interrupted(what))

    with pytest.raises(SystemExit) as exit_:
        web("http://deck", "root")

    assert exit_.value.code == EXIT_INTERRUPTED
    assert said.failed == [console.lit(what)]


def test_a_scene_that_failed_to_delete_stays_a_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    _ = Said(monkeypatch)
    walking_raises(monkeypatch, SceneLeft("deleting /display/7 failed: still listed"))

    with pytest.raises(Abort, match="deleting /display/7 failed"):
        web("http://deck", "root")


class Device:
    """What the web stage finds at the URL it walks, and what it was asked to do there."""

    def __init__(
        self,
        monkeypatch: pytest.MonkeyPatch,
        tmp_path: Path,
        *,
        in_setup: bool = False,
        offered: tuple[str, ...] = ("en-XA",),
        language: str = "en-US",
    ) -> None:
        self._in_setup: bool = in_setup
        self._offered: list[str] = list(offered)
        self._language: str = language
        for name, stand_in in (
            ("SWEEP", tmp_path),
            ("LOGS", tmp_path),
            ("open_probe", self.opened),
            ("in_setup", self.setting_up),
            ("log_in", self.logged_in),
            ("offered_languages", self.offers),
            ("current_language", self.speaks),
            ("speaking", self.speaking),
            ("shoot_pages", self.pages),
            ("shoot_login", self.login),
        ):
            monkeypatch.setattr(f"intl_sweep.__main__.{name}", stand_in)

    @contextmanager
    def opened(self, _url: str) -> Generator[Self]:
        yield self

    def setting_up(self, _probe: object) -> bool:
        return self._in_setup

    def logged_in(self, _probe: object, _password: str) -> None:
        pass

    def offers(self, _probe: object) -> list[str]:
        return self._offered

    def speaks(self, _probe: object) -> str:
        return self._language

    @contextmanager
    def speaking(self, _probe: object, _tag: str) -> Generator[None]:
        yield

    def pages(self, _probe: object, out: Path) -> Iterator[tuple[str, Path]]:
        yield "/display", out / "display.png"

    def login(self, _probe: object, _out: Path) -> None:
        return None

    def shot(self, file: Path) -> Path:
        return file


def steps_told(said: Said) -> list[str]:
    return [Text.from_markup(line).plain.split(" (")[0] for line in said.ended]


def test_a_walk_tells_each_step_it_took(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    said = Said(monkeypatch)
    _ = Device(monkeypatch, tmp_path)

    _ = _walk_web("http://deck", "root")

    assert steps_told(said) == [
        "opening the web UI",
        "logging in",
        "reading the device's languages",
        "switching the device to en-XA",
        "finding the pages",
        "capturing the pages",
        "capturing the login page",
        "setting the device's language back",
    ]


def test_a_device_in_setup_is_not_told_as_logged_into(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    said = Said(monkeypatch)
    _ = Device(monkeypatch, tmp_path, in_setup=True)

    _ = _walk_web("http://deck", "root")

    assert steps_told(said) == ["opening the web UI"]


def test_a_device_offering_no_languages_fails_at_reading_them_not_at_logging_in(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    said = Said(monkeypatch)
    _ = Device(monkeypatch, tmp_path, offered=())

    with pytest.raises(Abort, match="no languages offered"):
        _ = _walk_web("http://deck", "root")

    assert steps_told(said) == ["opening the web UI", "logging in"]
    assert said.failed[0].startswith("reading the device's languages (")


def test_a_device_already_speaking_the_language_is_not_told_as_switched(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    said = Said(monkeypatch)
    _ = Device(monkeypatch, tmp_path, language="en-XA")

    _ = _walk_web("http://deck", "root")

    told = steps_told(said)
    assert "switching the device to en-XA" not in told
    assert "setting the device's language back" not in told


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
