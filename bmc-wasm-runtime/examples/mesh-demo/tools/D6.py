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
Export a D6 (cube) as .glb with pip-textured faces.

Uses a simple cube with one UV island per face mapped to a 3x3 texture atlas.

Usage (full export via Blender):
    blender --background --python tools/D6.py -- assets/D6.glb

Usage (reset textures only, no Blender needed):
    python tools/D6.py --reset-texture assets/D6
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _common as common

# --- Texture atlas constants ---
# 3x3 grid. Rows 0-1 hold faces 1-6, row 2 is dead zone.

IMG_SIZE = 1_024
COLS = 3
ROWS = 3
CELL_W = IMG_SIZE // COLS
CELL_H = IMG_SIZE // ROWS

# All pip geometry is relative to CELL_W so it scales with IMG_SIZE.
_S = CELL_W / 170  # reference: cell=170px at IMG_SIZE=512

BG_COLOR = (0.92, 0.90, 0.85, 1.0)
PIP_COLOR = (0.12, 0.02, 0.02, 1.0)
PIP_RADIUS = round(14 * _S)

_P = round(35 * _S)  # pip offset from center
_Q = round(45 * _S)  # vertical offset for 6-pip layout
PIP_LAYOUTS = {
    1: [(0, 0)],
    2: [(-_P, -_P), (_P, _P)],
    3: [(-_P, -_P), (0, 0), (_P, _P)],
    4: [(-_P, -_P), (_P, -_P), (-_P, _P), (_P, _P)],
    5: [(-_P, -_P), (_P, -_P), (0, 0), (-_P, _P), (_P, _P)],
    6: [(-_P, -_Q), (_P, -_Q), (-_P, 0), (_P, 0), (-_P, _Q), (_P, _Q)],
}

# Normal map indent extends beyond pip for visible depression rim
INDENT_RADIUS = PIP_RADIUS + round(12 * _S)
INDENT_DEPTH = 0.6

FRAME_COLOR = (0.75, 0.72, 0.68, 1.0)


def _paint_circle(px_list: list[float], cx: int, cy: int, radius: int) -> None:
    """Paint an anti-aliased filled circle."""
    for oy in range(-radius - 1, radius + 2):
        for ox in range(-radius - 1, radius + 2):
            dist = math.sqrt(ox * ox + oy * oy)
            if dist > radius + 1:
                continue
            alpha = max(0.0, min(1.0, radius + 0.5 - dist))
            if alpha <= 0:
                continue
            px = cx + ox
            py = cy + oy
            if 0 <= px < IMG_SIZE and 0 <= py < IMG_SIZE:
                i = (py * IMG_SIZE + px) * 4
                px_list[i] += (PIP_COLOR[0] - px_list[i]) * alpha
                px_list[i + 1] += (PIP_COLOR[1] - px_list[i + 1]) * alpha
                px_list[i + 2] += (PIP_COLOR[2] - px_list[i + 2]) * alpha


def _set_normal(
    buf: list[float], px: int, py: int, nx: float, ny: float, nz: float
) -> None:
    """Set a pixel in the normal map buffer."""
    if 0 <= px < IMG_SIZE and 0 <= py < IMG_SIZE:
        i = (py * IMG_SIZE + px) * 4
        buf[i] = nx * 0.5 + 0.5
        buf[i + 1] = ny * 0.5 + 0.5
        buf[i + 2] = nz * 0.5 + 0.5


def generate_albedo() -> list[float]:
    """Generate the dice albedo texture atlas with pips and grid lines."""
    buf = [0.0] * (IMG_SIZE * IMG_SIZE * 4)
    common.fill_background(buf, IMG_SIZE, BG_COLOR)

    # Paint pips
    for face_num in range(1, 7):
        col = (face_num - 1) % COLS
        row = (face_num - 1) // COLS
        cx = col * CELL_W + CELL_W // 2
        cy = row * CELL_H + CELL_H // 2
        for pdx, pdy in PIP_LAYOUTS[face_num]:
            _paint_circle(buf, cx + pdx, cy + pdy, PIP_RADIUS)

    common.paint_grid_lines(buf, IMG_SIZE, COLS, ROWS, FRAME_COLOR)
    return buf


def generate_normal_map() -> list[float]:
    """Generate the dice normal map with pip indentations."""
    buf = [0.0] * (IMG_SIZE * IMG_SIZE * 4)
    common.fill_neutral_normal(buf, IMG_SIZE)

    # Pip indentations — normals point toward pip center (concave bowl)
    for face_num in range(1, 7):
        col = (face_num - 1) % COLS
        row = (face_num - 1) // COLS
        cx = col * CELL_W + CELL_W // 2
        cy = row * CELL_H + CELL_H // 2
        for pdx, pdy in PIP_LAYOUTS[face_num]:
            pip_cx = cx + pdx
            pip_cy = cy + pdy
            for oy in range(-INDENT_RADIUS - 1, INDENT_RADIUS + 2):
                for ox in range(-INDENT_RADIUS - 1, INDENT_RADIUS + 2):
                    dist = math.sqrt(ox * ox + oy * oy)
                    if dist > INDENT_RADIUS or dist < 0.001:
                        continue
                    falloff = max(0.0, 1.0 - dist / INDENT_RADIUS)
                    # Concave indent: nx tilts toward center in U, ny accounts
                    # for glTF V-flip (image Y and UV V point opposite directions)
                    nx = -ox / dist * falloff * INDENT_DEPTH
                    ny = oy / dist * falloff * INDENT_DEPTH
                    nz = math.sqrt(max(0.0, 1.0 - nx * nx - ny * ny))
                    _set_normal(buf, pip_cx + ox, pip_cy + oy, nx, ny, nz)

    return buf


# --- Entry point: --reset-texture mode (no Blender) ---

if '--reset-texture' in sys.argv:
    idx = sys.argv.index('--reset-texture')
    if idx + 1 >= len(sys.argv):
        print('Usage: python tools/D6.py --reset-texture assets/D6', file=sys.stderr)
        sys.exit(1)
    base_path = os.path.abspath(sys.argv[idx + 1])
    common.save_png(base_path + '.albedo.png', generate_albedo(), IMG_SIZE)
    common.save_png(base_path + '.normal.png', generate_normal_map(), IMG_SIZE)
    print(f'Reset textures: {base_path}.albedo.png, {base_path}.normal.png')
    sys.exit(0)

# --- Below here requires Blender ---

import bmesh  # type: ignore[import-not-found]  # noqa: E402
import bpy  # type: ignore[import-not-found]  # noqa: E402
import mathutils  # type: ignore[import-not-found]  # noqa: E402

output_path = common.parse_blender_args()
albedo_path, normal_path = common.ensure_textures(
    output_path, generate_albedo, generate_normal_map, IMG_SIZE
)

# --- Create dice mesh ---
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete()

bpy.ops.mesh.primitive_cube_add(size=1.0, location=(0, 0, 0))
cube = bpy.context.active_object
cube.name = 'Dice'

# --- UV map each face to its atlas cell ---
bpy.ops.object.mode_set(mode='EDIT')
bm = bmesh.from_edit_mesh(cube.data)
uv_layer = bm.loops.layers.uv.verify()

# Dice face mapping: normal direction → face number
# Standard: +Z=1, -Z=6, +X=2, -X=5, -Y=3, +Y=4
FACE_MAP = {
    (0, 0, 1): 1,
    (0, 0, -1): 6,
    (1, 0, 0): 2,
    (-1, 0, 0): 5,
    (0, -1, 0): 3,
    (0, 1, 0): 4,
}

for face in bm.faces:
    normal = face.normal
    abs_n = [abs(normal.x), abs(normal.y), abs(normal.z)]
    max_idx = abs_n.index(max(abs_n))
    sign = 1 if [normal.x, normal.y, normal.z][max_idx] > 0 else -1
    key = [0, 0, 0]
    key[max_idx] = sign
    face_num = FACE_MAP.get(tuple(key), 1)
    col = (face_num - 1) % COLS
    row = (face_num - 1) // COLS

    # Inset from cell edges; flip V for glTF (V=0 at bottom, PNG row 0 at top)
    margin = 3.0 / IMG_SIZE
    u_min = col / COLS + margin
    u_max = (col + 1) / COLS - margin
    v_min = 1.0 - (row + 1) / ROWS + margin
    v_max = 1.0 - row / ROWS - margin

    # Project face verts onto tangent plane for UV coords
    tangent_axes = [i for i in range(3) if i != max_idx]
    ta, tb = tangent_axes

    coords = [(loop.vert.co[ta], loop.vert.co[tb]) for loop in face.loops]
    min_a = min(co[0] for co in coords)
    max_a = max(co[0] for co in coords)
    min_b = min(co[1] for co in coords)
    max_b = max(co[1] for co in coords)
    range_a = max_a - min_a if max_a - min_a > 1e-6 else 1.0
    range_b = max_b - min_b if max_b - min_b > 1e-6 else 1.0

    # Per-axis U flip to match texture orientation when viewed from outside.
    # Y-axis tangent plane (X,Z) has opposite handedness from X/Z tangent planes.
    if max_idx == 1:
        flip_u = sign > 0  # Y: flip on positive
    else:
        flip_u = sign < 0  # X, Z: flip on negative

    for i, loop in enumerate(face.loops):
        t_a = (coords[i][0] - min_a) / range_a
        if flip_u:
            t_a = 1.0 - t_a
        t_b = (coords[i][1] - min_b) / range_b
        loop[uv_layer].uv = mathutils.Vector(
            (
                u_min + t_a * (u_max - u_min),
                v_min + t_b * (v_max - v_min),
            )
        )

bmesh.update_edit_mesh(cube.data)

# Apply bevel AFTER UV assignment — bevel faces interpolate UVs from original vertices.
bpy.ops.object.mode_set(mode='OBJECT')
bevel = cube.modifiers.new(name='Bevel', type='BEVEL')
bevel.width = 0.06
bevel.segments = 3
bevel.limit_method = 'ANGLE'
bevel.angle_limit = math.radians(30)
bpy.ops.object.modifier_apply(modifier=bevel.name)

# Smooth shading — auto-smooth keeps bevel edges smooth, flat face edges sharp.
bpy.ops.object.shade_auto_smooth()

# Material
common.setup_material(cube, albedo_path, normal_path, 'DiceMaterial')

# Embed face normals in glTF extras (glTF Y-up space) so include_mesh! can read them.
# Blender→glTF converts (x, y, z) → (x, z, -y). Ordered by face number 1..6.
# FACE_MAP: Blender normal → face number
d6_normals_flat = []
for face_num in range(1, 7):
    blender_n = next(k for k, v in FACE_MAP.items() if v == face_num)
    bx, by, bz = blender_n
    d6_normals_flat.extend([float(bx), float(bz), float(-by)])

common.finalize_and_export(cube, output_path, d6_normals_flat, 6)
