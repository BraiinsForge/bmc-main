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

"""Unit tests for the local asset server."""

import json
import socket
import struct
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

import pytest

from bmc_tui import server as bmc_server
from bmc_tui.server import Request, View, _Server, default_serve_ip, server


def _get(url: str) -> tuple[int, bytes, str]:
    with urllib.request.urlopen(url) as response:
        return response.status, response.read(), response.headers.get("Content-Type", "")


def test_bytes_view_is_served_verbatim() -> None:
    with server({"/raw": b"\x00\x01\x02"}) as handle:
        status, body, ctype = _get(handle.url("/raw"))
    assert (status, body) == (200, b"\x00\x01\x02")
    assert ctype == "application/octet-stream"


def test_mapping_view_is_served_as_json() -> None:
    with server({"/api": {"ok": True}}) as handle:
        _, body, ctype = _get(handle.url("/api"))
    assert json.loads(body) == {"ok": True}
    assert ctype == "application/json"


def test_file_view_infers_content_type_from_suffix(tmp_path: Path) -> None:
    png = tmp_path / "pixel.png"
    png.write_bytes(b"\x89PNG\r\n\x1a\n")
    with server({"/pixel.png": png}) as handle:
        _, body, ctype = _get(handle.url("/pixel.png"))
    assert body == b"\x89PNG\r\n\x1a\n"
    assert ctype == "image/png"


def test_file_view_streams_rather_than_buffering(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The package rig serves firmware tarballs and a store closure through here."""
    blob = tmp_path / "big.bin"
    payload = b"x" * (512 * 1024)
    blob.write_bytes(payload)

    def explode(_self: Path) -> bytes:
        raise AssertionError("a file response must stream, not buffer via read_bytes()")

    monkeypatch.setattr(Path, "read_bytes", explode)
    with server({"/big.bin": blob}) as handle:
        _, body, _ = _get(handle.url("/big.bin"))
    assert body == payload


def test_callable_view_sees_the_request() -> None:
    def echo(request: Request) -> bytes:
        return f"{request.method} {request.path} {request.query}".encode()

    with server({"/echo": echo}) as handle:
        _, body, _ = _get(handle.url("/echo") + "?a=1&a=2")
    assert body == b"GET /echo {'a': ['1', '2']}"


def test_callable_view_may_return_another_callable() -> None:
    with server({"/nested": lambda _req: lambda _inner: b"deep"}) as handle:
        _, body, _ = _get(handle.url("/nested"))
    assert body == b"deep"


def test_self_returning_view_fails_rather_than_hanging() -> None:
    def forever(_request: Request) -> object:
        return forever

    with server({"/loop": forever}) as handle, pytest.raises(urllib.error.HTTPError) as caught:
        _get(handle.url("/loop"))
    assert caught.value.status == 500


def test_view_can_force_status_and_content_type() -> None:
    views = {"/teapot": View(response="short and stout", status=418)}
    with server(views) as handle, pytest.raises(urllib.error.HTTPError) as caught:
        _get(handle.url("/teapot"))
    assert caught.value.status == 418


def test_unlisted_path_is_404() -> None:
    with server({"/known": b"x"}) as handle, pytest.raises(urllib.error.HTTPError) as caught:
        _get(handle.url("/missing"))
    assert caught.value.status == 404


def test_directory_view_mounts_the_tree(tmp_path: Path) -> None:
    (tmp_path / "nested").mkdir()
    (tmp_path / "nested" / "asset.txt").write_text("mounted")
    with server({"/files": tmp_path}) as handle:
        _, body, _ = _get(handle.url("/files/nested/asset.txt"))
    assert body == b"mounted"


def test_directory_view_follows_symlinks_out_of_the_tree(tmp_path: Path) -> None:
    """The package rig symlinks store artifacts in rather than copying them."""
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "artifact.bin").write_bytes(b"linked")
    root = tmp_path / "root"
    (root / "nested").mkdir(parents=True)
    (root / "nested" / "artifact.bin").symlink_to(outside / "artifact.bin")
    with server({"/": root}) as handle:
        _, body, _ = _get(handle.url("/nested/artifact.bin"))
    assert body == b"linked"


def test_directory_view_refuses_traversal(tmp_path: Path) -> None:
    (tmp_path / "root").mkdir()
    (tmp_path / "secret.txt").write_text("nope")
    views = {"/files": tmp_path / "root"}
    with server(views) as handle, pytest.raises(urllib.error.HTTPError) as caught:
        _get(handle.url("/files/../secret.txt"))
    assert caught.value.status == 404


def test_binds_include_loopback_and_a_free_port() -> None:
    with server({"/x": b"x"}) as handle:
        assert any(bind.startswith("http://127.0.0.1:") for bind in handle.binds)
        assert handle.port > 0
        assert handle.urls == ["/x"]


def test_stop_releases_the_port_and_is_idempotent() -> None:
    handle = server({"/x": b"x"})
    port = handle.port
    handle.stop()
    handle.stop()
    with socket.socket() as probe:
        probe.settimeout(1)
        assert probe.connect_ex(("127.0.0.1", port)) != 0


def test_bind_conflict_reaches_the_caller() -> None:
    """A taken port must abort the run, not hand back a dead handle."""
    with socket.socket() as taken:
        taken.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        taken.bind(("0.0.0.0", 0))
        taken.listen(1)
        port = taken.getsockname()[1]
        with pytest.raises(OSError):
            server({"/x": b"x"}, port=port)


def test_never_accepting_server_raises_and_leaves_no_thread(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A half-started server hands out URLs that fail as if the device were at fault."""
    monkeypatch.setattr(bmc_server, "STARTUP_TIMEOUT_SECS", 0.2)

    def refuse(*_args: object, **_kwargs: object) -> object:
        raise OSError("nothing is listening")

    monkeypatch.setattr(bmc_server.socket, "create_connection", refuse)

    before = {thread.ident for thread in threading.enumerate()}
    with pytest.raises(RuntimeError, match="never accepted"):
        server({"/x": b"x"})
    spawned = [thread for thread in threading.enumerate() if thread.ident not in before]
    assert not any(thread.is_alive() for thread in spawned)


def test_an_idle_client_cannot_wedge_stop(monkeypatch: pytest.MonkeyPatch) -> None:
    """stop() joins request workers, so a parked keep-alive must still time out."""
    monkeypatch.setattr(bmc_server._Handler, "timeout", 0.3)
    handle = server({"/x": b"x"})
    host, port = handle.binds[0].removeprefix("http://").split(":")
    with socket.create_connection((host, int(port))) as idle:
        idle.sendall(b"GET /x HTTP/1.1\r\nHost: x\r\n\r\n")
        idle.recv(64)  # response read; the connection now sits open and silent
        started = time.monotonic()
        handle.stop()
        assert time.monotonic() - started < 5, "stop() waited on an idle connection"


def test_stopped_server_leaves_no_thread_behind() -> None:
    before = {thread.ident for thread in threading.enumerate()}
    handle = server({"/x": b"x"})
    spawned = [thread for thread in threading.enumerate() if thread.ident not in before]
    handle.stop()
    assert spawned, "the server should have started a thread to begin with"
    assert not any(thread.is_alive() for thread in spawned)


def test_default_serve_ip_uses_the_route_towards_the_device() -> None:
    with socket.create_server(("127.0.0.1", 0)) as listener:
        port = listener.getsockname()[1]
        assert default_serve_ip("127.0.0.1", port=port) == "127.0.0.1"


def test_file_view_answers_a_ranged_request(tmp_path: Path) -> None:
    """The harness samples a header rather than pulling a whole fixture."""
    blob = tmp_path / "asset.bin"
    blob.write_bytes(bytes(range(256)))
    with server({"/asset.bin": blob}) as handle:
        request = urllib.request.Request(handle.url("/asset.bin"), headers={"Range": "bytes=0-15"})
        with urllib.request.urlopen(request) as response:
            assert response.status == 206
            assert response.headers["Content-Range"] == "bytes 0-15/256"
            assert response.read() == bytes(range(16))


def test_bytes_view_answers_a_ranged_request_like_a_file() -> None:
    """The harness synthesises its oversized cases, so a 200 here
    would make every pre-flight pull megabytes and hang up mid-write."""
    with server({"/asset.bin": bytes(range(256))}) as handle:
        request = urllib.request.Request(handle.url("/asset.bin"), headers={"Range": "bytes=0-15"})
        with urllib.request.urlopen(request) as response:
            assert response.status == 206
            assert response.headers["Content-Range"] == "bytes 0-15/256"
            assert response.read() == bytes(range(16))


def test_a_body_past_one_chunk_arrives_whole() -> None:
    """Bodies are written a chunk at a time, so the loop's slicing has to cover
    the tail: a device fetch reads megabytes through this path."""
    body = bytes(range(256)) * 1024  # 256 KiB, several chunks
    with server({"/big.bin": body}) as handle:
        status, received, _ = _get(handle.url("/big.bin"))
    assert status == 200
    assert received == body


def test_unsatisfiable_range_falls_back_to_the_whole_body(tmp_path: Path) -> None:
    blob = tmp_path / "asset.bin"
    blob.write_bytes(b"abcd")
    with server({"/asset.bin": blob}) as handle:
        request = urllib.request.Request(handle.url("/asset.bin"), headers={"Range": "bytes=99-"})
        with urllib.request.urlopen(request) as response:
            assert response.status == 200
            assert response.read() == b"abcd"


def test_a_client_that_hangs_up_is_recorded_not_printed(
    tmp_path: Path, capfd: pytest.CaptureFixture[str]
) -> None:
    """Workers must not print: a spinner owns stdout and would render doubled."""
    blob = tmp_path / "big.bin"
    blob.write_bytes(b"x" * (4 * 1024 * 1024))
    with server({"/big.bin": blob}) as handle:
        host, port = handle.binds[0].removeprefix("http://").split(":")
        with socket.create_connection((host, int(port))) as rude:
            rude.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
            rude.sendall(b"GET /big.bin HTTP/1.1\r\nHost: x\r\n\r\n")
            rude.recv(64)
        time.sleep(0.5)
        assert handle.drops, "the disconnect should have been recorded"
        assert handle.drops[0].path == "/big.bin"
        assert handle.drops[0].method == "GET"
        assert _get(handle.url("/big.bin"))[1] == b"x" * (4 * 1024 * 1024)
    assert "Traceback" not in capfd.readouterr().err


def test_unexpected_error_reaches_the_traceback_hook(
    capfd: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    seen: list[type[BaseException]] = []
    monkeypatch.setattr(sys, "excepthook", lambda kind, *_rest: seen.append(kind))
    handler = _Server.__new__(_Server)
    try:
        raise ValueError("boom")
    except ValueError:
        handler.handle_error(None, ("192.0.2.5", 1234))  # type: ignore[arg-type]
    assert seen == [ValueError]
    assert "serving 192.0.2.5 failed" in capfd.readouterr().err
