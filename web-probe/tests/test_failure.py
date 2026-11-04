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

"""Unit tests for reading a browser failure out of Playwright's message."""

from pathlib import Path
from typing import cast

import pytest

from web_probe.__main__ import _report_failure
from web_probe.browser import ConsoleEntry, Probe, Unreachable, read_failure

# Shortened from what Playwright 1.64 printed for a tab that could not be clicked.
BLOCKED_CLICK = """Page.click: Timeout 30000ms exceeded.
Call log:
  - waiting for locator("[data-tab]:enabled[data-tab=\\"display\\"]")
    - locator resolved to <button type="button" data-tab="display">…</button>
  - attempting click action
    2 × waiting for element to be visible, enabled and stable
      - element is visible, enabled and stable
      - <div class="tabs">…</div> intercepts pointer events
    - retrying click action
    - <div class="tabs">…</div> intercepts pointer events
  2 × retrying click action
      - <div class="global">…</div> from <header>…</header> subtree intercepts pointer events
"""  # noqa: RUF001  the multiplication sign is Playwright's own


DISABLED_CLICK = """Page.click: Timeout 30000ms exceeded.
Call log:
  - waiting for get_by_role("navigation").locator("#off").nth(0)
    - locator resolved to <button id="off" disabled>…</button>
  - attempting click action
    2 × waiting for element to be visible, enabled and stable
      - element is not enabled
    - retrying click action
    - element is not enabled
"""  # noqa: RUF001  the multiplication sign is Playwright's own

ABSENT_CLICK = """Page.click: Timeout 30000ms exceeded.
Call log:
  - waiting for locator("#gone")
"""


class Reported:
    """A probe as far as reporting a failure goes."""

    def console(self) -> list[ConsoleEntry]:
        return [ConsoleEntry("log", "not an error"), ConsoleEntry("error", "it broke")]


def report(error: Exception, capsys: pytest.CaptureFixture[str]) -> str:
    """What the failure's report said on stderr, having said nothing on stdout."""
    _report_failure(cast(Probe, cast(object, Reported())), error, Path("out/last.png"))
    said = capsys.readouterr()
    assert said.out == "", "a redirect must not part the failure from its details"
    return said.err


def test_the_whole_report_goes_to_stderr_with_how_far_the_walk_got(
    capsys: pytest.CaptureFixture[str],
) -> None:
    said = report(Unreachable("http://mock/ did not load: refused"), capsys)

    assert "the walk failed" in said
    assert "did not load" in said
    assert "out/last.png" in said
    assert "console errors: 1" in said
    assert "it broke" in said
    assert "not an error" not in said


def test_a_report_tells_a_missing_target_from_a_disabled_one(
    capsys: pytest.CaptureFixture[str],
) -> None:
    absent = report(RuntimeError(ABSENT_CLICK), capsys)
    disabled = report(RuntimeError(DISABLED_CLICK), capsys)

    assert "nothing" in absent
    assert "element is not enabled" in disabled
    assert "nothing" not in disabled


def test_a_blocked_click_names_what_it_waited_for() -> None:
    failure = read_failure(BLOCKED_CLICK)

    assert failure.reason == "Page.click: Timeout 30000ms exceeded."
    assert failure.waiting_for == 'locator("[data-tab]:enabled[data-tab=\\"display\\"]")'
    assert failure.resolved_to == '<button type="button" data-tab="display">…</button>'


def test_a_disabled_click_keeps_its_state_once_whatever_the_retries_and_locator_kind() -> None:
    failure = read_failure(DISABLED_CLICK)

    assert failure.waiting_for == 'get_by_role("navigation").locator("#off").nth(0)'
    assert failure.states == ["element is not enabled"]


def test_an_absent_target_resolved_to_nothing() -> None:
    assert read_failure(ABSENT_CLICK).resolved_to is None


def test_each_blocker_comes_once_however_many_retries_met_it() -> None:
    assert read_failure(BLOCKED_CLICK).blocked_by == [
        '<div class="tabs">…</div>',
        '<div class="global">…</div> from <header>…</header> subtree',
    ]


def test_a_message_without_a_call_log_is_just_its_reason() -> None:
    failure = read_failure("Page.goto: net::ERR_CONNECTION_REFUSED")

    assert failure.reason == "Page.goto: net::ERR_CONNECTION_REFUSED"
    assert failure.log == []
    assert failure.waiting_for is None
    assert failure.blocked_by == []


def test_a_log_of_another_shape_is_reported_whole(capsys: pytest.CaptureFixture[str]) -> None:
    said = report(RuntimeError("Page.click: gone\nCall log:\n  - the page crashed"), capsys)

    assert "the page crashed" in said
