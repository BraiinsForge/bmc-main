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

"""Unit tests for a walk's shots: what is shot on a page, and what each is named."""

from collections.abc import Iterator
from pathlib import Path
from typing import cast

import pytest

from web_probe import device
from web_probe.browser import Probe
from web_probe.device import TabsMissing, shoot_pages, shot_name

OUT = Path("out")
EDITOR_SHOT = ("/editor", OUT / "editor.png")


class Page:
    """The browser page, as far as opening a tab goes."""

    def __init__(self, seen: list[str]) -> None:
        self._seen: list[str] = seen

    def click(self, selector: str) -> None:
        self._seen.append(selector)


class Shooter:
    """A probe taking its shots without a browser, keeping what it did in order in `seen`."""

    def __init__(self) -> None:
        self.seen: list[str] = []
        self.page: Page = Page(self.seen)

    def path(self) -> str:
        return "/settings"

    def shot(self, file: Path) -> Path:
        self.seen.append(file.name)
        return file


def editor(_probe: Probe, _out: Path) -> Iterator[tuple[str, Path]]:
    yield EDITOR_SHOT


def walked(
    monkeypatch: pytest.MonkeyPatch, tabs: dict[str, list[str]], shooter: Shooter | None = None
) -> list[tuple[str, Path]]:
    """The shots of a UI with the pages in `tabs`, each with the tabs it offers."""
    open_page = [""]

    def pages(_probe: Probe) -> Iterator[str]:
        for path in tabs:
            open_page[0] = path
            yield path

    def page_tabs(_probe: Probe) -> list[str]:
        return tabs[open_page[0]]

    monkeypatch.setattr(device, "pages", pages)
    monkeypatch.setattr(device, "page_tabs", page_tabs)
    monkeypatch.setattr(device, "shoot_combined_editor", editor)
    return list(shoot_pages(cast(Probe, cast(object, shooter or Shooter())), OUT))


def test_a_page_with_tabs_is_shot_once_per_tab(monkeypatch: pytest.MonkeyPatch) -> None:
    shots = walked(monkeypatch, {"/settings": ["general", "updates"]})

    assert shots == [
        ("/settings#general", OUT / "settings_general.png"),
        ("/settings#updates", OUT / "settings_updates.png"),
        EDITOR_SHOT,
    ]


def test_each_tab_is_opened_before_its_shot(monkeypatch: pytest.MonkeyPatch) -> None:
    shooter = Shooter()

    _ = walked(monkeypatch, {"/settings": ["general", "updates"]}, shooter)

    assert shooter.seen == [
        '[data-tab]:enabled[data-tab="general"]',
        "settings_general.png",
        '[data-tab]:enabled[data-tab="updates"]',
        "settings_updates.png",
    ]


def test_a_page_without_tabs_is_shot_as_itself(monkeypatch: pytest.MonkeyPatch) -> None:
    assert walked(monkeypatch, {"/alarms": []}) == [("/alarms", OUT / "alarms.png"), EDITOR_SHOT]


def test_settings_without_tabs_is_refused_not_shot_as_if_walked(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    with pytest.raises(TabsMissing, match="data-tab"):
        _ = walked(monkeypatch, {"/alarms": [], "/settings": []})


@pytest.mark.parametrize(
    ("path", "name"),
    [
        ("/", "home.png"),
        ("/settings", "settings.png"),
        ("/display/1", "display_1.png"),
        ("/display/", "display.png"),
        ("/settings#sound-and-light", "settings_sound-and-light.png"),
    ],
)
def test_every_page_gets_a_file_of_its_own(path: str, name: str) -> None:
    assert shot_name(path) == name
