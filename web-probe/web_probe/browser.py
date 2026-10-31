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

"""A browser session over a web UI: the pages it visits, the shots it takes, what it logged."""

import os
from collections import deque
from collections.abc import Callable, Container, Generator, Iterable, Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import cast

from playwright.sync_api import ConsoleMessage, Page, sync_playwright

# Unset, Playwright looks in its own cache and asks for `playwright install`;
# the browsers come from nix instead.
_BROWSERS = "PLAYWRIGHT_BROWSERS_PATH"

# Each anchor's `href` as the browser resolved it, kept when it stays on the page's origin:
# what counts as in-app is decided by the parser that navigates.
_IN_APP_PATHS = """anchors => anchors
    .map(anchor => new URL(anchor.href))
    .filter(url => url.origin === location.origin)
    .map(url => url.pathname)"""


def page_string(value: object) -> str:
    """`value`, what a page script returned, as the string it was written to return."""
    if not isinstance(value, str):
        raise TypeError(f"a page script returned {value!r}, not a string")
    return value


def page_strings(value: object) -> list[str]:
    """`value`, what a page script returned, as the list of strings it was written to return."""
    if not isinstance(value, list):
        raise TypeError(f"a page script returned {value!r}, not a list of strings")
    items = cast("list[object]", value)
    return [page_string(item) for item in items]


@dataclass(frozen=True)
class ConsoleEntry:
    """A message the page logged: its kind (`log`, `warning`, `error`, …) and its text."""

    kind: str
    text: str


class Probe:
    """One page of a browser, opened on a web UI's origin."""

    def __init__(self, page: Page, origin: str) -> None:
        self.page: Page = page
        self._origin: str = origin.rstrip("/")
        self._console: list[ConsoleEntry] = []
        page.on("console", self._keep)

    def _keep(self, message: ConsoleMessage) -> None:
        self._console.append(ConsoleEntry(message.type, message.text))

    def visit(self, path: str) -> None:
        """Open `path` on the origin and wait until the page stops loading."""
        _ = self.page.goto(f"{self._origin}{path}")
        self.page.wait_for_load_state("networkidle")

    def path(self) -> str:
        """The path the page is on, as the browser parsed its URL."""
        return page_string(cast("object", self.page.evaluate("() => location.pathname")))

    def shot(self, file: Path) -> Path:
        """The whole page, scrolled length included, as a PNG at `file`."""
        file.parent.mkdir(parents=True, exist_ok=True)
        _ = self.page.screenshot(path=file, full_page=True)
        return file

    def links(self) -> list[str]:
        """The paths the current page links to on its own origin."""
        anchors = self.page.locator("a[href]")
        return page_strings(cast("object", anchors.evaluate_all(_IN_APP_PATHS)))

    def navigation_paths(self) -> list[str]:
        """Where each anchor in the page's navigation landmarks leads on its own origin,
        found by clicking it: one a script navigates by has no `href` to read,
        and without one no link role either, so it is found as an anchor."""
        start = self.path()
        origin = page_string(cast("object", self.page.evaluate("() => location.origin")))
        entries = self.page.get_by_role("navigation").locator("a")
        paths: list[str] = []
        for index in range(entries.count()):
            entries.nth(index).click()
            self.page.wait_for_load_state("networkidle")
            here = page_string(cast("object", self.page.evaluate("() => location.origin")))
            if here == origin:
                paths.append(self.path())
            self.visit(start)
        return paths

    def console(self) -> list[ConsoleEntry]:
        """Everything the page has logged since the probe opened."""
        return list(self._console)


@contextmanager
def open_probe(origin: str, *, width: int = 1280, height: int = 800) -> Generator[Probe]:
    """A headless Chromium on `origin`, closed on leaving the block.

    The browsers are nixpkgs' `playwright-driver.browsers`:
    `just web-probe::browsers` prints the path `PLAYWRIGHT_BROWSERS_PATH` must name.
    """
    if not os.environ.get(_BROWSERS):
        raise RuntimeError(f"{_BROWSERS} is unset; `just web-probe::browsers` prints its value")
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        try:
            page = browser.new_page(viewport={"width": width, "height": height})
            yield Probe(page, origin)
        finally:
            browser.close()


def crawl(start: str, links: Callable[[str], Iterable[str]], skip: Container[str]) -> Iterator[str]:
    """Every path reachable from `start` through `links`, each once, nearest first.

    `links` is asked about a path just before it is yielded, so whatever it opened
    is still open for the caller; a path in `skip` is neither yielded nor followed.
    Siblings come in sorted order, so two walks of the same UI visit its pages in
    the same order.
    """
    seen = {start}
    queue = deque([start])
    while queue:
        path = queue.popleft()
        found_here = sorted(set(links(path)))
        yield path
        for found in found_here:
            if found not in seen and found not in skip:
                seen.add(found)
                queue.append(found)
