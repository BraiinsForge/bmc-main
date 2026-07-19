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

"""Characterize grpcurl streaming and formatted-error behavior."""

import json
import multiprocessing
import shutil
import subprocess
import threading
import time
from collections.abc import Callable, Iterator
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from pathlib import Path
from typing import Any

import pytest

grpc = pytest.importorskip("grpc")

from bmc_tui import catalog  # noqa: E402

if shutil.which("grpcurl") is None:
    pytest.skip("grpcurl is not on PATH", allow_module_level=True)

_REPO_ROOT = Path(__file__).resolve().parents[2]
_PROTO_ROOT = Path("bmc-grpc/proto")
_PROTO_FILE = "web/upgrade.proto"

_DOWNLOADING = b"\x18\x01"
_DOWNLOAD = b"\x0a\x02\x08\x01"
_VERIFYING = b"\x18\x02"
_APPLYING = b"\x18\x03"


def _handler(
    behavior: Callable[[bytes, Any], Iterator[bytes]],
) -> Any:
    method = grpc.unary_stream_rpc_method_handler(
        behavior,
        request_deserializer=lambda payload: payload,
        response_serializer=lambda payload: payload,
    )
    return grpc.method_handlers_generic_handler(
        "braiins.bmc.web.UpgradeService", {"StartUpgrade": method}
    )


@contextmanager
def _server(behavior: Callable[[bytes, Any], Iterator[bytes]]) -> Iterator[int]:
    server = grpc.server(ThreadPoolExecutor(max_workers=1))
    server.add_generic_rpc_handlers((_handler(behavior),))
    port = server.add_insecure_port("127.0.0.1:0")
    server.start()
    try:
        yield port
    finally:
        server.stop(0).wait()


def _argv(port: int, *, max_time: float = 5) -> list[str]:
    return [
        "grpcurl",
        "-plaintext",
        "-format-error",
        "-max-time",
        str(max_time),
        "-import-path",
        str(_PROTO_ROOT),
        "-proto",
        _PROTO_FILE,
        "-d",
        json.dumps({"upgradeId": "offer-1"}),
        f"127.0.0.1:{port}",
        "braiins.bmc.web.UpgradeService/StartUpgrade",
    ]


def _objects(text: str) -> list[dict[str, Any]]:
    decoder = json.JSONDecoder()
    values: list[dict[str, Any]] = []
    remainder = text
    while remainder.strip():
        value, end = decoder.raw_decode(remainder.lstrip())
        if isinstance(value, dict):
            values.append(value)
        remainder = remainder.lstrip()[end:]
    return values


def _run(port: int, *, max_time: float = 5) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        _argv(port, max_time=max_time),
        capture_output=True,
        text=True,
        cwd=_REPO_ROOT,
        check=False,
    )


def _status(proc: subprocess.CompletedProcess[str]) -> tuple[str | None, str | None]:
    for value in _objects(proc.stdout):
        if (status := catalog._grpc_status(value)) is not None:
            return status
    return catalog._grpc_status_from_text(proc.stderr)


def _clean(_request: bytes, _context: Any) -> Iterator[bytes]:
    yield from (_DOWNLOADING, _DOWNLOAD, _VERIFYING, _APPLYING)


def test_grpcurl_clean_stream_closes_with_json_uint64_string() -> None:
    with _server(_clean) as port:
        proc = _run(port)
    assert proc.returncode == 0, proc.stderr
    assert _objects(proc.stdout) == [
        {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_DOWNLOADING"},
        {"download": {"downloadedBytes": "1"}},
        {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_VERIFYING"},
        {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_APPLYING"},
    ]


@pytest.mark.parametrize(
    ("grpc_code", "numeric_code", "expected"),
    [
        (grpc.StatusCode.INTERNAL, 13, "Internal"),
        (grpc.StatusCode.FAILED_PRECONDITION, 9, "FailedPrecondition"),
    ],
)
def test_grpcurl_format_error_exposes_terminal_status(
    grpc_code: Any, numeric_code: int, expected: str
) -> None:
    def abort(_request: bytes, context: Any) -> Iterator[bytes]:
        yield _DOWNLOADING
        context.abort(grpc_code, "characterized failure")

    with _server(abort) as port:
        proc = _run(port)
    assert proc.returncode != 0
    assert json.loads(proc.stderr) == {
        "code": numeric_code,
        "message": "characterized failure",
    }
    assert _status(proc) == (expected, "characterized failure")


def test_grpcurl_max_time_exposes_deadline_exceeded() -> None:
    release = threading.Event()

    def stall(_request: bytes, _context: Any) -> Iterator[bytes]:
        yield _DOWNLOADING
        release.wait(5)

    with _server(stall) as port:
        proc = _run(port, max_time=0.2)
        release.set()
    assert proc.returncode != 0
    status = json.loads(proc.stderr)
    assert status.keys() == {"code", "message"}
    assert status["code"] == 4
    assert isinstance(status["message"], str)
    assert _status(proc)[0] == "DeadlineExceeded"


def _serve_for_process(port_queue: Any, reached: Any) -> None:
    def stall_after_verifying(_request: bytes, _context: Any) -> Iterator[bytes]:
        yield from (_DOWNLOADING, _DOWNLOAD, _VERIFYING)
        reached.set()
        time.sleep(30)

    server = grpc.server(ThreadPoolExecutor(max_workers=1))
    server.add_generic_rpc_handlers((_handler(stall_after_verifying),))
    port = server.add_insecure_port("127.0.0.1:0")
    server.start()
    port_queue.put(port)
    server.wait_for_termination()


def test_grpcurl_abrupt_server_process_loss_after_verifying() -> None:
    context = multiprocessing.get_context("fork")
    port_queue = context.Queue()
    reached = context.Event()
    server = context.Process(target=_serve_for_process, args=(port_queue, reached))
    server.start()
    port = port_queue.get(timeout=5)
    client = subprocess.Popen(
        _argv(port),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        cwd=_REPO_ROOT,
    )
    try:
        assert reached.wait(5)
        server.terminate()
        server.join(5)
        stdout, stderr = client.communicate(timeout=5)
    finally:
        if server.is_alive():
            server.kill()
            server.join()
        if client.poll() is None:
            client.kill()
            client.wait()
    proc = subprocess.CompletedProcess(client.args, client.returncode, stdout, stderr)
    assert proc.returncode != 0
    assert _objects(proc.stdout)[:3] == [
        {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_DOWNLOADING"},
        {"download": {"downloadedBytes": "1"}},
        {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_VERIFYING"},
    ]
    status = json.loads(proc.stderr)
    assert status.keys() == {"code", "message"}
    assert status["code"] == 14
    assert isinstance(status["message"], str)
    assert _status(proc)[0] == "Unavailable"
