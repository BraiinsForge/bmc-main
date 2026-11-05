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

"""The device's own web UI: logging in, its pages, and its language picker.

Elements are found by the ids the frontend derives from its form paths (`getID`),
by `data-tab`, by their roles and by ARIA relations, which no language changes, never by wording.
"""

from collections.abc import Generator, Iterator
from contextlib import contextmanager
from pathlib import Path
from urllib.parse import urlsplit

from bmc_tui import console
from playwright.sync_api import Error as PlaywrightError
from playwright.sync_api import Frame, Locator, Response
from playwright.sync_api import TimeoutError as PlaywrightTimeoutError

from web_probe.browser import Probe, Unreachable, crawl

LOGIN_PATH = "/login"
SETTINGS_PATH = "/settings"
# The Wi-Fi wizard of a device at factory defaults or reconfiguring its Wi-Fi,
# and the device setup of one whose setup is pending.
SETUP_PATHS = ("/init_connect", "/init_setup")

# A login the device answers does so well within this.
LOGIN_TIMEOUT_MS = 10_000

_PASSWORD = "#login-password"
_SUBMIT = "#login-submit"
_LANGUAGE = "#settings-general-language"
_TAB = "[data-tab]:enabled"

DISPLAY_PATH = "/display"
# A scene's id changes with every walk, so its shots are named by this instead.
_EDITOR = "/display/combined"
_ADD_SCENE = "#display-list-add-scene button"
_ADD_COMBINED = "#display-list-add-combined"
_SCENE_ROW = "display-comp-scene-overview-row-"
_DELETE_SUFFIX = "-delete"
_REMOVE_SCENE = ".SceneManagementService/RemoveScene"
# Shown only by a display list whose scenes loaded, with or without any scene.
_LIST_LOADED = "#display-list-capacity"
# Shown with the scene itself, from the same answer of the device.
_SCENE_LOADED = "#display-combined-capacity"
_ADD_WIDGET = '[id^="display-comp-combined-scene-widget-"][id$="-add"]:enabled'
_DELETE_SCENE = "#display-combined-delete"
_PICKER = "#display-comp-scene-select-kind-modal"
_PICKED_WIDGET = f"{_PICKER} button[aria-describedby]"
_WIDGET_FORM = "#display-comp-manifest-form-dialog"
_WIDGET_FORM_DONE = "#display-comp-manifest-form-done"
_CONFIRMATION = "#confirmation-modal"
# The budget of the frontend's `SCENE_WRITE_TIMEOUT_MS`, which a view opened by a write can take.
_EDITOR_TIMEOUT_MS = 30_000
# The device refuses to delete a scene until its preview ends, which follows the editor closing.
_DELETE_ATTEMPTS = 10
_DELETE_RETRY_MS = 1_000
_DELETE_STEP_MS = 5_000
_INTERRUPTED = "the walk was interrupted"

# What the login form shows when the device refuses: an error notification for the form,
# or the password field marked invalid. Either describes the reason through `aria-describedby`.
_REFUSAL = f'form [role="alertdialog"], {_PASSWORD}[aria-invalid="true"]'


class LoginRefused(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """The login form showed the device's refusal."""


class LoginStalled(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """The login page neither left nor showed a refusal after the password was submitted."""


class LanguageUnknown(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """The language picker shows none of the device's languages as its own."""


class TabsMissing(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """The settings page shows no tabs, so all but its first section would go unseen."""


class LanguageStuck(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """The device's language could not be set back after the walk switched it."""


class EditorUnreached(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """The walk could not add the scene its combined-scene editor views are shot in."""


class SceneLeft(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """Adding or deleting the walk's combined scene did not end as the list should show it."""


class Interrupted(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """A Ctrl-C ended the walk in the middle of what the message names."""


@contextmanager
def _while(doing: str) -> Generator[None]:
    try:
        yield
    except KeyboardInterrupt as interrupted:
        raise Interrupted(f"{_INTERRUPTED} while {doing}") from interrupted


def log_in(probe: Probe, password: str) -> None:
    """Pass the login page if the device shows one. A device without a password shows none."""
    probe.visit("/")
    if probe.path() != LOGIN_PATH:
        return
    probe.page.fill(_PASSWORD, password)
    probe.page.click(_SUBMIT)
    refused = probe.page.locator(_REFUSAL)

    # The app's side navigation shows only once the device took the login.
    logged_in = probe.page.get_by_role("navigation")
    try:
        logged_in.or_(refused).first.wait_for(timeout=LOGIN_TIMEOUT_MS)
    except PlaywrightTimeoutError as error:
        seconds = LOGIN_TIMEOUT_MS // 1000
        msg = f"the login page showed neither the app nor an error within {seconds} s"
        raise LoginStalled(msg) from error

    if not refused.count():
        probe.page.wait_for_load_state("networkidle")
        return

    said = _description(probe, refused.first)
    raise LoginRefused(f"the device refused the login: {said or 'no reason shown'}")


def _description(probe: Probe, element: Locator) -> str:
    """The text of the elements `element` is described by, through its `aria-describedby`."""
    ids = (element.get_attribute("aria-describedby") or "").split()
    # An attribute selector takes a generated id as it is, where `#id` would need it CSS-escaped.
    texts = (text for id_ in ids for text in probe.page.locator(f'[id="{id_}"]').all_inner_texts())
    return " ".join(text.strip() for text in texts)


def in_setup(probe: Probe) -> bool:
    """Whether the device serves a setup wizard in place of the app, as one being set up does."""
    probe.visit("/")
    return probe.path().startswith(SETUP_PATHS)


def offered_languages(probe: Probe) -> list[str]:
    """The tags the language picker offers.
    Empty when the device offers no languages, as one without its i18n marker (`/etc/bmc/i18n`),
    which then shows no picker."""
    probe.visit(SETTINGS_PATH)
    picker = probe.page.locator(_LANGUAGE)
    if picker.count() == 0:
        return []
    picker.click()
    options = probe.page.locator('[role="option"] [lang]').all()
    tags = [tag for option in options if (tag := option.get_attribute("lang"))]
    probe.page.keyboard.press("Escape")
    return tags


def current_language(probe: Probe) -> str | None:
    """The tag the language picker shows as the device's, or none without a picker."""
    probe.visit(SETTINGS_PATH)
    shown = probe.page.locator(f"{_LANGUAGE} [lang]")
    return shown.first.get_attribute("lang") if shown.count() else None


def set_language(probe: Probe, tag: str) -> None:
    """Pick `tag` in the language picker and wait until the page speaks it."""
    probe.visit(SETTINGS_PATH)
    probe.page.click(_LANGUAGE)
    probe.page.click(f'[role="option"] [lang="{tag}"]')
    _ = probe.page.wait_for_selector(f'html[lang="{tag}"]', state="attached")


@contextmanager
def speaking(probe: Probe, tag: str) -> Generator[None]:
    """The device speaking `tag` for the block and its own language again after it:
    the language is the device's setting, its screens follow it too.
    Refused before any switch when its own language is none it offers,
    as that could not be set back."""
    before = current_language(probe)
    if before is None:
        msg = (
            "the device's own language is none it offers, so a switch could not be set back:"
            " pick one in its settings first"
        )
        raise LanguageUnknown(msg)
    if before == tag:
        yield
        return

    try:
        # Inside the `try`: the switch can fail waiting
        # for the page after the device took it.
        set_language(probe, tag)
        yield
    except BaseException:
        # The block's own error explains the failure.
        # One from setting the language back must not replace it, nor go unsaid.
        try:
            _set_language_back(probe, before)
        except LanguageStuck as stuck:
            console.error(console.lit(stuck))
        raise
    _set_language_back(probe, before)


def _set_language_back(probe: Probe, before: str) -> None:
    doing = f"setting the device's language back to {before}"
    try:
        with _while(doing):
            set_language(probe, before)
    except (PlaywrightError, Unreachable) as error:
        raise LanguageStuck(f"{doing} failed: {str(error).splitlines()[0]}") from error


def pages(probe: Probe) -> Iterator[str]:
    """Every page the logged-in UI leads to, from the page its home redirects to,
    the login page left out: its links, and the side navigation's entries,
    which navigate by script. The probe is on each page while its path is yielded."""
    probe.visit("/")
    home = probe.path()
    navigation = probe.navigation_paths()

    def links(path: str) -> list[str]:
        probe.visit(path)
        return [*probe.links(), *navigation]

    return crawl(home, links, skip={LOGIN_PATH, "/"})


def shot_name(path: str) -> str:
    """The file a page's shot is saved as: `/display/1` as `display_1.png`, `/` as `home.png`,
    `/settings#updates` as `settings_updates.png`."""
    name = path.strip("/").replace("/", "_").replace("#", "_")
    return f"{name or 'home'}.png"


def page_tabs(probe: Probe) -> list[str]:
    """The keys of the tabs the current page offers."""
    tabs = probe.page.locator(_TAB).all()
    return [key for tab in tabs if (key := tab.get_attribute("data-tab"))]


def shoot_pages(probe: Probe, out: Path) -> Iterator[tuple[str, Path]]:
    """Every page `pages` reaches, shot whole into `out`, with the path it was shot at.
    A page with tabs is shot once per tab, as `path#tab`, the tabs its device offers.
    Settings always has tabs, so finding none there is refused, not shot as if it had been walked:
    a passing run would hide the sections it never saw.
    The combined-scene editor, which no link leads to, comes last."""
    for path in pages(probe):
        tabs = page_tabs(probe)
        if not tabs and path == SETTINGS_PATH:
            msg = f"settings shows no tabs, at {probe.path()}: a frontend without `data-tab`?"
            raise TabsMissing(msg)
        if not tabs:
            yield path, probe.shot(out / shot_name(path))
        for key in tabs:
            probe.page.click(f'{_TAB}[data-tab="{key}"]')
            shown = f"{path}#{key}"
            yield shown, probe.shot(out / shot_name(shown))
    yield from shoot_combined_editor(probe, out)


def shoot_combined_editor(probe: Probe, out: Path) -> Iterator[tuple[str, Path]]:
    """The combined-scene editor and what it opens, in a scene the walk creates and deletes,
    so that adding a widget, which saves at once, touches nothing of the device's own."""

    def shot(view: str = "") -> tuple[str, Path]:
        shown = f"{_EDITOR}#{view}" if view else _EDITOR
        return shown, probe.shot(out / shot_name(shown))

    with _own_combined_scene(probe) as scene:
        if scene is None or not _editor_opened(probe, scene):
            return
        # The scene can load before the device refuses its preview, which sends the page away.
        try:
            yield shot()
            for view in _open_widget_views(probe):
                if probe.path() != scene:
                    break
                yield shot(view)
            if probe.path() == scene:
                probe.page.click(_DELETE_SCENE)
                probe.page.locator(_CONFIRMATION).wait_for()
                yield shot("delete-confirmation")
        except PlaywrightError:
            if probe.path() == scene:
                raise
        if probe.path() != scene:
            console.warn(f"the editor left for {probe.path()}: its other views went unseen")


def _editor_opened(probe: Probe, scene: str) -> bool:
    """Whether the editor at path `scene` loaded its scene before the page left it."""
    try:
        probe.page.locator(_SCENE_LOADED).wait_for(timeout=_EDITOR_TIMEOUT_MS)
    except PlaywrightTimeoutError:
        if probe.path() == scene:
            raise
        console.warn(f"the editor left for {probe.path()}: its views went unseen")
        return False
    return True


def _open_widget_views(probe: Probe) -> Iterator[str]:
    """The views of adding a widget, each named once it is open, closed again before the end."""
    page = probe.page
    add = page.locator(_ADD_WIDGET).first
    try:
        add.wait_for(timeout=_EDITOR_TIMEOUT_MS)
    except PlaywrightTimeoutError:
        console.warn("no slot offered to add a widget, so the editor's widget views went unseen")
        return
    add.click()
    page.locator(_PICKER).wait_for()

    widget = page.locator(_PICKED_WIDGET).first
    try:
        widget.wait_for(timeout=_EDITOR_TIMEOUT_MS)
    except PlaywrightTimeoutError:
        console.warn("the picker listed no widget in time, so the widget form went unseen")
        yield "picker"
        page.keyboard.press("Escape")
        return
    yield "picker"

    widget.click()
    form = page.locator(_WIDGET_FORM)
    try:
        form.wait_for(timeout=_EDITOR_TIMEOUT_MS)
    except PlaywrightTimeoutError:
        console.warn("the widget form did not open in time, so it went unseen")
        page.keyboard.press("Escape")
        return
    yield "widget-form"

    done = page.locator(_WIDGET_FORM_DONE)
    if done.is_enabled():
        done.click()
    try:
        form.wait_for(state="hidden", timeout=_EDITOR_TIMEOUT_MS)
    except PlaywrightTimeoutError:
        console.warn("the widget form did not close, so a placed widget went unseen")
        page.keyboard.press("Escape")
        form.wait_for(state="hidden")
        return
    yield "with-widget"


@contextmanager
def _own_combined_scene(probe: Probe) -> Generator[str | None]:
    """The path of a combined scene created for the block and deleted after it,
    or none where the device cannot add one.
    A scene that could not be deleted is refused, or told beside the block's own error."""
    scene = _add_combined_scene(probe)
    if scene is None:
        yield None
        return
    try:
        yield scene
    except KeyboardInterrupt as interrupted:
        try:
            _delete_interruptibly(probe, scene)
        except SceneLeft as left:
            raise Interrupted(f"{_INTERRUPTED}, then {left}") from interrupted
        raise
    except BaseException:
        try:
            _delete_interruptibly(probe, scene)
        except SceneLeft as left:
            console.error(console.lit(left))
        raise
    _delete_interruptibly(probe, scene)


def _delete_interruptibly(probe: Probe, scene: str) -> None:
    with _while(f"deleting {scene}"):
        _delete_scene(probe, scene)


def _add_combined_scene(probe: Probe) -> str | None:
    """Add a combined scene from the display page and open its editor, returning its path,
    or none where the web UI offers no combined scene.
    Refuses a device it cannot add a scene on, as a passing run would hide the editor's absence."""
    probe.visit(DISPLAY_PATH)
    before = _listed_scenes(probe)
    add = probe.page.locator(_ADD_SCENE)
    if add.is_disabled():
        msg = (
            "adding a scene is disabled, so the combined-scene editor went unseen:"
            " is the device at its limit of running widgets?"
        )
        raise EditorUnreached(msg)
    add.click()
    probe.page.get_by_role("menuitem").first.wait_for()
    combined = probe.page.locator(_ADD_COMBINED)
    if combined.count() == 0:
        console.warn("the web UI offers no combined scene, so the editor went unseen")
        probe.page.keyboard.press("Escape")
        return None
    with _while("adding a combined scene"):
        return _scene_opened_by(probe, combined, before)


def _scene_opened_by(probe: Probe, entry: Locator, before: set[str]) -> str:
    """The path of the scene that clicking the add `entry` opened."""
    # Read as the page passes through it, since the editor can send the page straight back.
    opened: list[str] = []

    def opens_a_scene(frame: Frame) -> bool:
        scene = _opened_scene(frame.url) if frame == probe.page.main_frame else None
        if scene is not None:
            opened.append(scene)
        return scene is not None

    try:
        with probe.page.expect_event(
            "framenavigated", predicate=opens_a_scene, timeout=_EDITOR_TIMEOUT_MS
        ):
            entry.click()
    except PlaywrightTimeoutError as timeout:
        _report_a_scene_added_unseen(probe, before)
        msg = "adding a combined scene opened no editor, so the editor went unseen"
        raise EditorUnreached(msg) from timeout
    return opened[0]


def _opened_scene(url: str) -> str | None:
    """The path of the scene editor at `url`, or none for any other page."""
    path = urlsplit(url).path
    prefix = f"{DISPLAY_PATH}/"
    scene = path.removeprefix(prefix)
    return path if path.startswith(prefix) and scene and "/" not in scene else None


def _listed_scenes(probe: Probe) -> set[str]:
    """The ids of the scenes the display list shows, asked only of a list that loaded."""
    probe.page.locator(_LIST_LOADED).wait_for(timeout=_EDITOR_TIMEOUT_MS)
    rows = probe.page.locator(f'[id^="{_SCENE_ROW}"][id$="{_DELETE_SUFFIX}"]').all()
    ids = (row.get_attribute("id") or "" for row in rows)
    return {id_.removeprefix(_SCENE_ROW).removesuffix(_DELETE_SUFFIX) for id_ in ids}


def _report_a_scene_added_unseen(probe: Probe, before: set[str]) -> None:
    """Refuse a list showing more than `before` after an editor that never opened.
    The owner can add a scene too and nothing ties a new row to the walk, so none is deleted."""
    left = "adding a combined scene opened no editor"
    try:
        probe.visit(DISPLAY_PATH)
        added = _listed_scenes(probe) - before
    except (PlaywrightError, Unreachable) as error:
        raise SceneLeft(f"{left}: {str(error).splitlines()[0]}") from error
    if added:
        paths = ", ".join(f"{DISPLAY_PATH}/{scene}" for scene in sorted(added))
        raise SceneLeft(f"{left}: the list also shows {paths} now")


def _is_remove_scene(response: Response) -> bool:
    return response.url.endswith(_REMOVE_SCENE)


def _delete_scene(probe: Probe, scene: str) -> None:
    """Delete the scene at path `scene` from the display list, which needs no preview of its own.
    The list drops a row before the device answers and shows no rows when it fails to load,
    so gone means absent from a reloaded list that shows it loaded, having been listed before.
    A row never listed says nothing of the scene, as a renamed row id would look the same."""
    page = probe.page
    row = page.locator(
        f'[id="{_SCENE_ROW}{scene.removeprefix(f"{DISPLAY_PATH}/")}{_DELETE_SUFFIX}"]'
    )
    reason = "still listed"
    listed = False
    # The last pass only looks, so that no click goes without a reloaded list to judge it.
    for clicks_left in range(_DELETE_ATTEMPTS, -1, -1):
        try:
            probe.visit(DISPLAY_PATH)
            page.locator(_LIST_LOADED).wait_for(timeout=_DELETE_STEP_MS)
            try:
                row.wait_for(state="detached", timeout=_DELETE_RETRY_MS)
            except PlaywrightTimeoutError:
                listed = True
                reason = "still listed"
            else:
                if listed:
                    return
                raise SceneLeft(f"deleting {scene} failed: never listed")
            if clicks_left > 0:
                with page.expect_response(_is_remove_scene, timeout=_EDITOR_TIMEOUT_MS):
                    row.click(timeout=_DELETE_STEP_MS)
        except (PlaywrightError, Unreachable) as error:
            reason = str(error).splitlines()[0]
    raise SceneLeft(f"deleting {scene} failed: {reason}")


def shoot_login(probe: Probe, out: Path) -> Path | None:
    """The login page shot whole into `out`, in the device's language,
    or none from a device without a password, which shows no login page.
    Shot from a session of its own, as `speaking` can only set the language back while logged in."""
    with probe.logged_out() as guest:
        guest.visit("/")
        if guest.path() != LOGIN_PATH:
            return None
        return guest.shot(out / shot_name(LOGIN_PATH))
