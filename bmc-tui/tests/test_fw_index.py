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

"""Tests for the firmware release index and recording HTTP server."""

import hashlib
import json
import socket
import struct
import threading
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

import pytest

import bmc_tui.fw_index
from bmc_tui.bos_version import BosVersion, parse_bos_version
from bmc_tui.fw_index import (
    INDEX_NAME,
    PLATFORM_ASSET_KEY,
    FwIndexServer,
    index_document,
)

_RUNNING_TEXT = "2025-06-15-0-acde0123-25.06"
_IMAGE_TEXT = "2025-07-01-0-0badc0de-25.07"
_ANCHOR_URL = "http://127.0.0.1:8082/anchor.tar"
_IMAGE_URL = "http://127.0.0.1:8082/firmware.tar"
_FIRMWARE_BYTES = b"BDK-609 firmware fixture\n"


def _versions() -> tuple[BosVersion, BosVersion]:
    return parse_bos_version(_RUNNING_TEXT), parse_bos_version(_IMAGE_TEXT)


def _document(firmware: Path) -> str:
    running, image = _versions()
    data = firmware.read_bytes()
    return index_document(
        running=running,
        image=image,
        anchor_url=_ANCHOR_URL,
        image_url=_IMAGE_URL,
        image_sha256=hashlib.sha256(data).hexdigest().upper(),
        image_size=len(data),
    )


def _get(url: str) -> bytes:
    with urllib.request.urlopen(url) as response:
        return response.read()


def _wait_for_requests(server: FwIndexServer, count: int) -> None:
    deadline = time.monotonic() + 5
    while len(server.requests()) < count and time.monotonic() < deadline:
        time.sleep(0.01)
    assert len(server.requests()) == count


def test_index_document_matches_minimal_golden_shape(tmp_path: Path) -> None:
    firmware = tmp_path / "firmware.tar"
    firmware.write_bytes(_FIRMWARE_BYTES)
    running, image = _versions()
    checksum = hashlib.sha256(_FIRMWARE_BYTES).hexdigest()

    document = json.loads(_document(firmware))

    assert document == {
        "type": "bmc",
        "status": "Active",
        "title": "BDK-609 e2e firmware releases",
        "version": "v1",
        "releases": [
            {
                "uuid": str(uuid.uuid5(uuid.NAMESPACE_URL, running.canonical)),
                "metadata_version": "v1",
                "metadata": {
                    "bmc_version": running.canonical,
                    "is_major": False,
                    "release_date": running.release_date,
                    "description": "BDK-609 e2e",
                    "assets": {PLATFORM_ASSET_KEY: _ANCHOR_URL},
                },
            },
            {
                "uuid": str(uuid.uuid5(uuid.NAMESPACE_URL, image.canonical)),
                "metadata_version": "v1",
                "metadata": {
                    "bmc_version": image.canonical,
                    "is_major": False,
                    "release_date": image.release_date,
                    "description": "BDK-609 e2e",
                    "assets": {
                        PLATFORM_ASSET_KEY: {
                            "url": _IMAGE_URL,
                            "integrity": {
                                "checksum": checksum,
                                "size_bytes": len(_FIRMWARE_BYTES),
                            },
                        }
                    },
                },
            },
        ],
    }


def test_index_document_is_byte_deterministic(tmp_path: Path) -> None:
    firmware = tmp_path / "firmware.tar"
    firmware.write_bytes(_FIRMWARE_BYTES)

    assert _document(firmware) == _document(firmware)


def test_server_records_completed_index_and_firmware_fetches(tmp_path: Path) -> None:
    root = tmp_path / "serve"
    root.mkdir()
    firmware = root / "firmware.tar"
    firmware.write_bytes(_FIRMWARE_BYTES)
    body = _document(firmware).encode()
    (root / INDEX_NAME).write_bytes(body)

    with FwIndexServer(root, port=0, bind_ip="127.0.0.1") as server:
        base_url = f"http://127.0.0.1:{server.port}"
        assert _get(f"{base_url}/{INDEX_NAME}") == body

        _wait_for_requests(server, 1)
        requests = server.requests()
        assert len(requests) == 1
        index_request = requests[0]
        assert index_request.path == f"/{INDEX_NAME}"
        assert index_request.status == 200
        assert index_request.complete is True
        assert index_request.bytes_written == len(body)
        assert index_request.bytes_attempted == len(body)
        assert index_request.error is None
        assert server.completed(f"/{INDEX_NAME}") is True
        assert server.completed("/firmware.tar") is False

        assert _get(f"{base_url}/firmware.tar") == _FIRMWARE_BYTES
        _wait_for_requests(server, 2)
        assert server.completed("/firmware.tar") is True


def test_keepalive_connection_records_bytes_per_request(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = tmp_path / "serve"
    root.mkdir()
    firmware = root / "firmware.tar"
    firmware.write_bytes(_FIRMWARE_BYTES)
    body = _document(firmware).encode()
    (root / INDEX_NAME).write_bytes(body)
    monkeypatch.setattr(bmc_tui.fw_index._RecordingHandler, "protocol_version", "HTTP/1.1")

    with FwIndexServer(root, port=0, bind_ip="127.0.0.1") as server:
        client = socket.create_connection(("127.0.0.1", server.port))
        client.sendall(f"GET /{INDEX_NAME} HTTP/1.1\r\nHost: localhost\r\n\r\n".encode())
        _wait_for_requests(server, 1)
        client.sendall(
            b"GET /firmware.tar HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        while client.recv(65_536):
            pass
        client.close()

        _wait_for_requests(server, 2)
        records = {record.path: record for record in server.requests()}
        index_record = records[f"/{INDEX_NAME}"]
        firmware_record = records["/firmware.tar"]
        assert index_record.bytes_written == len(body)
        assert firmware_record.bytes_attempted == len(_FIRMWARE_BYTES)
        assert firmware_record.bytes_written == len(_FIRMWARE_BYTES)
        assert firmware_record.complete is True
        assert server.completed("/firmware.tar") is True


def test_fully_served_404_is_not_provenance(tmp_path: Path) -> None:
    root = tmp_path / "serve"
    root.mkdir()

    with FwIndexServer(root, port=0, bind_ip="127.0.0.1") as server:
        path = "/missing.tar"
        with pytest.raises(urllib.error.HTTPError) as caught:
            urllib.request.urlopen(f"http://127.0.0.1:{server.port}{path}")
        with caught.value as response:
            assert response.read()

        _wait_for_requests(server, 1)
        request = server.requests()[0]
        assert request.path == path
        assert request.status == 404
        assert request.complete is True
        assert server.completed(path) is False


def test_aborted_transfer_is_not_complete(tmp_path: Path) -> None:
    root = tmp_path / "serve"
    root.mkdir()
    body = b"x" * (8 * 1_024 * 1_024)
    (root / "firmware.tar").write_bytes(body)

    with FwIndexServer(root, port=0, bind_ip="127.0.0.1") as server:
        client = socket.create_connection(("127.0.0.1", server.port))
        client.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4_096)
        client.sendall(
            b"GET /firmware.tar HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        assert client.recv(4_096)
        client.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
        client.close()

        _wait_for_requests(server, 1)
        request = server.requests()[0]
        assert request.path == "/firmware.tar"
        assert request.status == 200
        assert request.complete is False
        assert request.bytes_attempted > request.bytes_written
        assert request.error is not None
        assert server.completed("/firmware.tar") is False


def test_server_reports_active_transfer_until_handler_finishes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = tmp_path / "serve"
    root.mkdir()
    (root / "firmware.tar").write_bytes(_FIRMWARE_BYTES)
    release = threading.Event()
    write = bmc_tui.fw_index._RecordingWriter.write

    def blocked_write(writer: bmc_tui.fw_index._RecordingWriter, data: bytes) -> int:
        if data == _FIRMWARE_BYTES:
            release.wait(timeout=5)
        return write(writer, data)

    monkeypatch.setattr(bmc_tui.fw_index._RecordingWriter, "write", blocked_write)
    with FwIndexServer(root, port=0, bind_ip="127.0.0.1") as server:
        request = threading.Thread(
            target=_get,
            args=(f"http://127.0.0.1:{server.port}/firmware.tar",),
        )
        request.start()
        deadline = time.monotonic() + 5
        try:
            while server.active_transfers() == 0 and time.monotonic() < deadline:
                time.sleep(0.01)
            assert server.active_transfers() == 1
        finally:
            release.set()
        request.join(timeout=5)
        assert not request.is_alive()

        # The client returns once it has the whole body; end_transfer runs on
        # the server's handler thread after that, so poll rather than race it.
        deadline = time.monotonic() + 5
        while server.active_transfers() and time.monotonic() < deadline:
            time.sleep(0.01)

        assert server.active_transfers() == 0
