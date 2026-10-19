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

"""Boser's REST upgrade API: login, auto-upgrade, check, start and the state event stream.

The reply shapes mirror the BMC's own client types in `bmc-upgrade/types`.
Enumerations stay `str`, so a state or kind added on the device does not fail the decode.
"""

import http.client
import json
import time
import urllib.error
import urllib.request
from collections.abc import Callable, Iterator
from dataclasses import dataclass, field
from typing import NotRequired, TypedDict, cast

import msgspec

from bmc_tui.stage import Abort

TERMINAL_STATES = frozenset({"COMPLETED", "REBOOTING", "FAILED"})
_STREAM_PATH = "/api/v1/upgrade/state/events"


class Offer(TypedDict):
    id: str
    kind: str
    disruption: str


class PreviousRelease(TypedDict):
    version: str
    description: str


class FirmwareUpgrade(TypedDict):
    version: str
    hash: str
    release_date: str
    description: str
    file_size_bytes: int
    previous_releases: list[PreviousRelease]


class PackageChange(TypedDict):
    name: str
    version_from: str | None
    version_to: str | None
    category: str | None
    changelog: str | None


class PackagesPreview(TypedDict):
    changes: list[PackageChange]
    download_size_bytes: int | None
    unpacked_size_bytes: int | None
    bmc_version: str | None
    bmc_changelog: str | None


class PackageCapability(TypedDict):
    status: str
    reason: NotRequired[str]


class CheckUpgradeResponse(TypedDict):
    offer: Offer | None
    firmware: FirmwareUpgrade | None
    packages: PackagesPreview | None
    package_capability: PackageCapability


class InstallablePackage(TypedDict):
    name: str
    version: str
    category: str | None
    description: str | None
    metadata: dict[str, object]


class UpgradePhase(TypedDict):
    stage: str
    # Boser accepts any JSON `step` for a stage it does not know.
    step: NotRequired[object]


class DownloadProgress(TypedDict):
    downloaded_bytes: int
    total_bytes: int | None


class UpgradeState(TypedDict):
    """Flat over Boser's tagged union: which keys a snapshot carries depends on `state`."""

    state: str
    id: NotRequired[str]
    kind: NotRequired[str]
    phase: NotRequired[UpgradePhase]
    download: NotRequired[DownloadProgress]
    reason: NotRequired[str]


class Login(TypedDict):
    token: str


class InstallablePackages(TypedDict):
    packages: list[InstallablePackage]


class ErrorBody(TypedDict):
    error: str
    message: str


@dataclass
class BoserRest:
    base_url: str
    password: str = field(default="", repr=False)
    timeout: float = 30.0  # per request; the event stream carries its own deadline
    token: str | None = field(default=None, repr=False)  # an existing session; `login` replaces it

    def login(self) -> None:
        body: dict[str, object] = {"username": "root", "password": self.password}
        self.token = self._json("POST", "/api/v1/auth/login", body, reply=Login)["token"]

    def set_auto_upgrade(self, *, enabled: bool) -> None:
        self._send("PATCH", "/api/v1/upgrade/auto-upgrade", {"enabled": enabled})

    def check(self, packages: list[str]) -> CheckUpgradeResponse:
        path = "/api/v1/upgrade/check"
        return self._json("POST", path, {"packages": packages}, reply=CheckUpgradeResponse)

    def installable_packages(self) -> list[InstallablePackage]:
        path = "/api/v1/upgrade/packages/installable"
        return self._json("GET", path, reply=InstallablePackages)["packages"]

    def start(self, offer_id: str) -> None:
        self._send("POST", "/api/v1/upgrade/start", {"offer_id": offer_id})

    def state(self) -> UpgradeState:
        return self._json("GET", "/api/v1/upgrade/state", reply=UpgradeState)

    def events(
        self, *, deadline: float, clock: Callable[[], float] = time.monotonic
    ) -> Iterator[UpgradeState]:
        """Yield each snapshot the stream sends, ending after the first terminal one."""
        # Boser sends a keepalive comment every 15 s, so a quiet socket for `timeout` is a dead one.
        try:
            with self._open("GET", _STREAM_PATH) as response:
                for raw in response:
                    line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
                    if line.startswith("data:"):
                        data = line.removeprefix("data:")
                        snapshot = _decode(data, UpgradeState, f"GET {_STREAM_PATH}")
                        yield snapshot
                        if snapshot["state"] in TERMINAL_STATES:
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

    def _open(
        self, method: str, path: str, body: dict[str, object] | None = None
    ) -> http.client.HTTPResponse:
        headers = {"Accept": "application/json"}
        if self.token is not None:
            headers["Authorization"] = self.token
        data = None
        if body is not None:
            headers["Content-Type"] = "application/json"
            data = json.dumps(body).encode()
        request = urllib.request.Request(
            self.base_url + path, data=data, headers=headers, method=method
        )
        # typeshed types `urlopen` as Any; for an http(s) URL it is an HTTPResponse.
        return cast(
            "http.client.HTTPResponse", urllib.request.urlopen(request, timeout=self.timeout)
        )

    def _send(self, method: str, path: str, body: dict[str, object] | None = None) -> bytes:
        try:
            with self._open(method, path, body) as response:
                return response.read()
        except urllib.error.HTTPError as error:
            raise Abort(_describe(error, method, path)) from error
        except urllib.error.URLError as error:
            raise Abort(f"{method} {path}: {error.reason}") from error
        except TimeoutError as error:
            raise Abort(f"{method} {path}: the device went quiet") from error

    def _json[T](
        self, method: str, path: str, body: dict[str, object] | None = None, *, reply: type[T]
    ) -> T:
        return _decode(self._send(method, path, body), reply, f"{method} {path}")


def _decode[T](payload: bytes | str, reply: type[T], what: str) -> T:
    try:
        return msgspec.json.decode(payload, type=reply)
    except msgspec.DecodeError as error:
        raise Abort(f"{what}: unexpected reply: {error}: {payload[:80]!r}") from error


def _describe(error: urllib.error.HTTPError, method: str, path: str) -> str:
    detail = error.read().decode("utf-8", errors="replace")
    try:
        body = msgspec.json.decode(detail, type=ErrorBody)
        detail = f"{body['error']}: {body['message']}"
    except msgspec.DecodeError:
        pass
    return f"{method} {path}: HTTP {error.code} {detail}".rstrip()
