#!/usr/bin/env python3
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
Generate D6 MSDF assets — SDF texture + body/label color sidecar.

The pip atlas is encoded as a single-channel signed distance field
replicated across RGB (the runtime's MSDF shader path takes the median
of RGB; for circles with no corner concerns one channel suffices, and
replicating keeps the binary format consistent with future numbered dice
that need true MSDF).

Output:
    assets/D6.msdf.png   — RGBA8 atlas, R=G=B=encoded_distance, A=255
    assets/D6.msdf.json  — body_color + label_color metadata

Usage:
    python tools/D6_msdf.py assets/D6
"""

import json
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _common as common

# --- Atlas layout: matches the existing D6.glb UV mapping (3×3 grid). ---

IMG_SIZE = 256
COLS = 3
ROWS = 3
CELL_W = IMG_SIZE // COLS
CELL_H = IMG_SIZE // ROWS

# Pip geometry — proportions copied from tools/D6.py and rescaled to
# IMG_SIZE so the SDF lines up with the cube's UV cells.
_S = CELL_W / 170  # reference: cell=170 px at IMG_SIZE=512 (per D6.py)
PIP_RADIUS = 14 * _S
_P = round(35 * _S)  # pip offset from center
_Q = round(45 * _S)  # vertical offset for 6-pip layout
PIP_LAYOUTS: dict[int, list[tuple[int, int]]] = {
    1: [(0, 0)],
    2: [(-_P, -_P), (_P, _P)],
    3: [(-_P, -_P), (0, 0), (_P, _P)],
    4: [(-_P, -_P), (_P, -_P), (-_P, _P), (_P, _P)],
    5: [(-_P, -_P), (_P, -_P), (0, 0), (-_P, _P), (_P, _P)],
    6: [(-_P, -_Q), (_P, -_Q), (-_P, 0), (_P, 0), (-_P, _Q), (_P, _Q)],
}

# Distance range for SDF encoding (in pixels). At each texel we store
# (PIP_RADIUS - distance_to_nearest_pip_center) / DISTANCE_RANGE, mapped
# from [-1..1] to [0..1] for the PNG. Wider range = smoother gradient
# but more wasted dynamic range on far-from-edge texels. ~3px gives a
# crisp ~1-2px AA window after the shader's smoothstep(0.5±0.04).
DISTANCE_RANGE = 3.0

# Colors copied from tools/D6.py — body is the dice base color, label
# is the pip color. Stored as RGBA u8 in the sidecar JSON; the runtime
# converts to f32 [0..1] when uploading uniforms.
BODY_COLOR_RGBA = (235, 230, 217, 255)  # was (0.92, 0.90, 0.85, 1.0)
LABEL_COLOR_RGBA = (31, 5, 5, 255)  # was (0.12, 0.02, 0.02, 1.0)

# --- SDF encoding ---


def _signed_distance_to_pip(px: float, py: float) -> float:
    """Signed distance from (px, py) to the nearest pip in its own cell.

    Negative inside the pip, positive outside. Returns +inf when the
    pixel is outside the 3×3 active area or in a cell with no pips
    (faces 7-9 don't exist on a D6).
    """
    col = int(px) // CELL_W
    row = int(py) // CELL_H
    if col < 0 or col >= COLS or row < 0 or row >= ROWS:
        return math.inf
    face_num = row * COLS + col + 1
    pips = PIP_LAYOUTS.get(face_num)
    if not pips:
        return math.inf
    cx = col * CELL_W + CELL_W // 2
    cy = row * CELL_H + CELL_H // 2
    min_d = math.inf
    for pdx, pdy in pips:
        pcx = cx + pdx
        pcy = cy + pdy
        d = math.hypot(px - pcx, py - pcy)
        if d < min_d:
            min_d = d
    return min_d - PIP_RADIUS


def generate_msdf() -> list[float]:
    """Produce an RGBA float buffer (4 floats per pixel) for save_png.

    Encoding: pip-interior gets high values (→ label_color via the
    shader's smoothstep at 0.5), body gets low values (→ body_color).
    """
    buf = [0.0] * (IMG_SIZE * IMG_SIZE * 4)
    for py in range(IMG_SIZE):
        for px in range(IMG_SIZE):
            sd = _signed_distance_to_pip(px + 0.5, py + 0.5)
            # Sign-flipped so inside-pip → high value (label).
            normalized = -sd / DISTANCE_RANGE
            clamped = max(-1.0, min(1.0, normalized))
            value = (clamped + 1.0) * 0.5  # [0..1]
            i = (py * IMG_SIZE + px) * 4
            buf[i] = value
            buf[i + 1] = value
            buf[i + 2] = value
            buf[i + 3] = 1.0
    return buf


# --- Entry point ---


def main() -> None:
    if len(sys.argv) != 2:
        print('Usage: python tools/D6_msdf.py assets/D6', file=sys.stderr)
        sys.exit(1)
    base_path = os.path.abspath(sys.argv[1])
    common.save_png(base_path + '.msdf.png', generate_msdf(), IMG_SIZE)
    sidecar = {
        'body_color': list(BODY_COLOR_RGBA),
        'label_color': list(LABEL_COLOR_RGBA),
    }
    with open(base_path + '.msdf.json', 'w') as f:
        json.dump(sidecar, f, indent=2)
        f.write('\n')
    print(f'Wrote {base_path}.msdf.png + {base_path}.msdf.json')


if __name__ == '__main__':
    main()
