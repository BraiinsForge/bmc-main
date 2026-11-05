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
from urllib.parse import SplitResult, urlsplit

from playwright.sync_api import ConsoleMessage, Page, sync_playwright
from playwright.sync_api import Error as PlaywrightError

# Unset, Playwright looks in its own cache and asks for `playwright install`.
# The browsers come from nix instead.
_BROWSERS = "PLAYWRIGHT_BROWSERS_PATH"

# Each anchor's `href` as the browser resolved it, which no Python call returns:
# the raw attribute leaves relative links unresolved,
# and the browser's parser is the one that navigates.
# The URLs come back serialised by it, so splitting them cannot disagree with it.
_RESOLVED_HREFS = "anchors => anchors.map(anchor => anchor.href)"

_WAITING_FOR = "waiting for "
_WAITING_FOR_ELEMENT = "waiting for element"
_RESOLVED_TO = " resolved to "
_NOT_READY = "element is not "
_BLOCKS = " intercepts pointer events"


def _same_origin(url: SplitResult, other: SplitResult) -> bool:
    return (url.scheme, url.netloc) == (other.scheme, other.netloc)


def _strings(value: object) -> list[str]:
    """`value`, what a page script returned, as the list of strings it was written to return."""
    items = cast("list[object]", value) if isinstance(value, list) else None
    if items is None or not all(isinstance(item, str) for item in items):
        raise TypeError(f"a page script returned {value!r}, not a list of strings")
    return cast("list[str]", items)


class Unreachable(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """A page of the web UI did not load: nothing answered, or not in time."""


class BrowsersMissing(Exception):  # noqa: N818  names what happened, as `TimeoutError` does
    """`PLAYWRIGHT_BROWSERS_PATH` is unset, so no browser can be launched."""


@dataclass(frozen=True)
class ConsoleEntry:
    """A message the page logged: its kind (`log`, `warning`, `error`, …) and its text."""

    kind: str
    text: str


@dataclass(frozen=True)
class Failure:
    """A browser failure read from Playwright's message: its reason and its call log,
    with what the action waited for, what that resolved to, the states that kept it waiting
    and what stood in its way, where the log says."""

    reason: str
    log: list[str]
    waiting_for: str | None
    resolved_to: str | None
    states: list[str]
    blocked_by: list[str]


def read_failure(message: str) -> Failure:
    """`message` as a `Failure`: its first line is the failure, the rest the retries' call log,
    which repeats itself per retry, so each state and blocker comes once."""
    reason, *log = message.splitlines()
    steps = [line.strip().removeprefix("- ") for line in log]
    waits = (
        step.removeprefix(_WAITING_FOR)
        for step in steps
        if step.startswith(_WAITING_FOR) and not step.startswith(_WAITING_FOR_ELEMENT)
    )
    resolved = (step.split(_RESOLVED_TO)[1] for step in steps if _RESOLVED_TO in step)
    states = (step for step in steps if step.startswith(_NOT_READY))
    blockers = (step.removesuffix(_BLOCKS) for step in steps if step.endswith(_BLOCKS))
    return Failure(
        reason=reason,
        log=log,
        waiting_for=next(waits, None),
        resolved_to=next(resolved, None),
        states=list(dict.fromkeys(states)),
        blocked_by=list(dict.fromkeys(blockers)),
    )


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
        url = f"{self._origin}{path}"
        try:
            _ = self.page.goto(url)
        except PlaywrightError as error:
            # Its first line is the failure, the rest Playwright's call log.
            reason = error.message.splitlines()[0]
            raise Unreachable(f"{url} did not load: {reason}") from error
        self.page.wait_for_load_state("networkidle")

    def path(self) -> str:
        return urlsplit(self.page.url).path

    def shot(self, file: Path) -> Path:
        """The whole page, scrolled length included, as a PNG at `file`."""
        file.parent.mkdir(parents=True, exist_ok=True)
        _ = self.page.screenshot(path=file, full_page=True, animations="disabled")
        return file

    def links(self) -> list[str]:
        """The paths the current page links to on its own origin."""
        here = urlsplit(self.page.url)
        anchors = self.page.locator("a[href]")
        hrefs = _strings(cast("object", anchors.evaluate_all(_RESOLVED_HREFS)))
        return [url.path for url in map(urlsplit, hrefs) if _same_origin(url, here)]

    def navigation_paths(self) -> list[str]:
        """Where each anchor without an `href` in the page's navigation landmarks leads
        on its own origin, found by clicking it: a script navigates by it,
        and without an `href` it has no link role either, so it is found as an anchor.
        One with an `href` is for `links` to read, never clicked, as it may lead off the device."""
        start = urlsplit(self.page.url)
        entries = self.page.get_by_role("navigation").locator("a:not([href])")
        paths: list[str] = []
        for index in range(entries.count()):
            entries.nth(index).click()
            self.page.wait_for_load_state("networkidle")
            landed = urlsplit(self.page.url)
            if _same_origin(landed, start):
                paths.append(landed.path)
            self.visit(start.path)
        return paths

    def console(self) -> list[ConsoleEntry]:
        """Everything the page has logged since the probe opened."""
        return list(self._console)

    @contextmanager
    def logged_out(self) -> Generator["Probe"]:
        """A probe in a browser context of its own, with no session, this one's left untouched.
        What it logs joins this probe's."""
        browser = self.page.context.browser
        if browser is None:
            raise RuntimeError("BUG: `open_probe` always opens its page from a launched browser")
        page = browser.new_page(viewport=self.page.viewport_size)
        guest = Probe(page, self._origin)
        try:
            yield guest
        finally:
            self._console.extend(guest.console())
            page.context.close()


@contextmanager
def open_probe(origin: str, *, width: int = 1280, height: int = 800) -> Generator[Probe]:
    """A headless Chromium on `origin`, closed on leaving the block.

    The browsers are nixpkgs' `playwright-driver.browsers`:
    `just web-probe::browsers` prints the path `PLAYWRIGHT_BROWSERS_PATH` must name.
    """
    if not os.environ.get(_BROWSERS):
        raise BrowsersMissing(f"{_BROWSERS} is unset, `just web-probe::browsers` prints its value")
    with sync_playwright() as playwright:
        # A walk's Ctrl-C still has to set the device's language back, which takes the browser.
        browser = playwright.chromium.launch(handle_sigint=False)
        try:
            page = browser.new_page(viewport={"width": width, "height": height})
            yield Probe(page, origin)
        finally:
            browser.close()


def crawl(start: str, links: Callable[[str], Iterable[str]], skip: Container[str]) -> Iterator[str]:
    """Every path reachable from `start` through `links`, each once, nearest first.

    `links` is asked about a path just before it is yielded, so the caller sees what it opened.
    A path in `skip` is neither yielded nor followed.
    Siblings come sorted, so two walks of the same UI visit its pages in the same order.
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
