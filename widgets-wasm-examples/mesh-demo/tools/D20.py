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
Export a D20 (icosahedron) as .glb with numbered face texture.

Creates a 5×4 texture atlas with face numbers 1-20, then UV-maps each
triangular face to its atlas cell.

Usage (full export via Blender):
    blender --background --python tools/D20.py -- assets/D20.glb

Usage (reset textures only, no Blender needed):
    python tools/D20.py --reset-texture assets/D20
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _common as common

# --- Texture atlas constants ---
# 5×4 grid = 20 cells for faces 1-20, plus row 4 as dead zone for bevel.

IMG_SIZE = 1_024
COLS = 5
ROWS = 5  # 4 used rows + 1 dead zone
CELL_W = IMG_SIZE // COLS
CELL_H = IMG_SIZE // ROWS

BG_COLOR = (0.15, 0.08, 0.22, 1.0)  # deep purple
TEXT_COLOR = (0.95, 0.90, 0.80, 1.0)  # cream
FRAME_COLOR = (0.25, 0.15, 0.30, 1.0)


def generate_albedo() -> list[float]:
    """Generate the D20 albedo texture atlas with face numbers."""
    buf = [0.0] * (IMG_SIZE * IMG_SIZE * 4)
    common.fill_background(buf, IMG_SIZE, BG_COLOR)

    # Paint face numbers at cell centers. The UV mapping centers the triangle
    # centroid at (0.5, 0.5) in each cell, so cell center = face center.
    for face_num in range(1, 21):
        col = (face_num - 1) % COLS
        row = (face_num - 1) // COLS
        cx = col * CELL_W + CELL_W // 2
        cy = row * CELL_H + CELL_H // 2
        common.paint_number(buf, face_num, cx, cy, IMG_SIZE, TEXT_COLOR, COLS)

    common.paint_grid_lines(buf, IMG_SIZE, COLS, ROWS, FRAME_COLOR)
    return buf


def generate_normal_map() -> list[float]:
    """Generate a D20 normal map with engraved number indentations."""
    return common.generate_sobel_normal_map(generate_albedo(), IMG_SIZE)


# --- Entry point: --reset-texture mode (no Blender) ---

if '--reset-texture' in sys.argv:
    idx = sys.argv.index('--reset-texture')
    if idx + 1 >= len(sys.argv):
        print('Usage: python tools/D20.py --reset-texture assets/D20', file=sys.stderr)
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

# --- Create D20 mesh (icosahedron) ---
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete()

bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1, radius=0.5, location=(0, 0, 0))
ico = bpy.context.active_object
ico.name = 'D20'

# --- UV map each face to its atlas cell ---
bpy.ops.object.mode_set(mode='EDIT')
bm = bmesh.from_edit_mesh(ico.data)
uv_layer = bm.loops.layers.uv.verify()

# Sort faces by normal for deterministic ordering, then apply a permutation
# that gives a standard D20 layout: opposite faces sum to 21, adjacent faces
# have well-distributed numbers (min adjacent diff = 4).
face_normals = [(f.index, (f.normal.x, f.normal.y, f.normal.z)) for f in bm.faces]
face_normals.sort(
    key=lambda fn: (round(fn[1][2], 2), round(fn[1][1], 2), round(fn[1][0], 2))
)

# Permutation: sorted_index (0-based) → display number (1-based).
# Optimized so opposite faces sum to 21 and no adjacent pair is within 3.
DISPLAY_NUMBER = [3, 9, 10, 2, 6, 7, 5, 4, 8, 1, 20, 13, 17, 16, 14, 15, 19, 11, 12, 18]

face_to_num = {}
for i, (fidx, _) in enumerate(face_normals):
    face_to_num[fidx] = DISPLAY_NUMBER[i]

for face in bm.faces:
    face_num = face_to_num[face.index]
    col = (face_num - 1) % COLS
    row = (face_num - 1) // COLS

    # Flip V for glTF (V=0 at bottom, PNG row 0 at top)
    margin = 4.0 / IMG_SIZE
    u_min = col / COLS + margin
    u_max = (col + 1) / COLS - margin
    v_min = 1.0 - (row + 1) / ROWS + margin
    v_max = 1.0 - row / ROWS - margin

    # Compute tangent frame using projected world-up for consistent text orientation.
    # Must match lib.rs target_orientation() so text reads upright on screen.
    normal = face.normal.normalized()
    world_up = mathutils.Vector((0, 0, 1))  # Blender Z-up
    if abs(normal.dot(world_up)) > 0.95:
        world_up = mathutils.Vector((1, 0, 0))  # fallback for near-vertical faces
    tangent_v = (world_up - normal * world_up.dot(normal)).normalized()
    tangent_u = tangent_v.cross(normal).normalized()

    # Project vertices onto tangent plane, centered at face centroid.
    # Use centroid-centered mapping so the triangle center lands at cell center (0.5, 0.5).
    center = face.calc_center_median()
    coords = [
        (
            (loop.vert.co - center).dot(tangent_u),
            (loop.vert.co - center).dot(tangent_v),
        )
        for loop in face.loops
    ]
    # Centroid in tangent space is ~(0,0) since we subtracted center.
    # Map so centroid → 0.5, max extent → edges of cell.
    centroid_a = sum(co[0] for co in coords) / len(coords)
    centroid_b = sum(co[1] for co in coords) / len(coords)
    half_a = max(abs(co[0] - centroid_a) for co in coords)
    half_b = max(abs(co[1] - centroid_b) for co in coords)
    half_a = half_a if half_a > 1e-6 else 1.0
    half_b = half_b if half_b > 1e-6 else 1.0
    half = max(half_a, half_b)

    for i, loop in enumerate(face.loops):
        t_a = 0.5 + (coords[i][0] - centroid_a) / (2 * half)
        t_b = 0.5 + (coords[i][1] - centroid_b) / (2 * half)
        loop[uv_layer].uv = mathutils.Vector(
            (
                u_min + t_a * (u_max - u_min),
                v_min + t_b * (v_max - v_min),
            )
        )

bmesh.update_edit_mesh(ico.data)

# Bevel edges slightly for smoother look
bpy.ops.object.mode_set(mode='OBJECT')
bevel = ico.modifiers.new(name='Bevel', type='BEVEL')
bevel.width = 0.015
bevel.segments = 2
bevel.limit_method = 'ANGLE'
bevel.angle_limit = math.radians(20)
bpy.ops.object.modifier_apply(modifier=bevel.name)

# Smooth shading
bpy.ops.object.shade_auto_smooth()

# Material
common.setup_material(ico, albedo_path, normal_path, 'D20Material')

# Embed face normals in glTF extras (glTF Y-up space) so include_mesh! can read them.
# Blender→glTF converts (x, y, z) → (x, z, -y). Ordered by display number 1..20.
normals_by_display = {}
for i, (_fidx, (nx, ny, nz)) in enumerate(face_normals):
    dnum = DISPLAY_NUMBER[i]
    normals_by_display[dnum] = [round(nx, 6), round(nz, 6), round(-ny, 6)]
# Flat array: [x1,y1,z1, x2,y2,z2, ...] — glTF extras don't support nested arrays well
face_normals_flat = []
for d in range(1, 21):
    face_normals_flat.extend(normals_by_display[d])

common.finalize_and_export(ico, output_path, face_normals_flat, 20)
