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
which no language changes, never by their wording.
"""

from collections.abc import Iterator
from typing import cast

from web_probe.browser import Probe, crawl, page_strings

LOGIN_PATH = "/login"
SETTINGS_PATH = "/settings"

_PASSWORD = "#login-password"
_SUBMIT = "#login-submit"
_LANGUAGE = "#settings-general-language"


def log_in(probe: Probe, password: str) -> None:
    """Pass the login page if the device shows one; a device without a password shows none."""
    probe.visit("/")
    if probe.path() != LOGIN_PATH:
        return
    probe.page.fill(_PASSWORD, password)
    probe.page.click(_SUBMIT)
    probe.page.wait_for_url(lambda url: not url.endswith(LOGIN_PATH))
    probe.page.wait_for_load_state("networkidle")


def offered_languages(probe: Probe) -> list[str]:
    """The tags the language picker offers; none when the device offers no languages,
    as one without its i18n marker (`/etc/bmc/i18n`) does and then shows no picker."""
    probe.visit(SETTINGS_PATH)
    picker = probe.page.locator(_LANGUAGE)
    if picker.count() == 0:
        return []
    picker.click()
    options = probe.page.locator('[role="option"] [lang]')
    tags = page_strings(
        cast("object", options.evaluate_all("options => options.map(option => option.lang)"))
    )
    probe.page.keyboard.press("Escape")
    return tags


def current_language(probe: Probe) -> str | None:
    """The tag the language picker shows as the device's; none without a picker."""
    probe.visit(SETTINGS_PATH)
    shown = probe.page.locator(f"{_LANGUAGE} [lang]")
    return shown.first.get_attribute("lang") if shown.count() else None


def set_language(probe: Probe, tag: str) -> None:
    """Pick `tag` in the language picker and wait until the page speaks it."""
    probe.visit(SETTINGS_PATH)
    probe.page.click(_LANGUAGE)
    probe.page.click(f'[role="option"] [lang="{tag}"]')
    _ = probe.page.wait_for_selector(f'html[lang="{tag}"]', state="attached")


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
