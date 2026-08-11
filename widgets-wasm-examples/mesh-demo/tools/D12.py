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
Export a D12 (dodecahedron) as .glb with numbered face texture.

Creates a 4×3 texture atlas with face numbers 1-12, then UV-maps each
pentagonal face to its atlas cell before triangulating.

Usage (full export via Blender):
    blender --background --python tools/D12.py -- assets/D12.glb

Usage (reset textures only, no Blender needed):
    python tools/D12.py --reset-texture assets/D12
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import _common as common

# --- Texture atlas constants ---
# 4×3 grid = 12 cells, all used.

IMG_SIZE = 1_024
COLS = 4
ROWS = 3
CELL_W = IMG_SIZE // COLS
CELL_H = IMG_SIZE // ROWS

BG_COLOR = (0.08, 0.18, 0.12, 1.0)  # dark green
TEXT_COLOR = (0.95, 0.90, 0.80, 1.0)  # cream
FRAME_COLOR = (0.15, 0.28, 0.20, 1.0)


def generate_albedo() -> list[float]:
    """Generate the D12 albedo texture atlas with face numbers."""
    buf = [0.0] * (IMG_SIZE * IMG_SIZE * 4)
    common.fill_background(buf, IMG_SIZE, BG_COLOR)

    for face_num in range(1, 13):
        col = (face_num - 1) % COLS
        row = (face_num - 1) // COLS
        cx = col * CELL_W + CELL_W // 2
        cy = row * CELL_H + CELL_H // 2
        common.paint_number(buf, face_num, cx, cy, IMG_SIZE, TEXT_COLOR, COLS)

    common.paint_grid_lines(buf, IMG_SIZE, COLS, ROWS, FRAME_COLOR)
    return buf


def generate_normal_map() -> list[float]:
    """Generate a D12 normal map with engraved number indentations."""
    return common.generate_sobel_normal_map(generate_albedo(), IMG_SIZE)


# --- Entry point: --reset-texture mode (no Blender) ---

if '--reset-texture' in sys.argv:
    idx = sys.argv.index('--reset-texture')
    if idx + 1 >= len(sys.argv):
        print('Usage: python tools/D12.py --reset-texture assets/D12', file=sys.stderr)
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

# --- Create D12 mesh (dodecahedron) via bmesh ---
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete()

mesh = bpy.data.meshes.new('D12Mesh')
obj = bpy.data.objects.new('D12', mesh)
bpy.context.collection.objects.link(obj)
bpy.context.view_layer.objects.active = obj
obj.select_set(True)

bm = bmesh.new()

# Dodecahedron: 20 vertices using golden ratio coordinates
phi = (1 + math.sqrt(5)) / 2
inv_phi = 1 / phi

# Three groups of vertices (cube + rectangles)
dodeca_verts = [
    # 8 cube vertices (±1, ±1, ±1)
    (1, 1, 1),
    (1, 1, -1),
    (1, -1, 1),
    (1, -1, -1),
    (-1, 1, 1),
    (-1, 1, -1),
    (-1, -1, 1),
    (-1, -1, -1),
    # 4 vertices on XY plane (0, ±phi, ±1/phi)
    (0, phi, inv_phi),
    (0, phi, -inv_phi),
    (0, -phi, inv_phi),
    (0, -phi, -inv_phi),
    # 4 vertices on YZ plane (±1/phi, 0, ±phi)
    (inv_phi, 0, phi),
    (-inv_phi, 0, phi),
    (inv_phi, 0, -phi),
    (-inv_phi, 0, -phi),
    # 4 vertices on XZ plane (±phi, ±1/phi, 0)
    (phi, inv_phi, 0),
    (phi, -inv_phi, 0),
    (-phi, inv_phi, 0),
    (-phi, -inv_phi, 0),
]

bm_verts = [bm.verts.new(v) for v in dodeca_verts]
bm.verts.ensure_lookup_table()

# Compute faces from edge graph: two vertices are adjacent iff distance = 2/φ.
# This is robust — no hand-guessed face indices.
edge_len = 2.0 / phi
tolerance = 0.01

# Build adjacency list
from collections import defaultdict  # noqa: E402

adj: dict[int, list[int]] = defaultdict(list)
for i in range(len(dodeca_verts)):
    for j in range(i + 1, len(dodeca_verts)):
        vi = mathutils.Vector(dodeca_verts[i])
        vj = mathutils.Vector(dodeca_verts[j])
        if abs((vi - vj).length - edge_len) < tolerance:
            adj[i].append(j)
            adj[j].append(i)

# Find pentagonal faces by walking 5-cycles in the edge graph.


def _find_pentagons(adjacency):
    """Walk all 5-cycles a→b→c→d→e→a and return unique sorted face tuples."""
    faces = set()
    for a in adjacency:
        for b in adjacency[a]:
            for c in adjacency[b]:
                if c == a:
                    continue
                _walk_cd(adjacency, faces, a, b, c)
    return faces


def _walk_cd(adjacency, faces, a, b, c):
    for d in adjacency[c]:
        if d in (b, a):
            continue
        for e in adjacency[d]:
            if e in (c, b):
                continue
            if a in adjacency[e]:
                faces.add(tuple(sorted([a, b, c, d, e])))


found_faces = _find_pentagons(adj)

assert len(found_faces) == 12, f'Expected 12 faces, got {len(found_faces)}'

# Order vertices in each face by angle around face centroid for proper winding
for face_indices in found_faces:
    pts = [mathutils.Vector(dodeca_verts[i]) for i in face_indices]
    centroid = sum(pts, mathutils.Vector()) / len(pts)
    normal = (centroid).normalized()  # outward from origin for convex solid
    # Sort by angle around normal
    ref = (pts[0] - centroid).normalized()
    cross_ref = normal.cross(ref)

    def angle_key(idx):
        v = mathutils.Vector(dodeca_verts[idx]) - centroid
        return math.atan2(v.dot(cross_ref), v.dot(ref))

    ordered = sorted(face_indices, key=angle_key)
    bm.faces.new([bm_verts[i] for i in ordered])

bm.faces.ensure_lookup_table()
bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
bm.normal_update()

bm.to_mesh(mesh)
mesh.update()

# --- UV map each pentagonal face BEFORE triangulation ---
bpy.ops.object.mode_set(mode='EDIT')
bm = bmesh.from_edit_mesh(obj.data)
uv_layer = bm.loops.layers.uv.verify()

face_normals_list = [(f.index, (f.normal.x, f.normal.y, f.normal.z)) for f in bm.faces]
face_normals_list.sort(
    key=lambda fn: (round(fn[1][2], 2), round(fn[1][1], 2), round(fn[1][0], 2))
)

# Permutation: opposite faces sum to 13
DISPLAY_NUMBER = [10, 4, 8, 2, 6, 12, 1, 7, 11, 5, 9, 3]

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

# Bevel AFTER UV assignment, BEFORE triangulation
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
common.setup_material(obj, albedo_path, normal_path, 'D12Material')

# Embed face normals: Blender (x,y,z) → glTF (x,z,-y), ordered by display number 1..12
normals_by_display = {}
for i, (_fidx, (nx, ny, nz)) in enumerate(face_normals_list):
    dnum = DISPLAY_NUMBER[i]
    normals_by_display[dnum] = [round(nx, 6), round(nz, 6), round(-ny, 6)]
face_normals_flat = []
for d in range(1, 13):
    face_normals_flat.extend(normals_by_display[d])

common.finalize_and_export(obj, output_path, face_normals_flat, 12)
