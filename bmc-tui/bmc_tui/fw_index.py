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

"""Firmware release index generation and request-recording HTTP serving."""

import io
import json
import threading
import uuid
from collections.abc import Callable
from dataclasses import dataclass
from functools import partial
from http import HTTPStatus
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from socketserver import BaseRequestHandler
from typing import cast

from bmc_tui.bos_version import BosVersion

INDEX_NAME = "index.v1.json"
PLATFORM_ASSET_KEY = "sysupgrade_emmc_stm32mp157c_ii3_bmc1"


def release_uuid(full_version: str) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, full_version))


def index_document(  # noqa: PLR0913
    *,
    running: BosVersion,
    image: BosVersion,
    anchor_url: str,
    image_url: str,
    image_sha256: str,
    image_size: int,
) -> str:
    running_release = _release_document(running, anchor_url)
    image_release = _release_document(
        image,
        {
            "url": image_url,
            "integrity": {
                "checksum": image_sha256.lower(),
                "size_bytes": image_size,
            },
        },
    )
    return json.dumps(
        {
            "type": "bmc",
            "status": "Active",
            "title": "BDK-609 e2e firmware releases",
            "version": "v1",
            "releases": [running_release, image_release],
        },
        indent=2,
    )


def _release_document(version: BosVersion, asset: str | dict[str, object]) -> dict[str, object]:
    return {
        "uuid": release_uuid(version.canonical),
        "metadata_version": "v1",
        "metadata": {
            "bmc_version": version.canonical,
            "is_major": False,
            "release_date": version.release_date,
            "description": "BDK-609 e2e",
            "assets": {PLATFORM_ASSET_KEY: asset},
        },
    }


@dataclass(frozen=True)
class RequestRecord:
    path: str
    status: int
    bytes_attempted: int
    bytes_written: int
    complete: bool
    error: str | None


class _RecordingWriter:
    def __init__(self, output: io.BufferedIOBase) -> None:
        self._output = output
        self.reset()

    def reset(self) -> None:
        self.recording = False
        self.bytes_attempted = 0
        self.bytes_written = 0

    @property
    def closed(self) -> bool:
        return self._output.closed

    def write(self, data: bytes) -> int:
        if self.recording:
            self.bytes_attempted += len(data)
        written = self._output.write(data)
        if self.recording:
            self.bytes_written += written
        return written

    def flush(self) -> None:
        self._output.flush()

    def close(self) -> None:
        self._output.close()


class _RecordingHTTPServer(ThreadingHTTPServer):
    def __init__(
        self,
        server_address: tuple[str, int],
        request_handler: Callable[..., BaseRequestHandler],
    ) -> None:
        self._records: list[RequestRecord] = []
        self._active_transfers = 0
        self._records_lock = threading.Lock()
        super().__init__(server_address, request_handler)

    def begin_transfer(self) -> None:
        with self._records_lock:
            self._active_transfers += 1

    def end_transfer(self) -> None:
        with self._records_lock:
            self._active_transfers -= 1

    def append_record(self, record: RequestRecord) -> None:
        with self._records_lock:
            self._records.append(record)

    def requests(self) -> list[RequestRecord]:
        with self._records_lock:
            return list(self._records)

    def active_transfers(self) -> int:
        with self._records_lock:
            return self._active_transfers


class _RecordingHandler(SimpleHTTPRequestHandler):
    server: _RecordingHTTPServer

    def setup(self) -> None:
        super().setup()
        self._body_writer = _RecordingWriter(self.wfile)
        self.wfile = cast("io.BufferedIOBase", self._body_writer)

    def do_GET(self) -> None:
        self.server.begin_transfer()
        # The writer is per-connection while records are per-request;
        # a keep-alive connection must not leak counts across requests.
        self._body_writer.reset()
        self._status = 0
        self._expected_bytes: int | None = None
        error: str | None = None
        try:
            super().do_GET()
        except OSError as exc:
            error = str(exc)
        finally:
            complete = (
                error is None
                and self._expected_bytes is not None
                and self._body_writer.bytes_written == self._expected_bytes
            )
            self.server.append_record(
                RequestRecord(
                    path=self.path,
                    status=self._status,
                    bytes_attempted=self._body_writer.bytes_attempted,
                    bytes_written=self._body_writer.bytes_written,
                    complete=complete,
                    error=error,
                )
            )
            self.server.end_transfer()

    def send_response(self, code: int, message: str | None = None) -> None:
        self._status = code
        super().send_response(code, message)

    def send_header(self, keyword: str, value: str) -> None:
        if keyword.lower() == "content-length":
            self._expected_bytes = int(value)
        super().send_header(keyword, value)

    def end_headers(self) -> None:
        super().end_headers()
        self._body_writer.recording = True


class FwIndexServer:
    """Serve a firmware index root and record each GET body transfer."""

    def __init__(self, root: Path, *, port: int, bind_ip: str = "0.0.0.0") -> None:
        handler = partial(_RecordingHandler, directory=str(root))
        self._httpd = _RecordingHTTPServer((bind_ip, port), handler)
        self._thread = threading.Thread(target=self._httpd.serve_forever, daemon=True)

    @property
    def port(self) -> int:
        return int(self._httpd.server_address[1])

    def requests(self) -> list[RequestRecord]:
        return self._httpd.requests()

    def active_transfers(self) -> int:
        return self._httpd.active_transfers()

    def completed(self, path: str) -> bool:
        return any(
            record.path == path and record.status == HTTPStatus.OK and record.complete
            for record in self.requests()
        )

    def __enter__(self) -> "FwIndexServer":
        self._thread.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self._httpd.shutdown()
        self._httpd.server_close()
