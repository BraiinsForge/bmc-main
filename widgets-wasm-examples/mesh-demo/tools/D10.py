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
Export a D10 (pentagonal trapezohedron) as .glb with numbered face texture.

Creates a 5×2 texture atlas with face values 0-9, then UV-maps each
kite face to its atlas cell before triangulating.

The D10 uses 0-9 numbering (not 1-10). Internally faces are 1-indexed
in the face_normals array; the Rust side maps face N → value N-1.

Usage (full export via Blender):
    blender --background --python tools/D10.py -- assets/D10.glb

Usage (reset textures only, no Blender needed):
    python tools/D10.py --reset-texture assets/D10
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _common as common

# --- Texture atlas constants ---
# 5×3 grid = 15 cells, 10 used. Cell aspect (204×341) ≈ 1:1.67 matches
# the kite face's golden-ratio aspect (1:φ ≈ 1:1.618) to avoid text distortion.

IMG_SIZE = 1_024
COLS = 5
ROWS = 3
CELL_W = IMG_SIZE // COLS
CELL_H = IMG_SIZE // ROWS

BG_COLOR = (0.18, 0.10, 0.22, 1.0)  # dark purple
TEXT_COLOR = (0.95, 0.90, 0.80, 1.0)  # cream
FRAME_COLOR = (0.28, 0.18, 0.32, 1.0)


def generate_albedo() -> list[float]:
    """Generate the D10 albedo texture atlas with face values 0-9."""
    buf = [0.0] * (IMG_SIZE * IMG_SIZE * 4)
    common.fill_background(buf, IMG_SIZE, BG_COLOR)

    # Face values 0-9, stored in cells 0-9 (cell index = face value)
    for value in range(10):
        col = value % COLS
        row = value // COLS
        cx = col * CELL_W + CELL_W // 2
        cy = row * CELL_H + CELL_H // 2
        common.paint_number(buf, value, cx, cy, IMG_SIZE, TEXT_COLOR, COLS)

    common.paint_grid_lines(buf, IMG_SIZE, COLS, ROWS, FRAME_COLOR)
    return buf


def generate_normal_map() -> list[float]:
    """Generate a D10 normal map with engraved number indentations."""
    return common.generate_sobel_normal_map(generate_albedo(), IMG_SIZE)


# --- Entry point: --reset-texture mode (no Blender) ---

if '--reset-texture' in sys.argv:
    idx = sys.argv.index('--reset-texture')
    if idx + 1 >= len(sys.argv):
        print('Usage: python tools/D10.py --reset-texture assets/D10', file=sys.stderr)
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

# --- Create D10 mesh (pentagonal trapezohedron) via bmesh ---
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete()

mesh = bpy.data.meshes.new('D10Mesh')
obj = bpy.data.objects.new('D10', mesh)
bpy.context.collection.objects.link(obj)
bpy.context.view_layer.objects.active = obj
obj.select_set(True)

bm = bmesh.new()

# Pentagonal trapezohedron: 12 vertices
# 2 apex vertices + 2 rings of 5 (twisted 36° = pi/5)
# Regular solid: all edges equal, kite faces have golden-ratio aspect (φ:1).
cos72 = math.cos(math.radians(72))
cos36 = math.cos(math.radians(36))
ring_r = 1.0 / math.sqrt(2.0 - 2.0 * cos72)  # ≈ 0.851
ring_h = math.sqrt((1.0 - 2.0 * ring_r**2 * (1.0 - cos36)) / 4.0)  # ≈ 0.425
apex_h = ring_h + math.sqrt(max(0.0, 1.0 - ring_r**2))  # ≈ 0.951

top_apex = bm.verts.new((0, 0, apex_h))  # vertex 0
bot_apex = bm.verts.new((0, 0, -apex_h))  # vertex 1

# Upper ring (5 vertices) at angle offsets 0, 72, 144, 216, 288 degrees
upper_ring = []
for i in range(5):
    angle = math.radians(i * 72)
    v = bm.verts.new((ring_r * math.cos(angle), ring_r * math.sin(angle), ring_h))
    upper_ring.append(v)

# Lower ring (5 vertices) twisted by 36° (half of 72°)
lower_ring = []
for i in range(5):
    angle = math.radians(i * 72 + 36)
    v = bm.verts.new((ring_r * math.cos(angle), ring_r * math.sin(angle), -ring_h))
    lower_ring.append(v)

bm.verts.ensure_lookup_table()

# 10 kite (quadrilateral) faces
# Upper faces: top_apex → upper[i] → lower[i] → upper[(i+1)%5]
# Lower faces: bot_apex → lower[(i+1)%5] → upper[(i+1)%5] → lower[i]
for i in range(5):
    ni = (i + 1) % 5
    # Upper kite: apex-top, upper[i], lower[i], upper[next]
    bm.faces.new([top_apex, upper_ring[i], lower_ring[i], upper_ring[ni]])
    # Lower kite: apex-bot, lower[next], upper[next], lower[i]
    bm.faces.new([bot_apex, lower_ring[ni], upper_ring[ni], lower_ring[i]])

bm.faces.ensure_lookup_table()
bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
bm.normal_update()
bm.to_mesh(mesh)
mesh.update()

# --- UV map each kite face BEFORE triangulation ---
bpy.ops.object.mode_set(mode='EDIT')
bm = bmesh.from_edit_mesh(obj.data)
uv_layer = bm.loops.layers.uv.verify()

face_normals_list = [(f.index, (f.normal.x, f.normal.y, f.normal.z)) for f in bm.faces]
face_normals_list.sort(
    key=lambda fn: (round(fn[1][2], 2), round(fn[1][1], 2), round(fn[1][0], 2))
)

# Permutation: opposite faces sum to 9 (values 0-9, so face 1 shows "0", face 10 shows "9")
# Faces are 1-indexed internally but display values 0-9.
# Atlas cell index = display value = face_index - 1
DISPLAY_NUMBER = [5, 1, 9, 3, 7, 6, 10, 2, 8, 4]

face_to_num = {}
for i, (fidx, _) in enumerate(face_normals_list):
    face_to_num[fidx] = DISPLAY_NUMBER[i]

for face in bm.faces:
    face_num = face_to_num[face.index]
    # Atlas cell = face_num - 1 (so face 1 → cell 0 → value "0")
    cell = face_num - 1
    col = cell % COLS
    row = cell // COLS

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

    for i, loop in enumerate(face.loops):
        t_a = 0.5 + (coords[i][0] - centroid_a) / (2 * half_a)
        t_b = 0.5 + (coords[i][1] - centroid_b) / (2 * half_b)
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
bevel.width = 0.02
bevel.segments = 2
bevel.limit_method = 'ANGLE'
bevel.angle_limit = math.radians(20)
bpy.ops.object.modifier_apply(modifier=bevel.name)

# Smooth shading
bpy.ops.object.shade_auto_smooth()

# Material
common.setup_material(obj, albedo_path, normal_path, 'D10Material')

# Embed face normals: Blender (x,y,z) → glTF (x,z,-y), ordered by display number 1..10
normals_by_display = {}
for i, (_fidx, (nx, ny, nz)) in enumerate(face_normals_list):
    dnum = DISPLAY_NUMBER[i]
    normals_by_display[dnum] = [round(nx, 6), round(nz, 6), round(-ny, 6)]
face_normals_flat = []
for d in range(1, 11):
    face_normals_flat.extend(normals_by_display[d])

common.finalize_and_export(obj, output_path, face_normals_flat, 10)
