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

"""Low-level JSONL TCP client with SO_KEEPALIVE and background reader.

Manages the TCP connection to the guest event daemon, reads incoming messages
on a background thread, and exposes a thread-safe interface for sending
commands and retrieving events.
"""

from __future__ import annotations

import contextlib
import socket
import threading
from typing import TYPE_CHECKING

from bmc_virt.protocol import PROTOCOL_VERSION, Msg, MsgType

if TYPE_CHECKING:
    from collections.abc import Callable

# ── TCP keepalive tuning ───────────────────────────────────────────────────────
# Start probing after 5s idle, probe every 3s, give up after 3 failures → ~14s
_KEEPALIVE_IDLE = 5
_KEEPALIVE_INTERVAL = 3
_KEEPALIVE_COUNT = 3


class DaemonConnectionError(Exception):
    """Raised when the connection to the event daemon fails or is lost."""


class DaemonProtocolError(Exception):
    """Raised on unexpected protocol messages (version mismatch, bad handshake)."""


class Client:
    """Low-level JSONL TCP client for the bmc-virt event daemon.

    Connects to the daemon, performs the hello/synced handshake, then reads
    messages on a background thread. Incoming messages are dispatched to a
    callback.

    Not intended for direct use — the VM class wraps this.
    """

    def __init__(
        self,
        sock: socket.socket,
        on_message: Callable[[Msg], None],
        initial_buf: bytes = b"",
    ) -> None:
        self._sock = sock
        self._on_message = on_message
        self._write_lock = threading.Lock()
        self._closed = False
        self._initial_buf = initial_buf
        self._reader_thread = threading.Thread(
            target=self._reader_loop,
            name="bmc-virt-reader",
            daemon=True,
        )
        self._reader_thread.start()

    @classmethod
    def connect(
        cls,
        host: str,
        port: int,
        on_message: Callable[[Msg], None],
    ) -> Client:
        """Open a TCP connection to the event daemon and perform handshake."""
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            _configure_keepalive(sock)
            sock.connect((host, port))
            sock.settimeout(10.0)
            hello_msg, remainder = _read_one(sock)
        except OSError as exc:
            sock.close()
            msg = f"Failed to connect to {host}:{port}"
            raise DaemonConnectionError(msg) from exc

        if hello_msg.type != MsgType.HELLO:
            sock.close()
            msg = f"Expected hello, got {hello_msg.type}"
            raise DaemonProtocolError(msg)
        if hello_msg.version != PROTOCOL_VERSION:
            sock.close()
            sv, cv = hello_msg.version, PROTOCOL_VERSION
            raise DaemonProtocolError(f"Protocol version mismatch: server={sv}, client={cv}")

        # Switch to blocking reads for the background thread
        sock.settimeout(None)
        return cls(sock, on_message, initial_buf=remainder)

    def send(self, msg: Msg) -> None:
        """Send a message to the daemon. Thread-safe."""
        with self._write_lock:
            if self._closed:
                raise DaemonConnectionError("Connection closed")
            try:
                self._sock.sendall(msg.to_line())
            except OSError as exc:
                self._closed = True
                raise DaemonConnectionError("Send failed") from exc

    def close(self) -> None:
        """Shut down the connection and stop the reader thread."""
        if self._closed:
            return
        self._closed = True
        with contextlib.suppress(OSError):
            self._sock.shutdown(socket.SHUT_RDWR)
        self._sock.close()

    def _reader_loop(self) -> None:
        """Background thread: read JSONL lines and dispatch to callback."""
        buf = self._initial_buf
        # Process any data already buffered from the handshake
        buf = self._process_buf(buf)
        while not self._closed:
            try:
                chunk = self._sock.recv(65_536)
            except OSError:
                break
            if not chunk:
                break
            buf = self._process_buf(buf + chunk)
        self._closed = True

    def _process_buf(self, buf: bytes) -> bytes:
        """Parse and dispatch all complete JSONL lines from buf, return remainder."""
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            if not line:
                continue
            try:
                msg = Msg.from_line(line)
            except (ValueError, KeyError):
                continue
            self._on_message(msg)
        return buf


# ── Helpers ────────────────────────────────────────────────────────────────────


def _configure_keepalive(sock: socket.socket) -> None:
    """Enable TCP keepalive with aggressive probing.

    Linux exposes the per-socket idle/interval/count tunables via
    ``TCP_KEEPIDLE``/``TCP_KEEPINTVL``/``TCP_KEEPCNT``; macOS uses
    ``TCP_KEEPALIVE`` for idle and inherits the other two from the system.
    """
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_KEEPALIVE, 1)
    if hasattr(socket, "TCP_KEEPIDLE"):
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPIDLE, _KEEPALIVE_IDLE)
    elif hasattr(socket, "TCP_KEEPALIVE"):
        # macOS / *BSD: per-socket idle time only.
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPALIVE, _KEEPALIVE_IDLE)
    if hasattr(socket, "TCP_KEEPINTVL"):
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPINTVL, _KEEPALIVE_INTERVAL)
    if hasattr(socket, "TCP_KEEPCNT"):
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPCNT, _KEEPALIVE_COUNT)


def _read_one(sock: socket.socket) -> tuple[Msg, bytes]:
    """Read exactly one JSONL message from the socket.

    Returns the parsed message and any remaining bytes in the buffer.
    """
    buf = b""
    while b"\n" not in buf:
        chunk = sock.recv(4_096)
        if not chunk:
            raise DaemonConnectionError("Connection closed during handshake")
        buf += chunk
    line, remainder = buf.split(b"\n", 1)
    return Msg.from_line(line), remainder
