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

"""Boser's REST upgrade API: login, check, start and the state event stream."""

import json
import time
import urllib.error
import urllib.request
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from typing import Any

from bmc_tui.stage import Abort

TERMINAL_STATES = frozenset({"COMPLETED", "REBOOTING", "FAILED"})
_STREAM_PATH = "/api/v1/upgrade/state/events"


@dataclass
class BoserRest:
    base_url: str
    password: str = ""
    timeout: float = 30.0  # per request; the event stream carries its own deadline

    def __post_init__(self) -> None:
        self._token: str | None = None

    def login(self) -> None:
        body = self._json(
            "POST", "/api/v1/auth/login", {"username": "root", "password": self.password}
        )
        self._token = str(body["token"])

    def check(self, packages: list[str]) -> dict[str, Any]:
        return self._json("POST", "/api/v1/upgrade/check", {"packages": packages})

    def installable_packages(self) -> list[dict[str, Any]]:
        return list(self._json("GET", "/api/v1/upgrade/packages/installable")["packages"])

    def start(self, offer_id: str) -> None:
        self._json("POST", "/api/v1/upgrade/start", {"offer_id": offer_id})

    def state(self) -> dict[str, Any]:
        return self._json("GET", "/api/v1/upgrade/state")

    def events(
        self, *, deadline: float, clock: Callable[[], float] = time.monotonic
    ) -> Iterator[dict[str, Any]]:
        """Yield each snapshot the stream sends, ending after the first terminal one."""
        # Boser sends a keepalive comment every 15 s, so a quiet socket for `timeout` is a dead one.
        try:
            with urllib.request.urlopen(
                self._request("GET", _STREAM_PATH), timeout=self.timeout
            ) as response:
                for raw in response:
                    line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
                    if line.startswith("data:"):
                        snapshot = json.loads(line.removeprefix("data:"))
                        yield snapshot
                        if snapshot.get("state") in TERMINAL_STATES:
                            return
                    if clock() >= deadline:
                        raise Abort("upgrade state stream: no terminal state within the deadline")
        except TimeoutError as error:
            raise Abort("upgrade state stream: the device went quiet") from error
        except urllib.error.HTTPError as error:
            raise Abort(_describe(error, "GET", _STREAM_PATH)) from error
        except urllib.error.URLError as error:
            raise Abort(f"GET {_STREAM_PATH}: {error.reason}") from error
        raise Abort("upgrade state stream ended before a terminal state")

    def _request(
        self, method: str, path: str, body: dict[str, Any] | None = None
    ) -> urllib.request.Request:
        headers = {"Accept": "application/json"}
        if self._token is not None:
            headers["Authorization"] = self._token
        data = None
        if body is not None:
            headers["Content-Type"] = "application/json"
            data = json.dumps(body).encode()
        return urllib.request.Request(
            self.base_url + path, data=data, headers=headers, method=method
        )

    def _json(self, method: str, path: str, body: dict[str, Any] | None = None) -> Any:
        try:
            with urllib.request.urlopen(
                self._request(method, path, body), timeout=self.timeout
            ) as response:
                payload = response.read()
        except urllib.error.HTTPError as error:
            raise Abort(_describe(error, method, path)) from error
        except urllib.error.URLError as error:
            raise Abort(f"{method} {path}: {error.reason}") from error
        except TimeoutError as error:
            raise Abort(f"{method} {path}: the device went quiet") from error
        try:
            return json.loads(payload) if payload else None
        except json.JSONDecodeError as error:
            raise Abort(f"{method} {path}: not JSON: {payload[:80]!r}") from error


def _describe(error: urllib.error.HTTPError, method: str, path: str) -> str:
    detail = error.read().decode("utf-8", errors="replace")
    try:
        body = json.loads(detail)
        detail = f"{body['error']}: {body['message']}"
    except (ValueError, KeyError, TypeError):
        pass
    return f"{method} {path}: HTTP {error.code} {detail}".rstrip()
