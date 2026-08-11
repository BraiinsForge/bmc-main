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
Export a D8 (octahedron) as .glb with numbered face texture.

Creates a 3×3 texture atlas with face numbers 1-8, then UV-maps each
triangular face to its atlas cell.

Usage (full export via Blender):
    blender --background --python tools/D8.py -- assets/D8.glb

Usage (reset textures only, no Blender needed):
    python tools/D8.py --reset-texture assets/D8
"""

import math
import os
import sys

# Add tools dir to path for _common import
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _common as common

# --- Texture atlas constants ---
# 3×3 grid = 9 cells, 8 used for faces 1-8, 1 dead zone.

IMG_SIZE = 1_024
COLS = 3
ROWS = 3
CELL_W = IMG_SIZE // COLS
CELL_H = IMG_SIZE // ROWS

BG_COLOR = (0.12, 0.12, 0.18, 1.0)  # dark blue-gray
TEXT_COLOR = (0.95, 0.90, 0.80, 1.0)  # cream
FRAME_COLOR = (0.22, 0.20, 0.28, 1.0)


def generate_albedo() -> list[float]:
    """Generate the D8 albedo texture atlas with face numbers."""
    buf = [0.0] * (IMG_SIZE * IMG_SIZE * 4)
    common.fill_background(buf, IMG_SIZE, BG_COLOR)

    for face_num in range(1, 9):
        col = (face_num - 1) % COLS
        row = (face_num - 1) // COLS
        cx = col * CELL_W + CELL_W // 2
        cy = row * CELL_H + CELL_H // 2
        common.paint_number(buf, face_num, cx, cy, IMG_SIZE, TEXT_COLOR, COLS)

    common.paint_grid_lines(buf, IMG_SIZE, COLS, ROWS, FRAME_COLOR)
    return buf


def generate_normal_map() -> list[float]:
    """Generate a D8 normal map with engraved number indentations."""
    return common.generate_sobel_normal_map(generate_albedo(), IMG_SIZE)


# --- Entry point: --reset-texture mode (no Blender) ---

if '--reset-texture' in sys.argv:
    idx = sys.argv.index('--reset-texture')
    if idx + 1 >= len(sys.argv):
        print('Usage: python tools/D8.py --reset-texture assets/D8', file=sys.stderr)
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

# --- Create D8 mesh (octahedron) via bmesh ---
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete()

mesh = bpy.data.meshes.new('D8Mesh')
obj = bpy.data.objects.new('D8', mesh)
bpy.context.collection.objects.link(obj)
bpy.context.view_layer.objects.active = obj
obj.select_set(True)

bm = bmesh.new()

# 6 vertices: ±1 along each axis
verts = [
    bm.verts.new((1, 0, 0)),  # 0: +X
    bm.verts.new((-1, 0, 0)),  # 1: -X
    bm.verts.new((0, 1, 0)),  # 2: +Y
    bm.verts.new((0, -1, 0)),  # 3: -Y
    bm.verts.new((0, 0, 1)),  # 4: +Z
    bm.verts.new((0, 0, -1)),  # 5: -Z
]
bm.verts.ensure_lookup_table()

# 8 triangular faces — winding order for outward normals
# Upper hemisphere (+Z)
bm.faces.new([verts[0], verts[2], verts[4]])  # +X +Y +Z
bm.faces.new([verts[2], verts[1], verts[4]])  # -X +Y +Z
bm.faces.new([verts[1], verts[3], verts[4]])  # -X -Y +Z
bm.faces.new([verts[3], verts[0], verts[4]])  # +X -Y +Z
# Lower hemisphere (-Z)
bm.faces.new([verts[2], verts[0], verts[5]])  # +X +Y -Z
bm.faces.new([verts[1], verts[2], verts[5]])  # -X +Y -Z
bm.faces.new([verts[3], verts[1], verts[5]])  # -X -Y -Z
bm.faces.new([verts[0], verts[3], verts[5]])  # +X -Y -Z

bm.faces.ensure_lookup_table()
bm.to_mesh(mesh)
mesh.update()

# --- Sort faces and assign display numbers ---
bpy.ops.object.mode_set(mode='EDIT')
bm = bmesh.from_edit_mesh(obj.data)
uv_layer = bm.loops.layers.uv.verify()

face_normals = [(f.index, (f.normal.x, f.normal.y, f.normal.z)) for f in bm.faces]
face_normals.sort(
    key=lambda fn: (round(fn[1][2], 2), round(fn[1][1], 2), round(fn[1][0], 2))
)

# Permutation: opposite faces sum to 9.
# Sorted normals give a predictable order; assign numbers so that
# diametrically opposite faces (negated normal) sum to 9.
# We pair them manually after inspecting sorted order.
DISPLAY_NUMBER = [6, 2, 7, 3, 5, 1, 8, 4]

face_to_num = {}
for i, (fidx, _) in enumerate(face_normals):
    face_to_num[fidx] = DISPLAY_NUMBER[i]

for face in bm.faces:
    face_num = face_to_num[face.index]
    col = (face_num - 1) % COLS
    row = (face_num - 1) // COLS

    margin = 4.0 / IMG_SIZE
    u_min = col / COLS + margin
    u_max = (col + 1) / COLS - margin
    v_min = 1.0 - (row + 1) / ROWS + margin
    v_max = 1.0 - row / ROWS - margin

    # Tangent frame for consistent text orientation (matches lib.rs)
    normal = face.normal.normalized()
    world_up = mathutils.Vector((0, 0, 1))  # Blender Z-up
    if abs(normal.dot(world_up)) > 0.95:
        world_up = mathutils.Vector((1, 0, 0))
    tangent_v = (world_up - normal * world_up.dot(normal)).normalized()
    tangent_u = tangent_v.cross(normal).normalized()

    center = face.calc_center_median()
    coords = [
        (
            (loop.vert.co - center).dot(tangent_u),
            (loop.vert.co - center).dot(tangent_v),
        )
        for loop in face.loops
    ]
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

bmesh.update_edit_mesh(obj.data)

# Bevel
bpy.ops.object.mode_set(mode='OBJECT')
bevel = obj.modifiers.new(name='Bevel', type='BEVEL')
bevel.width = 0.03
bevel.segments = 2
bevel.limit_method = 'ANGLE'
bevel.angle_limit = math.radians(20)
bpy.ops.object.modifier_apply(modifier=bevel.name)

# Smooth shading
bpy.ops.object.shade_auto_smooth()

# Material
common.setup_material(obj, albedo_path, normal_path, 'D8Material')

# Embed face normals: Blender (x,y,z) → glTF (x,z,-y), ordered by display number 1..8
normals_by_display = {}
for i, (_fidx, (nx, ny, nz)) in enumerate(face_normals):
    dnum = DISPLAY_NUMBER[i]
    normals_by_display[dnum] = [round(nx, 6), round(nz, 6), round(-ny, 6)]
face_normals_flat = []
for d in range(1, 9):
    face_normals_flat.extend(normals_by_display[d])

common.finalize_and_export(obj, output_path, face_normals_flat, 8)
