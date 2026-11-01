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
by their roles and by ARIA relations, which no language changes, never by their wording.
"""

import sys
from collections.abc import Generator, Iterator
from contextlib import contextmanager
from pathlib import Path

from playwright.sync_api import Error as PlaywrightError
from playwright.sync_api import Locator
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

# What the login form shows when the device refuses: an error notification for the form,
# or the password field marked invalid. Either describes the reason through `aria-describedby`.
_REFUSAL = f'form [role="alertdialog"], {_PASSWORD}[aria-invalid="true"]'


class LoginRefused(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """The login form showed the device's refusal."""


class LoginStalled(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """The login page neither left nor showed a refusal after the password was submitted."""


class LanguageUnknown(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """The language picker shows none of the device's languages as its own."""


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
        # One from setting the language back must not replace it.
        try:
            set_language(probe, before)
        except (PlaywrightError, Unreachable) as error:
            print(f"could not set the language back to {before}: {error}", file=sys.stderr)
        raise
    set_language(probe, before)


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
    """The file a page's shot is saved as: `/display/1` as `display_1.png`, `/` as `home.png`."""
    return f"{path.strip('/').replace('/', '_') or 'home'}.png"


def shoot_pages(probe: Probe, out: Path) -> Iterator[tuple[str, Path]]:
    """Every page `pages` reaches, shot whole into `out`, with the path it was shot at."""
    for path in pages(probe):
        yield path, probe.shot(out / shot_name(path))


def shoot_login(probe: Probe, out: Path, password: str) -> Path | None:
    """The login page shot whole into `out`, in the device's language, logged in again after,
    or none from a device without a password, which shows no login page.
    The session is the browser's cookie alone, so dropping it is a logout no wording decides."""
    probe.page.context.clear_cookies()
    probe.visit("/")
    if probe.path() != LOGIN_PATH:
        return None
    shot = probe.shot(out / shot_name(LOGIN_PATH))
    log_in(probe, password)
    return shot
