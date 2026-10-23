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

"""HTTP requests against the servers the tests start on localhost."""

import http.client
import time
import urllib.request
from typing import cast

from bmc_tui.fw_index import FwIndexServer


def open_url(
    url: str | urllib.request.Request, *, timeout: float | None = None
) -> http.client.HTTPResponse:
    """`urlopen`, typed: typeshed types its result as Any; for http(s) it is an HTTPResponse."""
    if timeout is None:
        return cast("http.client.HTTPResponse", urllib.request.urlopen(url))
    return cast("http.client.HTTPResponse", urllib.request.urlopen(url, timeout=timeout))


def wait_for_requests(server: FwIndexServer, count: int) -> None:
    """Wait for `server` to record `count` requests.

    It records a request once the response is sent,
    so the client can have read it all before the record exists.
    """
    deadline = time.monotonic() + 5
    while len(server.requests()) < count and time.monotonic() < deadline:
        time.sleep(0.01)
    assert len(server.requests()) == count
