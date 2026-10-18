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

"""Firmware release index: the v1 document (generation and parsing) and
request-recording HTTP serving."""

import io
import json
import threading
import uuid
from collections.abc import Buffer, Callable
from dataclasses import dataclass
from functools import partial
from http import HTTPStatus
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from socketserver import BaseRequestHandler
from typing import NotRequired, TypedDict, cast, override

import msgspec

from bmc_tui.bos_version import BosVersion

INDEX_NAME = "index.v1.json"
INDEX_VERSION = "v1"
PLATFORM_ASSET_KEY = "sysupgrade_emmc_stm32mp157c_ii3_bmc1"


def release_uuid(full_version: str) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, full_version))


@dataclass(frozen=True)
class Release:
    """One index entry's sysupgrade asset for this platform."""

    version: str
    release_date: str
    is_major: bool
    url: str
    sha256: str | None
    size: int | None


class Integrity(TypedDict, total=False):
    checksum: str
    size_bytes: int


class AssetObject(TypedDict):
    url: str
    integrity: NotRequired[Integrity]


# The e2e index anchors the running firmware with a bare URL;
# a published asset is an object with an integrity block.
type Asset = str | AssetObject


# Optional throughout, as the published index is not ours:
# only an entry carrying this platform's asset must name its version and date,
# and other boards' assets stay raw so their shape cannot reject the index.
class Metadata(TypedDict, total=False):
    bmc_version: str
    release_date: str
    is_major: bool
    assets: dict[str, msgspec.Raw]


class Entry(TypedDict, total=False):
    metadata: Metadata


class Index(TypedDict):
    version: str
    releases: NotRequired[list[Entry]]


def parse_releases(document: str, *, asset_key: str = PLATFORM_ASSET_KEY) -> list[Release]:
    """The releases in an index document that ship an `asset_key` asset,
    in the document's own order.

    Raises ``ValueError`` when the document is not a v1 index.
    """
    try:
        index = msgspec.json.decode(document, type=Index)
    except msgspec.ValidationError as exc:
        msg = f"index is not a {INDEX_VERSION} document: {exc}"
        raise ValueError(msg) from None
    except msgspec.DecodeError as exc:
        msg = f"index is not JSON: {exc}"
        raise ValueError(msg) from None
    if index["version"] != INDEX_VERSION:
        msg = f"index is not a {INDEX_VERSION} document"
        raise ValueError(msg)
    releases: list[Release] = []
    for entry in index.get("releases", []):
        metadata = entry.get("metadata", Metadata())
        raw_asset = metadata.get("assets", {}).get(asset_key)
        if raw_asset is None:
            continue
        version, release_date = metadata.get("bmc_version"), metadata.get("release_date")
        try:
            # msgspec validates against the union but types only a `type[T]` target.
            asset = cast("Asset | None", msgspec.json.decode(raw_asset, type=Asset | None))
        except msgspec.ValidationError as exc:
            release = version or "without a version"
            msg = f"index release {release} has a malformed {asset_key} asset: {exc}"
            raise ValueError(msg) from None
        if asset is None:
            continue
        if version is None or release_date is None:
            msg = f"an index release with a {asset_key} asset names no version or date"
            raise ValueError(msg)
        integrity = Integrity() if isinstance(asset, str) else asset.get("integrity", Integrity())
        checksum = integrity.get("checksum")
        releases.append(
            Release(
                version=version,
                release_date=release_date,
                is_major=metadata.get("is_major", False),
                url=asset if isinstance(asset, str) else asset["url"],
                sha256=checksum.lower() if checksum is not None else None,
                size=integrity.get("size_bytes"),
            )
        )
    return releases


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


class _RecordingWriter(io.BufferedIOBase):
    def __init__(self, output: io.BufferedIOBase) -> None:
        super().__init__()
        self._output = output
        self.reset()

    def reset(self) -> None:
        self.recording = False
        self.bytes_attempted = 0
        self.bytes_written = 0

    @property
    @override
    def closed(self) -> bool:
        return self._output.closed

    @override
    def writable(self) -> bool:
        return True

    @override
    def write(self, data: Buffer, /) -> int:
        if self.recording:
            self.bytes_attempted += memoryview(data).nbytes
        written = self._output.write(data)
        if self.recording:
            self.bytes_written += written
        return written

    @override
    def flush(self) -> None:
        self._output.flush()

    @override
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
    _status: int = 0
    _expected_bytes: int | None = None

    @override
    def setup(self) -> None:
        super().setup()
        # BaseRequestHandler.__init__ runs setup() before handle(), so every method sees it set.
        self._body_writer = _RecordingWriter(self.wfile)  # pyright: ignore[reportUninitializedInstanceVariable]
        self.wfile = self._body_writer

    @override
    def do_GET(self) -> None:
        server = cast("_RecordingHTTPServer", self.server)
        server.begin_transfer()
        # The writer is per-connection while records are per-request;
        # a keep-alive connection must not leak counts across requests.
        self._body_writer.reset()
        self._status = 0
        self._expected_bytes = None
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
            server.append_record(
                RequestRecord(
                    path=self.path,
                    status=self._status,
                    bytes_attempted=self._body_writer.bytes_attempted,
                    bytes_written=self._body_writer.bytes_written,
                    complete=complete,
                    error=error,
                )
            )
            server.end_transfer()

    @override
    def send_response(self, code: int, message: str | None = None) -> None:
        self._status = code
        super().send_response(code, message)

    @override
    def send_header(self, keyword: str, value: str) -> None:
        if keyword.lower() == "content-length":
            self._expected_bytes = int(value)
        super().send_header(keyword, value)

    @override
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
