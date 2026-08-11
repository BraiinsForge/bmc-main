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
Export a D4 (tetrahedron) as .glb with numbered face texture.

Creates a 2×2 texture atlas with face numbers 1-4, then UV-maps each
triangular face to its atlas cell.

Usage (full export via Blender):
    blender --background --python tools/D4.py -- assets/D4.glb

Usage (reset textures only, no Blender needed):
    python tools/D4.py --reset-texture assets/D4
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _common as common

# --- Texture atlas constants ---
# 2×2 grid = 4 cells, all used.

IMG_SIZE = 1_024
COLS = 2
ROWS = 2
CELL_W = IMG_SIZE // COLS
CELL_H = IMG_SIZE // ROWS

BG_COLOR = (0.20, 0.08, 0.08, 1.0)  # dark red
TEXT_COLOR = (0.95, 0.90, 0.80, 1.0)  # cream
FRAME_COLOR = (0.30, 0.15, 0.15, 1.0)


def generate_albedo() -> list[float]:
    """Generate the D4 albedo texture atlas with face numbers."""
    buf = [0.0] * (IMG_SIZE * IMG_SIZE * 4)
    common.fill_background(buf, IMG_SIZE, BG_COLOR)

    for face_num in range(1, 5):
        col = (face_num - 1) % COLS
        row = (face_num - 1) // COLS
        cx = col * CELL_W + CELL_W // 2
        cy = row * CELL_H + CELL_H // 2
        common.paint_number(buf, face_num, cx, cy, IMG_SIZE, TEXT_COLOR, COLS)

    common.paint_grid_lines(buf, IMG_SIZE, COLS, ROWS, FRAME_COLOR)
    return buf


def generate_normal_map() -> list[float]:
    """Generate a D4 normal map with engraved number indentations."""
    return common.generate_sobel_normal_map(generate_albedo(), IMG_SIZE)


# --- Entry point: --reset-texture mode (no Blender) ---

if '--reset-texture' in sys.argv:
    idx = sys.argv.index('--reset-texture')
    if idx + 1 >= len(sys.argv):
        print('Usage: python tools/D4.py --reset-texture assets/D4', file=sys.stderr)
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

# --- Create D4 mesh (tetrahedron) via bmesh ---
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete()

mesh = bpy.data.meshes.new('D4Mesh')
obj = bpy.data.objects.new('D4', mesh)
bpy.context.collection.objects.link(obj)
bpy.context.view_layer.objects.active = obj
obj.select_set(True)

bm = bmesh.new()

# Regular tetrahedron: 4 vertices
s = 1.0
tetra_verts = [
    bm.verts.new((s, s, s)),
    bm.verts.new((s, -s, -s)),
    bm.verts.new((-s, s, -s)),
    bm.verts.new((-s, -s, s)),
]
bm.verts.ensure_lookup_table()

# 4 triangular faces — winding for outward normals
bm.faces.new([tetra_verts[0], tetra_verts[1], tetra_verts[2]])
bm.faces.new([tetra_verts[0], tetra_verts[3], tetra_verts[1]])
bm.faces.new([tetra_verts[0], tetra_verts[2], tetra_verts[3]])
bm.faces.new([tetra_verts[1], tetra_verts[3], tetra_verts[2]])

bm.faces.ensure_lookup_table()
bm.normal_update()
bm.to_mesh(mesh)
mesh.update()

# --- UV map each face ---
bpy.ops.object.mode_set(mode='EDIT')
bm = bmesh.from_edit_mesh(obj.data)
uv_layer = bm.loops.layers.uv.verify()

face_normals_list = [(f.index, (f.normal.x, f.normal.y, f.normal.z)) for f in bm.faces]
face_normals_list.sort(
    key=lambda fn: (round(fn[1][2], 2), round(fn[1][1], 2), round(fn[1][0], 2))
)

# D4 has no "opposite" convention — just assign 1-4 in sorted order
DISPLAY_NUMBER = [1, 2, 3, 4]

face_to_num = {}
for i, (fidx, _) in enumerate(face_normals_list):
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

    # Tangent frame
    normal = face.normal.normalized()
    world_up = mathutils.Vector((0, 0, 1))
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
bevel.width = 0.04
bevel.segments = 2
bevel.limit_method = 'ANGLE'
bevel.angle_limit = math.radians(20)
bpy.ops.object.modifier_apply(modifier=bevel.name)

# Smooth shading
bpy.ops.object.shade_auto_smooth()

# Material
common.setup_material(obj, albedo_path, normal_path, 'D4Material')

# Embed face normals: Blender (x,y,z) → glTF (x,z,-y), ordered by display number 1..4
normals_by_display = {}
for i, (_fidx, (nx, ny, nz)) in enumerate(face_normals_list):
    dnum = DISPLAY_NUMBER[i]
    normals_by_display[dnum] = [round(nx, 6), round(nz, 6), round(-ny, 6)]
face_normals_flat = []
for d in range(1, 5):
    face_normals_flat.extend(normals_by_display[d])

common.finalize_and_export(obj, output_path, face_normals_flat, 4)
