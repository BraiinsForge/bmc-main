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

"""A scripted stand-in for Boser's REST upgrade API, speaking real HTTP on localhost."""

import json
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import Any, Self

TOKEN = "stub-token"
OFFER_ID = "9e2c7c2e-0000-4000-8000-00000000abcd"

CHECK_OFFER = {
    "offer": {"id": OFFER_ID, "kind": "PACKAGES", "disruption": "NONE"},
    "firmware": None,
    "packages": {
        "changes": [
            {
                "name": "weather",
                "version_from": None,
                "version_to": "1.2.0",
                "category": "widget",
                "changelog": None,
            }
        ],
        "download_size_bytes": 1_048_576,
        "unpacked_size_bytes": 2_097_152,
        "bmc_version": None,
        "bmc_changelog": None,
    },
    "package_capability": {"status": "READY"},
}

CHECK_NOTHING = {
    "offer": None,
    "firmware": None,
    "packages": None,
    "package_capability": {"status": "READY"},
}

RUNNING = {"state": "RUNNING", "id": OFFER_ID, "kind": "PACKAGES", "phase": {"stage": "PREPARING"}}
REALIZING = {
    "state": "RUNNING",
    "id": OFFER_ID,
    "kind": "PACKAGES",
    "phase": {"stage": "PACKAGES", "step": "REALIZING"},
    "download": {"downloaded_bytes": 524_288, "total_bytes": 1_048_576},
}
COMPLETED = {"state": "COMPLETED", "id": OFFER_ID, "kind": "PACKAGES"}
REBOOTING = {"state": "REBOOTING", "id": OFFER_ID, "kind": "FIRMWARE_AND_PACKAGES"}
CHECK_FIRMWARE = {
    **CHECK_OFFER,
    "offer": {"id": OFFER_ID, "kind": "FIRMWARE_AND_PACKAGES", "disruption": "REBOOT"},
    "firmware": {
        "version": "2026-09-01-0-0badc0de-26.09-plus",
        "hash": "0badc0de",
        "release_date": "2026-09-01",
        "description": "",
        "file_size_bytes": 50_094_912,
        "previous_releases": [],
    },
}
FAILED = {
    "state": "FAILED",
    "id": OFFER_ID,
    "kind": "PACKAGES",
    "phase": {"stage": "PACKAGES", "step": "ACTIVATING"},
    "reason": "activation script exited 1",
}


@dataclass
class Script:
    password: str = ""
    check: dict[str, Any] = field(default_factory=lambda: dict(CHECK_OFFER))
    recheck: dict[str, Any] = field(default_factory=lambda: dict(CHECK_NOTHING))  # after a start
    state: dict[str, Any] = field(default_factory=lambda: {"state": "NONE"})
    before_terminal: Callable[[], None] = lambda: None  # the device changes while the stream runs
    start_status: int = 204  # 409 BUSY for a run that already holds admission
    stall_seconds: float = 0.0  # every reply waits this long, like a Boser still starting
    events: list[dict[str, Any]] = field(default_factory=lambda: [RUNNING, REALIZING, COMPLETED])
    requests: list[tuple[str, str, str | None, Any]] = field(default_factory=list)


class _Handler(BaseHTTPRequestHandler):
    server: "BoserStub"

    def log_message(self, format: str, *args: object) -> None:  # noqa: A002
        pass

    def _body(self) -> Any:
        length = int(self.headers.get("Content-Length", "0"))
        return json.loads(self.rfile.read(length)) if length else None

    def _reply(self, status: int, body: Any = None) -> None:
        payload = json.dumps(body).encode() if body is not None else b""
        self.send_response(status)
        if payload:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def _error(self, status: int, code: str, message: str) -> None:
        self._reply(status, {"error": code, "message": message})

    def _handle(self) -> None:  # noqa: PLR0911
        script = self.server.script
        body = self._body()
        token = self.headers.get("Authorization")
        script.requests.append((self.command, self.path, token, body))
        time.sleep(script.stall_seconds)
        if self.path == "/api/v1/auth/login":
            if body != {"username": "root", "password": script.password}:
                return self._error(401, "UNAUTHORIZED", "wrong password")
            return self._reply(200, {"token": TOKEN, "timeout_s": 3600})
        if token != TOKEN:
            return self._error(401, "UNAUTHORIZED", "missing token")
        if self.path == "/api/v1/upgrade/packages/installable":
            return self._reply(
                200,
                {
                    "packages": [
                        {
                            "name": "weather",
                            "version": "1.2.0",
                            "category": "widget",
                            "description": None,
                            "metadata": {},
                        }
                    ]
                },
            )
        if self.path == "/api/v1/upgrade/check":
            started = any(path == "/api/v1/upgrade/start" for _m, path, _t, _b in script.requests)
            return self._reply(200, script.recheck if started else script.check)
        if self.path == "/api/v1/upgrade/state":
            return self._reply(200, script.state)
        if self.path == "/api/v1/upgrade/start":
            if body != {"offer_id": OFFER_ID}:
                return self._error(404, "EXPIRED", "unknown offer")
            if script.start_status != 204:
                return self._error(script.start_status, "BUSY", "an upgrade is running")
            return self._reply(204)
        if self.path == "/api/v1/upgrade/state/events":
            return self._stream(script)
        return self._error(404, "NOT_FOUND", self.path)

    def _stream(self, script: Script) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        self.wfile.write(b": keepalive\n\n")
        for event in script.events:
            if event.get("state") in ("COMPLETED", "REBOOTING"):
                script.before_terminal()
            self.wfile.write(f"data: {json.dumps(event)}\n\n".encode())
            self.wfile.flush()

    def do_GET(self) -> None:
        self._handle()

    def do_POST(self) -> None:
        self._handle()


class BoserStub(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, script: Script) -> None:
        super().__init__(("127.0.0.1", 0), _Handler)
        self.script = script
        self._thread = threading.Thread(target=self.serve_forever, daemon=True)

    @property
    def address(self) -> str:
        return f"127.0.0.1:{self.server_port}"

    def __enter__(self) -> Self:
        self._thread.start()
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: TracebackType | None,
    ) -> None:
        self.shutdown()
        self.server_close()
        self._thread.join()
