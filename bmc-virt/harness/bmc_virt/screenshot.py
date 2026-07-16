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

"""Screenshot capture — grabs a frame from the relay daemon's IPC stream.

Connects directly to the relay (port 5910), reads one frame, converts to PNG.
No event daemon involvement — the relay is always streaming.
"""

from __future__ import annotations

import socket
import struct
import zlib
from dataclasses import dataclass
from pathlib import Path

# Relay binary IPC wire format
_TAG_FRAME = 0x01
_HEADER_SIZE = 5  # tag(1) + length(4)
_FRAME_HEADER_SIZE = 25  # seq(8) + width(4) + height(4) + stride(4) + bpp(4) + brightness(1)

# Pixel formats
_BPP_16 = 16
_BPP_32 = 32


def capture(host: str = "localhost", port: int = 5910, timeout: float = 5) -> FrameData:
    """Capture a single frame from the relay daemon."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.settimeout(timeout)
    try:
        sock.connect((host, port))
        # Read messages until we get a frame
        while True:
            # Read header: tag(1) + length(4)
            header = _recv_exact(sock, _HEADER_SIZE)
            tag = header[0]
            length = struct.unpack_from("<I", header, 1)[0]
            payload = _recv_exact(sock, length)

            if tag == _TAG_FRAME and length >= _FRAME_HEADER_SIZE:
                _seq, width, height, stride, bpp, brightness = struct.unpack_from(
                    "<QIIIIB", payload
                )
                pixels = payload[_FRAME_HEADER_SIZE:]
                return FrameData(
                    width=width,
                    height=height,
                    stride=stride,
                    bpp=bpp,
                    brightness=brightness,
                    pixels=pixels,
                )
    finally:
        sock.close()


@dataclass
class FrameData:
    """Raw framebuffer data from a single frame capture."""

    width: int
    height: int
    stride: int
    bpp: int
    brightness: int
    pixels: bytes

    def save_png(self, path: str | Path, *, rotate: bool = True) -> Path:
        """Save frame as a PNG file. Returns the path.

        The app renders at 480x1280 (portrait). With rotate=True (default),
        the image is rotated 90° clockwise for natural landscape viewing.
        """
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)

        rgba_rows = self._to_rgba_rows()

        if rotate:
            # Rotate 90° clockwise: (w,h) -> (h,w)
            # New row y comes from old column (width-1-y), reading top to bottom
            rotated: list[bytes] = []
            for x in range(self.width):
                row = bytearray()
                for y in range(self.height - 1, -1, -1):
                    src_off = x * 4
                    row.extend(rgba_rows[y][src_off : src_off + 4])
                rotated.append(bytes(row))
            png_data = _encode_png(self.height, self.width, rotated)
        else:
            png_data = _encode_png(self.width, self.height, rgba_rows)

        path.write_bytes(png_data)
        return path

    def _to_rgba_rows(self) -> list[bytes]:
        """Convert framebuffer pixels to RGBA rows."""
        rows: list[bytes] = []
        row_bytes = self.stride
        for y in range(self.height):
            row_start = y * row_bytes
            raw_row = self.pixels[row_start : row_start + row_bytes]

            if self.bpp == _BPP_32:
                rows.append(_bgra_to_rgba(raw_row, self.width))
            elif self.bpp == _BPP_16:
                rows.append(_rgb565_to_rgba(raw_row, self.width))
            else:
                msg = f"Unsupported bpp: {self.bpp}"
                raise ValueError(msg)
        return rows


# ── Pixel format conversion ───────────────────────────────────────────────────


def _bgra_to_rgba(raw_row: bytes, width: int) -> bytes:
    """Convert a BGRA row to RGBA."""
    rgba = bytearray(width * 4)
    for x in range(width):
        off = x * 4
        rgba[off] = raw_row[off + 2]  # R <- B
        rgba[off + 1] = raw_row[off + 1]  # G
        rgba[off + 2] = raw_row[off]  # B <- R
        rgba[off + 3] = raw_row[off + 3]  # A
    return bytes(rgba)


def _rgb565_to_rgba(raw_row: bytes, width: int) -> bytes:
    """Convert an RGB565 row to RGBA."""
    rgba = bytearray(width * 4)
    for x in range(width):
        pixel = struct.unpack_from("<H", raw_row, x * 2)[0]
        r = ((pixel >> 11) & 0x1F) * 255 // 31
        g = ((pixel >> 5) & 0x3F) * 255 // 63
        b = (pixel & 0x1F) * 255 // 31
        off = x * 4
        rgba[off] = r
        rgba[off + 1] = g
        rgba[off + 2] = b
        rgba[off + 3] = 255
    return bytes(rgba)


# ── Minimal PNG encoder (stdlib only) ──────────────────────────────────────────


def _encode_png(width: int, height: int, rgba_rows: list[bytes]) -> bytes:
    """Encode RGBA rows as a PNG file."""
    # Build raw image data: each row prefixed with filter byte 0 (no filter)
    raw = bytearray()
    for row in rgba_rows:
        raw.append(0)  # filter: none
        raw.extend(row)

    # Compress with zlib
    compressed = zlib.compress(bytes(raw))

    # Assemble PNG
    out = bytearray()
    out.extend(b"\x89PNG\r\n\x1a\n")  # PNG signature

    # IHDR: width, height, bit_depth=8, color_type=6 (RGBA)
    ihdr_data = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    out.extend(_png_chunk(b"IHDR", ihdr_data))

    # IDAT: compressed image data
    out.extend(_png_chunk(b"IDAT", compressed))

    # IEND
    out.extend(_png_chunk(b"IEND", b""))

    return bytes(out)


def _png_chunk(chunk_type: bytes, data: bytes) -> bytes:
    """Create a PNG chunk: length + type + data + CRC."""
    chunk = chunk_type + data
    return struct.pack(">I", len(data)) + chunk + struct.pack(">I", zlib.crc32(chunk) & 0xFFFFFFFF)


# ── Helpers ────────────────────────────────────────────────────────────────────


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    """Read exactly n bytes from a socket."""
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            msg = f"Connection closed, expected {n} bytes, got {len(buf)}"
            raise ConnectionError(msg)
        buf.extend(chunk)
    return bytes(buf)
