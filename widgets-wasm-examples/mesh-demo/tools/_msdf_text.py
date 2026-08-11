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

"""
Shared MSDF generation for numbered dice (D4/D8/D10/D12/D20).

Produces the same atlas layout the existing D*.py exporters set up via
Blender — same COLS×ROWS grid, same per-cell font centering — but encoded
as a single-channel signed distance field replicated across RGB so the
runtime's MSDF shader path samples median-of-RGB cleanly.

Build flow per face: render the digit into a binary mask at
`SUPERSAMPLE×` atlas resolution (so the boundary lives at sub-atlas-pixel
positions), compute the two-direction Euclidean distance transform via
8SSEDT (Felzenszwalb-style vector sweep, O(N²), pure Python), then box-
average the supersampled SDF down to atlas resolution. Cells with no
glyph stay at 0 → body color via the runtime's smoothstep + lerp.

Pure Python + PIL — no numpy, no scipy, no msdfgen dependency. Without
supersampling the distances would quantize to {0, 1, √2, 2, …} and show
visible concentric-ring banding in the PNG; supersampling moves the
banding below the encoded resolution.
"""

import array
import json
import math
import os
import sys
from typing import Iterable

from PIL import Image, ImageDraw, ImageFont

_TOOLS_DIR = os.path.dirname(os.path.abspath(__file__))
_FONT_DIR = os.path.normpath(
    os.path.join(_TOOLS_DIR, '..', '..', '..', '..', 'assets', 'fonts')
)
_FONT_PATH = os.path.join(_FONT_DIR, 'BraiinsSans-Bold.otf')

# Encoded SDF range — each atlas pixel of distance from the glyph edge
# maps to this many encoded units around 0.5. Wider = softer edge,
# narrower = sharper (and more wasted dynamic range on far-from-edge
# pixels). 4 px matches the runtime shader's smoothstep window.
DEFAULT_DISTANCE_RANGE_PX = 4.0

# Glyph mask + SDF are computed at `SUPERSAMPLE × atlas` resolution and
# box-downsampled. 4× moves the smallest distance step to 0.25 atlas px,
# below the 1/256 PNG quantization, eliminating ring banding. Higher is
# diminishing returns; lower brings the bands back.
SUPERSAMPLE = 4


def _font_size(cell_size: int) -> int:
    """Same heuristic as `_common.paint_number`: ~40 % of cell height."""
    return max(12, cell_size * 2 // 5)


def _glyph_mask(text: str, font_size_px: int) -> tuple[list[list[bool]], int, int]:
    """Render `text` to a tight binary mask at supersampled resolution.

    Returns `(mask, height, width)` in supersampled pixels — i.e. each
    boolean represents a 1/SUPERSAMPLE × 1/SUPERSAMPLE atlas-pixel
    fragment.
    """
    hi_font_size = font_size_px * SUPERSAMPLE
    font = ImageFont.truetype(_FONT_PATH, hi_font_size)
    bbox = font.getbbox(text)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    pad = 4 * SUPERSAMPLE
    img = Image.new('L', (tw + pad * 2, th + pad * 2), 0)
    draw = ImageDraw.Draw(img)
    draw.text((pad - bbox[0], pad - bbox[1]), text, fill=255, font=font)
    px = img.load()
    gw, gh = img.size
    mask = [[px[x, y] >= 128 for x in range(gw)] for y in range(gh)]
    return mask, gh, gw


def _edt_to_false(mask: list[list[bool]], h: int, w: int) -> list[list[float]]:
    """8SSEDT: distance to the nearest `False` pixel, in source pixels.

    Two-pass vector sweep (forward top-left→bottom-right, backward the
    other way), each pixel storing a `(dy, dx)` offset to its nearest
    False neighbour. The squared length of that vector is compared on
    relax; we only convert to scalar distance at the end. O(N²) regardless
    of how much of the mask is True.
    """
    inf = float('inf')
    dy: list[list[float]] = [
        [0.0 if not mask[y][x] else inf for x in range(w)] for y in range(h)
    ]
    dx: list[list[float]] = [
        [0.0 if not mask[y][x] else inf for x in range(w)] for y in range(h)
    ]

    def relax(y: int, x: int, oy: int, ox: int) -> None:
        ny, nx = y + oy, x + ox
        if 0 <= ny < h and 0 <= nx < w:
            ndy = dy[ny][nx] + oy
            ndx = dx[ny][nx] + ox
            cur_dy = dy[y][x]
            cur_dx = dx[y][x]
            if ndy * ndy + ndx * ndx < cur_dy * cur_dy + cur_dx * cur_dx:
                dy[y][x] = ndy
                dx[y][x] = ndx

    for y in range(h):
        for x in range(w):
            relax(y, x, -1, -1)
            relax(y, x, -1, 0)
            relax(y, x, -1, 1)
            relax(y, x, 0, -1)
        for x in range(w - 1, -1, -1):
            relax(y, x, 0, 1)

    for y in range(h - 1, -1, -1):
        for x in range(w - 1, -1, -1):
            relax(y, x, 1, -1)
            relax(y, x, 1, 0)
            relax(y, x, 1, 1)
            relax(y, x, 0, 1)
        for x in range(w):
            relax(y, x, 0, -1)

    return [[math.hypot(dy[y][x], dx[y][x]) for x in range(w)] for y in range(h)]


def _cell_sdf(
    glyph_mask: list[list[bool]],
    glyph_h: int,
    glyph_w: int,
    cell_h: int,
    cell_w: int,
    distance_range_px: float,
) -> list[list[float]]:
    """Compute the SDF for one cell at atlas resolution.

    Internally works at `SUPERSAMPLE × cell` resolution and box-averages
    to atlas size. Returns a `cell_h × cell_w` list of floats in [0..1]
    — 0.5 at the glyph boundary, higher inside (→ label color), lower
    outside.
    """
    cell_h_hi = cell_h * SUPERSAMPLE
    cell_w_hi = cell_w * SUPERSAMPLE

    # Place the supersampled glyph centered into the supersampled cell.
    cell_mask = [[False] * cell_w_hi for _ in range(cell_h_hi)]
    oy = (cell_h_hi - glyph_h) // 2
    ox = (cell_w_hi - glyph_w) // 2
    for gy in range(glyph_h):
        cy = gy + oy
        if 0 <= cy < cell_h_hi:
            row_in = glyph_mask[gy]
            row_out = cell_mask[cy]
            for gx in range(glyph_w):
                cx = gx + ox
                if 0 <= cx < cell_w_hi and row_in[gx]:
                    row_out[cx] = True

    # All-zero shortcut — empty cells encode to flat body-color.
    if not any(any(row) for row in cell_mask):
        return [[0.0] * cell_w for _ in range(cell_h)]

    dist_to_false = _edt_to_false(cell_mask, cell_h_hi, cell_w_hi)
    inverted = [[not v for v in row] for row in cell_mask]
    dist_to_true = _edt_to_false(inverted, cell_h_hi, cell_w_hi)

    # Encode in atlas-pixel units: source pixels / SUPERSAMPLE.
    inv_supersample = 1.0 / SUPERSAMPLE
    inv_range = 1.0 / distance_range_px
    inv_block = 1.0 / (SUPERSAMPLE * SUPERSAMPLE)

    out = [[0.0] * cell_w for _ in range(cell_h)]
    for ay in range(cell_h):
        ay_hi = ay * SUPERSAMPLE
        row_out = out[ay]
        for ax in range(cell_w):
            ax_hi = ax * SUPERSAMPLE
            acc = 0.0
            for sy in range(SUPERSAMPLE):
                hy = ay_hi + sy
                row_mask = cell_mask[hy]
                row_to_false = dist_to_false[hy]
                row_to_true = dist_to_true[hy]
                for sx in range(SUPERSAMPLE):
                    hx = ax_hi + sx
                    if row_mask[hx]:
                        signed = row_to_false[hx]
                    else:
                        signed = -row_to_true[hx]
                    normalized = signed * inv_supersample * inv_range
                    if normalized > 1.0:
                        normalized = 1.0
                    elif normalized < -1.0:
                        normalized = -1.0
                    acc += (normalized + 1.0) * 0.5
            row_out[ax] = acc * inv_block

    return out


def build_atlas(
    img_size: int,
    cols: int,
    rows: int,
    face_to_cell: dict[int, int],
    face_to_text: dict[int, str],
    distance_range_px: float = DEFAULT_DISTANCE_RANGE_PX,
) -> Image.Image:
    """Compose the full atlas as a PIL `RGBA` image.

    `face_to_cell`: face number → flat cell index (row-major).
    `face_to_text`: face number → label string (e.g. "20").
    """
    cell_h = img_size // rows
    cell_w = img_size // cols
    font_size = _font_size(min(cell_h, cell_w))

    # Single-channel atlas buffer (one byte per texel — the encoded SDF
    # value). Replicated to RGB at PNG-write time.
    atlas = array.array('B', bytes(img_size * img_size))
    for face, cell_idx in face_to_cell.items():
        col = cell_idx % cols
        row = cell_idx // cols
        glyph_mask, glyph_h, glyph_w = _glyph_mask(face_to_text[face], font_size)
        sdf = _cell_sdf(glyph_mask, glyph_h, glyph_w, cell_h, cell_w, distance_range_px)
        y0 = row * cell_h
        x0 = col * cell_w
        for ly in range(cell_h):
            row_sdf = sdf[ly]
            base = (y0 + ly) * img_size + x0
            for lx in range(cell_w):
                v = int(row_sdf[lx] * 255.0 + 0.5)
                if v < 0:
                    v = 0
                elif v > 255:
                    v = 255
                atlas[base + lx] = v

    rgba = bytearray(img_size * img_size * 4)
    for i, v in enumerate(atlas):
        j = i * 4
        rgba[j] = v
        rgba[j + 1] = v
        rgba[j + 2] = v
        rgba[j + 3] = 255
    return Image.frombytes('RGBA', (img_size, img_size), bytes(rgba))


def write_die(
    out_base: str,
    img_size: int,
    cols: int,
    rows: int,
    face_values: Iterable[int],
    body_color: tuple[int, int, int, int],
    label_color: tuple[int, int, int, int],
    *,
    cell_index: dict[int, int] | None = None,
    distance_range_px: float = DEFAULT_DISTANCE_RANGE_PX,
) -> None:
    """Generate `<out_base>.msdf.png` + `<out_base>.msdf.json` for one die.

    `face_values`: the integers to render, in face-number order.
    `cell_index`: optional override mapping face → flat cell index.
                  Defaults to `face - min(face_values)` (zero-based).
    """
    face_values = list(face_values)
    if cell_index is None:
        base = min(face_values)
        cell_index = {face: face - base for face in face_values}
    face_to_text = {face: str(face) for face in face_values}
    img = build_atlas(img_size, cols, rows, cell_index, face_to_text, distance_range_px)
    img.save(out_base + '.msdf.png', optimize=True)

    with open(out_base + '.msdf.json', 'w') as f:
        json.dump(
            {
                'body_color': list(body_color),
                'label_color': list(label_color),
            },
            f,
            indent=2,
        )
        f.write('\n')


def main_for_die(
    *,
    img_size: int,
    cols: int,
    rows: int,
    face_values: Iterable[int],
    body_color: tuple[int, int, int, int],
    label_color: tuple[int, int, int, int],
    cell_index: dict[int, int] | None = None,
) -> None:
    """Boilerplate `main()` for the per-die scripts.

    Usage: python tools/D<N>_msdf.py assets/D<N>
    """
    if len(sys.argv) != 2:
        print(f'Usage: python {sys.argv[0]} assets/D<N>', file=sys.stderr)
        sys.exit(1)
    out_base = os.path.abspath(sys.argv[1])
    write_die(
        out_base,
        img_size=img_size,
        cols=cols,
        rows=rows,
        face_values=face_values,
        body_color=body_color,
        label_color=label_color,
        cell_index=cell_index,
    )
    print(f'Wrote {out_base}.msdf.png + {out_base}.msdf.json')
