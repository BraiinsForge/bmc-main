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

"""Unit tests for walking a UI's pages through its links."""

from web_probe.browser import crawl

# A UI shaped like the device's: a shell linking every page, one page linking deeper.
LINKS = {
    "/": ["/settings", "/display", "/alarms", "/login"],
    "/display": ["/display/1", "/display/2", "/"],
    "/settings": ["/", "/settings"],
    "/alarms": ["/"],
    "/display/1": ["/display"],
    "/display/2": ["/display"],
}


def test_every_reachable_page_comes_once_nearest_first_in_sorted_order() -> None:
    walked = list(crawl("/", LINKS.__getitem__, skip={"/login"}))

    assert walked == ["/", "/alarms", "/display", "/settings", "/display/1", "/display/2"]


def test_a_skipped_page_is_neither_yielded_nor_asked_about() -> None:
    asked: list[str] = []

    def links(path: str) -> list[str]:
        asked.append(path)
        return LINKS[path]

    walked = list(crawl("/", links, skip={"/login", "/display"}))

    assert "/display" not in walked
    assert "/display" not in asked
    assert "/display/1" not in walked, "nothing else links to it"


def test_a_page_is_open_while_its_path_is_yielded() -> None:
    opened: list[str] = []

    def links(path: str) -> list[str]:
        opened.append(path)
        return LINKS[path]

    for path in crawl("/", links, skip={"/login"}):
        assert opened[-1] == path, "the caller sees the page `links` just opened"
