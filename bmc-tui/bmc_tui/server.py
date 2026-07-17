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

"""Configurable local HTTP server for driving a device against local assets.

A test can pin exact bytes, dimensions and status codes without a public URL:
nothing to fetch through a DNS filter, and no upstream that can change under
the test. A view maps a path to a body, to a callable computing one per
request, or to a directory to mount; the handle reports every address it bound
on, so a caller can hand the device one it can actually reach.
"""

import json
import mimetypes
import shutil
import socket
import socketserver
import sys
import threading
import time
import weakref
from collections.abc import Callable, Mapping
from dataclasses import dataclass, field
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import BinaryIO, NamedTuple, Self, cast
from urllib.parse import parse_qs, unquote, urlsplit

from bmc_tui import console

# Bounded so a self-returning view fails loudly instead of hanging the thread.
MAX_RESPONSE_DEPTH = 16

STARTUP_TIMEOUT_SECS = 5.0

# Backstop for joining the accept loop; shutdown() should end it at once.
SHUTDOWN_JOIN_SECS = 5.0

# stop() joins workers, and a kept-alive connection parks one
# in a blocking read — so without a bound, one idle client wedges shutdown.
IDLE_TIMEOUT_SECS = 5.0
# Per-write payload for an in-memory body, matching what `shutil.copyfileobj`
# uses for files: small enough that one write cannot exhaust `IDLE_TIMEOUT_SECS`.
CHUNK_BYTES = 64 * 1024

_JSON_TYPES = (dict, list)

# A client that hangs up resets rather than closing cleanly.
EXPECTED_ERRORS = (BrokenPipeError, ConnectionResetError)


class Drop(NamedTuple):
    """A client that hung up, and the request it had last been served."""

    peer: str
    method: str
    path: str


@dataclass(frozen=True)
class Request:
    """One inbound request, as handed to a callable view."""

    method: str
    path: str
    query: dict[str, list[str]]
    headers: Mapping[str, str]
    body: bytes


ResponseValue = bytes | str | Path | dict | list
Response = ResponseValue | Callable[[Request], "Response"]


@dataclass(frozen=True)
class View:
    """A response with a non-200 status or a forced content type."""

    response: Response
    status: int = 200
    content_type: str | None = None


ViewConfig = View | Response

# Pre-dispatch hook handed the raw handler; returning True claims the request
# and no view runs. This is how the package rig injects connection-level
# faults (refused connections, stalled bodies) a view cannot express.
Intercept = Callable[[BaseHTTPRequestHandler], bool]


def default_serve_ip(device_host: str, *, port: int = 22) -> str:
    """The IPv4 address the device can reach us on: the source address the
    kernel picks for an AF_INET connection towards the device. Pinned to
    AF_INET — the URLs we hand out never bracket IPv6 literals."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(8)
        sock.connect((device_host, port))
        ip: str = sock.getsockname()[0]
        return ip


def _route_source_ip() -> str | None:
    """Source address for the default route, or None when there is no route.

    A UDP connect only fixes the socket's peer, so this costs no traffic and
    works offline. TEST-NET-1 (RFC 5737) is never routable to a real host.
    """
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.connect(("192.0.2.1", 9))
            ip: str = sock.getsockname()[0]
            return ip
    except OSError:
        return None


def _resolve(response: Response, request: Request) -> ResponseValue:
    current = response
    for _ in range(MAX_RESPONSE_DEPTH):
        if not callable(current):
            return current
        current = cast("Callable[[Request], Response]", current)(request)
    raise RecursionError(f"view for {request.path} still callable after {MAX_RESPONSE_DEPTH} hops")


def _encode(value: ResponseValue) -> tuple[bytes, str]:
    if isinstance(value, _JSON_TYPES):
        return json.dumps(value).encode(), "application/json"
    if isinstance(value, str):
        return value.encode(), "text/plain; charset=utf-8"
    if isinstance(value, (bytes, bytearray)):
        return bytes(value), "application/octet-stream"
    raise TypeError(f"view returned an unsupported response: {type(value).__name__}")


def _byte_range(header: str | None, size: int) -> tuple[int, int] | None:
    """Inclusive start/end for a single `bytes=` range, or None to send it all.

    Only the one-range form is honoured; anything else falls back to the whole
    body, which is a valid answer to any Range request.
    """
    if not header or not header.startswith("bytes=") or "," in header:
        return None
    first, _, last = header.removeprefix("bytes=").partition("-")
    if not first.isdigit():
        return None
    start = int(first)
    end = int(last) if last.isdigit() else size - 1
    if start > end or start >= size:
        return None
    return start, min(end, size - 1)


def _as_view(config: ViewConfig) -> View:
    return config if isinstance(config, View) else View(response=config)


def _mount_target(root: Path, request_path: str, prefix: str) -> Path | None:
    """Resolve `request_path` inside a mounted `root`, or None if it escapes.

    Escape is judged on the request path rather than the resolved one, because
    a mounted tree may symlink outside itself on purpose — the package rig
    links store artifacts instead of copying them.
    """
    relative = unquote(request_path[len(prefix) :]).lstrip("/")
    if ".." in Path(relative).parts:
        return None
    target = root / relative
    return target if target.is_file() else None


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    timeout = IDLE_TIMEOUT_SECS

    def __init__(
        self,
        request: socket.socket,
        client_address: tuple[str, int],
        server: socketserver.BaseServer,
        *,
        views: Mapping[str, View],
        intercept: Intercept | None = None,
    ) -> None:
        self._views = views
        self._intercept = intercept
        self._mounts = sorted(
            ((p, v.response) for p, v in views.items() if isinstance(v.response, Path)),
            key=lambda item: len(item[0]),
            reverse=True,
        )
        super().__init__(request, client_address, server)

    # Silent: the per-request stderr line drowns a harness's own progress output.
    def log_message(self, format: str, *args: object) -> None:  # noqa: A002
        del format, args

    def handle_one_request(self) -> None:
        try:
            super().handle_one_request()
        except EXPECTED_ERRORS:
            # Recorded, not printed: a spinner owns stdout,
            # and a write from here renders over the top of it.
            self.close_connection = True
            server = cast("_Server", self.server)
            server.note_drop(Drop(self.client_address[0], self.command or "?", self.path or "?"))

    def _request(self) -> Request:
        split = urlsplit(self.path)
        length = int(self.headers.get("Content-Length") or 0)
        return Request(
            method=self.command,
            path=split.path,
            query=parse_qs(split.query),
            headers=dict(self.headers.items()),
            body=self.rfile.read(length) if length else b"",
        )

    def _lookup(self, request: Request) -> tuple[View, Path | None] | None:
        view = self._views.get(request.path)
        if view is not None:
            return view, None
        for prefix, root in self._mounts:
            if isinstance(root, Path) and root.is_dir() and request.path.startswith(prefix):
                target = _mount_target(root, request.path, prefix)
                if target is not None:
                    return self._views[prefix], target
        return None

    def _headers(self, *, status: int, length: int, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(length))
        self.end_headers()

    def _respond(self, *, status: int, body: bytes, content_type: str, head: bool) -> None:
        """Ranged like `_respond_file`, so sampling a header costs 16 bytes."""
        size = len(body)
        ranged = status == HTTPStatus.OK
        span = _byte_range(self.headers.get("Range"), size) if ranged else None
        if span is None:
            self._headers(status=status, length=size, content_type=content_type)
        else:
            start, end = span
            body = body[start : end + 1]
            self.send_response(HTTPStatus.PARTIAL_CONTENT)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
        if not head:
            # One socket operation per chunk. `timeout` bounds each of them, and
            # `wfile` is unbuffered, so a single multi-megabyte write to a peer
            # that stops reading at its own cap trips it before that cap is hit.
            view = memoryview(body)
            for start in range(0, len(view), CHUNK_BYTES):
                self.wfile.write(view[start : start + CHUNK_BYTES])

    def _respond_file(self, *, status: int, path: Path, content_type: str, head: bool) -> None:
        """Stream a file rather than buffering it — the package rig serves
        firmware tarballs and a whole store closure through here.

        A ranged request is answered as one, so a caller sampling a header
        does not pull a whole fixture down and hang up mid-write.
        """
        size = path.stat().st_size
        span = _byte_range(self.headers.get("Range"), size)
        with path.open("rb") as handle:
            if span is None:
                self._headers(status=status, length=size, content_type=content_type)
            else:
                start, end = span
                handle.seek(start)
                self.send_response(206)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
                self.send_header("Content-Length", str(end - start + 1))
                self.end_headers()
            if head:
                return
            self._stream(handle, span)

    def _stream(self, handle: BinaryIO, span: tuple[int, int] | None) -> None:
        if span is None:
            shutil.copyfileobj(handle, self.wfile)
        else:
            self.wfile.write(handle.read(span[1] - span[0] + 1))

    def _handle(self, *, head: bool = False) -> None:
        if self._intercept is not None and self._intercept(self):
            return
        request = self._request()
        found = self._lookup(request)
        if found is None:
            self._respond(status=404, body=b"not found\n", content_type="text/plain", head=head)
            return
        view, mounted = found
        # A view is caller code: a failure has to reach the client as a status
        # rather than drop the connection mid-response.
        try:
            value = mounted if mounted is not None else _resolve(view.response, request)
            if isinstance(value, Path):
                guessed, _ = mimetypes.guess_type(value.name)
                self._respond_file(
                    status=view.status,
                    path=value,
                    content_type=view.content_type or guessed or "application/octet-stream",
                    head=head,
                )
                return
            body, guessed = _encode(value)
        except Exception as error:
            detail = f"view for {request.path} failed: {error}\n"
            self._respond(status=500, body=detail.encode(), content_type="text/plain", head=head)
            return
        self._respond(
            status=view.status,
            body=body,
            content_type=view.content_type or guessed,
            head=head,
        )

    def do_GET(self) -> None:
        self._handle()

    def do_HEAD(self) -> None:
        self._handle(head=True)

    def do_POST(self) -> None:
        self._handle()

    def do_PUT(self) -> None:
        self._handle()

    def do_DELETE(self) -> None:
        self._handle()


class _Server(ThreadingHTTPServer):
    """Threading server that renders a worker failure rather than dumping it.

    `stage` installs rich's hook on `sys.excepthook`, which a worker never
    reaches — that covers the main thread only.
    """

    def __init__(
        self,
        server_address: tuple[str, int],
        handler: Callable[..., BaseHTTPRequestHandler],
    ) -> None:
        self.drops: list[Drop] = []
        self._drop_lock = threading.Lock()
        super().__init__(server_address, handler)

    def note_drop(self, drop: Drop) -> None:
        with self._drop_lock:
            self.drops.append(drop)

    def handle_error(
        self,
        request: socket.socket | tuple[bytes, socket.socket],
        client_address: tuple[str, int] | str,
    ) -> None:
        del request
        _, exc_value, exc_tb = sys.exc_info()
        peer = client_address[0] if isinstance(client_address, tuple) else client_address
        console.error(f"serving {peer} failed")
        if exc_value is not None:
            sys.excepthook(type(exc_value), exc_value, exc_tb)


@dataclass
class ServerHandle:
    """A running server: where it bound, what it serves, how to stop it."""

    binds: list[str]
    urls: list[str]
    _httpd: _Server = field(repr=False)
    _finalizer: weakref.finalize = field(repr=False)

    @property
    def port(self) -> int:
        return int(self._httpd.server_address[1])

    @property
    def drops(self) -> list[Drop]:
        return self._httpd.drops

    def url(self, path: str, *, bind: str | None = None) -> str:
        """Absolute URL for a served path, defaulting to the first bind."""
        return f"{bind or self.binds[0]}{path}"

    def stop(self) -> None:
        """Shut down and join the thread. Idempotent."""
        self._finalizer()

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_exc: object) -> None:
        self.stop()


def _shutdown(httpd: _Server, thread: threading.Thread) -> None:
    httpd.shutdown()
    httpd.server_close()
    thread.join(timeout=SHUTDOWN_JOIN_SECS)


def _await_healthy(httpd: _Server, thread: threading.Thread) -> None:
    """Block until the listener accepts, else tear down and raise.

    A half-started server would otherwise hand out URLs that time out on the
    device, which reads as a device fault rather than a harness fault.
    """
    port = int(httpd.server_address[1])
    for _ in range(int(STARTUP_TIMEOUT_SECS * 20)):
        if not thread.is_alive():
            break
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return
        except OSError:
            time.sleep(0.05)
    _shutdown(httpd, thread)
    raise RuntimeError(f"local server never accepted on port {port}")


def server(
    views: Mapping[str, ViewConfig],
    *,
    reachable_from: str | None = None,
    bind_host: str = "0.0.0.0",
    port: int = 0,
    intercept: Intercept | None = None,
) -> ServerHandle:
    """Serve `views` on an ephemeral port until the handle is stopped.

    `reachable_from` is a device host; when given, the LAN bind is the address
    that device would reach us on rather than a guess at the default route.
    """
    resolved = {path: _as_view(config) for path, config in views.items()}
    httpd = _Server((bind_host, port), _handler(resolved, intercept))
    # socketserver's _Threads.append drops daemon threads,
    # so ThreadingHTTPServer's daemon_threads=True default leaves nothing
    # for server_close() to join — a request can outlive stop().
    #
    # The accept loop stays daemon so it cannot wedge exit.
    httpd.daemon_threads = False
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    _await_healthy(httpd, thread)

    bound = int(httpd.server_address[1])
    lan = default_serve_ip(reachable_from) if reachable_from else _route_source_ip()
    hosts = ["127.0.0.1"] + ([lan] if lan and lan != "127.0.0.1" else [])
    return ServerHandle(
        binds=[f"http://{host}:{bound}" for host in hosts],
        urls=sorted(resolved),
        _httpd=httpd,
        _finalizer=weakref.finalize(httpd, _shutdown, httpd, thread),
    )


def _handler(
    views: Mapping[str, View], intercept: Intercept | None = None
) -> Callable[..., BaseHTTPRequestHandler]:
    def build(
        request: socket.socket,
        client_address: tuple[str, int],
        server: socketserver.BaseServer,
    ) -> BaseHTTPRequestHandler:
        return _Handler(request, client_address, server, views=views, intercept=intercept)

    return build
