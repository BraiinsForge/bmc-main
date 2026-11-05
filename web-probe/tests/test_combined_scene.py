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

"""Unit tests for when a walk deletes the scene it created, and when it believes it gone."""

from collections.abc import Callable, Generator
from contextlib import contextmanager
from typing import cast

import pytest
from playwright.sync_api import Response
from playwright.sync_api import TimeoutError as PlaywrightTimeoutError

from web_probe import __main__ as cli
from web_probe import device
from web_probe.browser import Probe, Unreachable
from web_probe.device import (
    EditorUnreached,
    Interrupted,
    SceneLeft,
    _add_combined_scene,
    _delete_scene,
    _is_remove_scene,
    _opened_scene,
    _own_combined_scene,
)

SCENE = "/display/7"
SCENE_DELETE = '[id="display-comp-scene-overview-row-7-delete"]'
LIST_LOADED = "#display-list-capacity"
# `_own_combined_scene` reaches the page only through the calls `Device` stands in for.
PROBE = cast(Probe, object())


class Device:
    """The device's scenes, as adding and deleting one goes."""

    def __init__(
        self,
        *,
        can_add: bool = True,
        deleting_fails: bool = False,
        deleting_interrupted: bool = False,
    ) -> None:
        self.deleted: list[str] = []
        self._can_add: bool = can_add
        self._deleting_fails: bool = deleting_fails
        self._deleting_interrupted: bool = deleting_interrupted

    def add(self, _probe: Probe) -> str | None:
        return SCENE if self._can_add else None

    def delete(self, _probe: Probe, scene: str) -> None:
        if self._deleting_interrupted:
            raise KeyboardInterrupt
        if self._deleting_fails:
            raise SceneLeft(f"deleting {scene} failed")
        self.deleted.append(scene)


def installed(monkeypatch: pytest.MonkeyPatch, scenes: Device) -> Device:
    monkeypatch.setattr(device, "_add_combined_scene", scenes.add)
    monkeypatch.setattr(device, "_delete_scene", scenes.delete)
    return scenes


def test_the_scene_is_deleted_after_the_block(monkeypatch: pytest.MonkeyPatch) -> None:
    scenes = installed(monkeypatch, Device())

    with _own_combined_scene(PROBE) as scene:
        assert scene == SCENE
        assert scenes.deleted == [], "the block still has its scene"

    assert scenes.deleted == [SCENE]


def test_a_failing_block_still_deletes_the_scene(monkeypatch: pytest.MonkeyPatch) -> None:
    scenes = installed(monkeypatch, Device())

    with pytest.raises(RuntimeError, match="the walk failed"), _own_combined_scene(PROBE):
        raise RuntimeError("the walk failed")

    assert scenes.deleted == [SCENE]


def test_a_device_that_cannot_add_one_gives_no_scene_and_deletes_nothing(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    scenes = installed(monkeypatch, Device(can_add=False))

    with _own_combined_scene(PROBE) as scene:
        assert scene is None

    assert scenes.deleted == []


def test_an_interrupt_with_the_scene_deleted_claims_no_scene(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    scenes = installed(monkeypatch, Device())

    with pytest.raises(KeyboardInterrupt), _own_combined_scene(PROBE):
        raise KeyboardInterrupt

    assert scenes.deleted == [SCENE]


def test_an_interrupt_and_a_failed_delete_are_both_named(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _ = installed(monkeypatch, Device(deleting_fails=True))

    with (
        pytest.raises(Interrupted, match=f"interrupted, then deleting {SCENE} failed"),
        _own_combined_scene(PROBE),
    ):
        raise KeyboardInterrupt


def test_an_interrupt_during_the_delete_after_a_clean_block_names_the_scene(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _ = installed(monkeypatch, Device(deleting_interrupted=True))

    with (
        pytest.raises(Interrupted, match=f"interrupted while deleting {SCENE}"),
        _own_combined_scene(PROBE),
    ):
        pass


def test_a_second_interrupt_during_the_delete_names_the_scene(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _ = installed(monkeypatch, Device(deleting_interrupted=True))

    with (
        pytest.raises(Interrupted, match=f"interrupted while deleting {SCENE}"),
        _own_combined_scene(PROBE),
    ):
        raise KeyboardInterrupt


def test_an_interrupt_during_a_failing_blocks_delete_names_the_scene(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _ = installed(monkeypatch, Device(deleting_interrupted=True))

    with (
        pytest.raises(Interrupted, match=f"interrupted while deleting {SCENE}"),
        _own_combined_scene(PROBE),
    ):
        raise RuntimeError("the walk failed")


def test_a_scene_left_after_a_clean_block_is_refused(monkeypatch: pytest.MonkeyPatch) -> None:
    _ = installed(monkeypatch, Device(deleting_fails=True))

    with pytest.raises(SceneLeft, match=f"deleting {SCENE} failed"), _own_combined_scene(PROBE):
        pass


LISTED = "the list loaded and shows the scene"
GONE = "the list loaded without the scene"
BROKEN = "the list failed to load, showing no rows at all"
UNREACHABLE = "the page did not load"


class Located:
    """What the delete looks for on the page."""

    def __init__(self, listings: "Listings", selector: str) -> None:
        self._listings: Listings = listings
        self._selector: str = selector

    def wait_for(self, *, timeout: int, state: str = "visible") -> None:
        _ = timeout
        current = self._listings.current
        if self._selector == LIST_LOADED:
            resolved = current != BROKEN
        else:
            assert self._selector == SCENE_DELETE, "only the walk's own scene may be deleted"
            resolved = current != LISTED if state == "detached" else current == LISTED
        if not resolved:
            raise PlaywrightTimeoutError(f"{self._selector} did not get {state}")

    def click(self, *, timeout: int) -> None:
        _ = timeout
        self._listings.clicks += 1


class Listings:
    """A probe's display list: what each fresh visit finds, the last answer staying."""

    def __init__(self, *visits: str) -> None:
        self.clicks: int = 0
        self.visits: int = 0
        self.current: str = ""
        self.page: Listings = self
        self._visits: list[str] = list(visits)

    def visit(self, _path: str) -> None:
        self.visits += 1
        self.current = self._visits.pop(0) if len(self._visits) > 1 else self._visits[0]
        if self.current == UNREACHABLE:
            raise Unreachable("the page did not load")

    def locator(self, selector: str) -> Located:
        return Located(self, selector)

    @contextmanager
    def expect_response(self, _matcher: object, *, timeout: int) -> Generator[None]:
        _ = timeout
        yield


class Answered:
    """A response, as far as the delete reads one."""

    def __init__(self, url: str) -> None:
        self.url: str = url


def test_the_wait_recognises_the_devices_remove_scene_answer() -> None:
    answer = Answered("http://deck/braiins.bmc.web.SceneManagementService/RemoveScene")

    assert _is_remove_scene(cast(Response, cast(object, answer)))


def test_the_wait_ignores_the_devices_other_answers() -> None:
    answer = Answered("http://deck/braiins.bmc.web.SceneManagementService/GetScenes")

    assert not _is_remove_scene(cast(Response, cast(object, answer)))


def deleting(listings: Listings) -> None:
    _delete_scene(cast(Probe, cast(object, listings)), SCENE)


def test_a_scene_is_gone_once_a_loaded_list_stops_showing_it() -> None:
    listings = Listings(LISTED, GONE)

    deleting(listings)

    assert listings.clicks == 1


def test_a_scene_a_loaded_list_never_shows_is_not_believed_gone() -> None:
    listings = Listings(GONE)

    with pytest.raises(SceneLeft, match="never listed"):
        deleting(listings)

    assert listings.clicks == 0


def test_a_refused_delete_is_tried_again_until_the_scene_is_gone() -> None:
    listings = Listings(LISTED, LISTED, LISTED, GONE)

    deleting(listings)

    assert listings.clicks == 3


def test_a_delete_accepted_on_the_last_attempt_is_seen_to_have_worked() -> None:
    listings = Listings(*[LISTED] * device._DELETE_ATTEMPTS, GONE)

    deleting(listings)

    assert listings.clicks == device._DELETE_ATTEMPTS


def test_a_list_that_failed_to_load_is_not_believed_after_a_delete() -> None:
    listings = Listings(LISTED, BROKEN, BROKEN, GONE)

    deleting(listings)

    assert listings.visits == 4, "only the list that loaded could say the scene is gone"


def test_a_scene_behind_a_list_that_never_loads_is_left() -> None:
    listings = Listings(BROKEN)

    with pytest.raises(SceneLeft, match="did not get visible"):
        deleting(listings)

    assert listings.clicks == 0


def test_a_scene_that_stays_listed_is_left() -> None:
    with pytest.raises(SceneLeft, match="still listed"):
        deleting(Listings(LISTED))


def test_one_page_that_does_not_load_does_not_end_the_retries() -> None:
    listings = Listings(UNREACHABLE, LISTED, GONE)

    deleting(listings)

    assert listings.clicks == 1


class Visited:
    """A frame as far as the walk reads one: one object whose URL changes as it navigates."""

    def __init__(self) -> None:
        self.url: str = f"http://deck{device.DISPLAY_PATH}"

    def navigate(self, path: str) -> None:
        self.url = f"http://deck{path}"


class Offered:
    """Whatever the add flow finds on the page: the add button, the menu and its entries."""

    def __init__(self, adding: "Adding") -> None:
        self._adding: Adding = adding
        self.first: Offered = self

    def is_disabled(self) -> bool:
        return self._adding.add_disabled

    def wait_for(self) -> None:
        pass

    def count(self) -> int:
        return 1 if self._adding.combined_offered else 0

    def click(self) -> None:
        self._adding.clicks += 1


class Adding:
    """A probe's display list, and where its frames go once the add entry is clicked."""

    def __init__(
        self,
        *paths: str,
        in_another_frame: str | None = None,
        listings_fail: bool = False,
        interrupted: bool = False,
        add_disabled: bool = False,
        combined_offered: bool = True,
    ) -> None:
        self.clicks: int = 0
        self.add_disabled: bool = add_disabled
        self.combined_offered: bool = combined_offered
        self.keyboard: Adding = self
        self.main_frame: Visited = Visited()
        self.page: Adding = self
        self._paths: tuple[str, ...] = paths
        self._in_another_frame: str | None = in_another_frame
        self._listings_fail: bool = listings_fail
        self._interrupted: bool = interrupted

    def visit(self, _path: str) -> None:
        if self._listings_fail and self.clicks > 0:
            raise Unreachable("the page did not load")

    def press(self, _key: str) -> None:
        pass

    def locator(self, _selector: str) -> Offered:
        return Offered(self)

    def get_by_role(self, _role: str) -> Offered:
        return Offered(self)

    @contextmanager
    def expect_event(
        self, _event: str, *, predicate: Callable[[Visited], bool], timeout: int
    ) -> Generator[None]:
        _ = timeout
        yield
        assert self.clicks > 0, "only what the walk's own click opened may count"
        if self._interrupted:
            raise KeyboardInterrupt
        if self._in_another_frame is not None:
            other = Visited()
            other.navigate(self._in_another_frame)
            assert not predicate(other), "a frame inside the page opens no scene of the walk's"
        seen = False
        for path in self._paths:
            self.main_frame.navigate(path)
            seen = seen or predicate(self.main_frame)
        if not seen:
            raise PlaywrightTimeoutError("no navigation")


def listing(monkeypatch: pytest.MonkeyPatch, before: set[str], after: set[str]) -> None:
    listings = iter((before, after))

    def listed(_probe: Probe) -> set[str]:
        return next(listings)

    monkeypatch.setattr(device, "_listed_scenes", listed)


def adding_on(page: Adding) -> str | None:
    return _add_combined_scene(cast(Probe, cast(object, page)))


def test_the_scene_is_the_one_the_click_opened(monkeypatch: pytest.MonkeyPatch) -> None:
    listing(monkeypatch, {"1"}, {"1", "7"})

    assert adding_on(Adding(SCENE)) == SCENE


def test_a_scene_the_editor_left_at_once_is_still_the_one_it_opened(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    listing(monkeypatch, {"1"}, {"1"})

    assert adding_on(Adding(SCENE, device.DISPLAY_PATH)) == SCENE


def test_a_row_nothing_ties_to_the_click_is_reported_and_never_deleted(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    scenes = Device()
    monkeypatch.setattr(device, "_delete_scene", scenes.delete)
    listing(monkeypatch, {"1"}, {"1", "9"})

    with (
        pytest.raises(SceneLeft, match="/display/9"),
        _own_combined_scene(cast(Probe, cast(object, Adding()))),
    ):
        pytest.fail("no scene is the walk's to work on")

    assert scenes.deleted == []


def test_a_navigation_inside_another_frame_is_not_the_click_opening_a_scene(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    listing(monkeypatch, {"1"}, {"1"})

    with pytest.raises(EditorUnreached, match="opened no editor"):
        _ = adding_on(Adding(in_another_frame=SCENE))


def test_an_add_that_changed_nothing_is_refused(monkeypatch: pytest.MonkeyPatch) -> None:
    listing(monkeypatch, {"1"}, {"1"})

    with pytest.raises(EditorUnreached, match="opened no editor"):
        _ = adding_on(Adding())


def test_a_device_that_cannot_add_a_scene_is_refused(monkeypatch: pytest.MonkeyPatch) -> None:
    listing(monkeypatch, {"1"}, {"1"})

    with pytest.raises(EditorUnreached, match="limit of running widgets"):
        _ = adding_on(Adding(add_disabled=True))


def test_a_web_ui_offering_no_combined_scene_gives_none_and_says_so(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    listing(monkeypatch, {"1"}, {"1"})

    assert adding_on(Adding(combined_offered=False)) is None
    assert "offers no combined scene" in capsys.readouterr().err


def test_a_list_unreadable_after_an_add_that_opened_nothing_is_refused(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    listing(monkeypatch, {"1"}, {"1"})

    with pytest.raises(SceneLeft, match="did not load"):
        _ = adding_on(Adding(listings_fail=True))


def test_an_interrupt_while_adding_says_so(monkeypatch: pytest.MonkeyPatch) -> None:
    listing(monkeypatch, {"1"}, {"1"})

    with pytest.raises(Interrupted, match="interrupted while adding"):
        _ = adding_on(Adding(interrupted=True))


def test_the_walk_renders_an_interrupt_with_its_context_and_exits_as_interrupted(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    @contextmanager
    def opened(_origin: str) -> Generator[Probe]:
        yield PROBE

    def interrupted(_probe: Probe, _password: str) -> None:
        raise Interrupted(f"the walk was interrupted while deleting {SCENE}")

    monkeypatch.setattr(cli, "open_probe", opened)
    monkeypatch.setattr(cli, "log_in", interrupted)

    assert cli.walk("http://deck") == cli.EXIT_INTERRUPTED
    assert SCENE in capsys.readouterr().err


@pytest.mark.parametrize("path", ["/display", "/display/", "/display/7/edit", "/login"])
def test_other_pages_are_no_scene_editor(path: str) -> None:
    assert _opened_scene(f"http://deck{path}") is None


def test_a_scene_left_by_a_failing_block_keeps_its_error_and_says_so(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    _ = installed(monkeypatch, Device(deleting_fails=True))

    with pytest.raises(RuntimeError, match="the walk failed"), _own_combined_scene(PROBE):
        raise RuntimeError("the walk failed")

    assert SCENE in capsys.readouterr().err
