#!/usr/bin/env nix
#!nix shell nixpkgs#python3
#!nix --command python3
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

"""Wrapper script that invokes Blender headlessly to preprocess a 3D model.

Imports a model file, decimates to target triangle count, triangulates,
recalculates normals, centers origin, normalizes scale to unit bounding box,
Smart UV projects if missing UVs, and exports as .glb.

Supports any format Blender can import: STL, OBJ, FBX, PLY, glTF/glb, etc.

Usage:
    ./scripts/prepare_model.py input.stl output.glb [--max-tris 3000] [--texture path/to/texture.png]

Requires Blender installed via flatpak (org.blender.Blender) or on PATH.
"""

import argparse
import os
import shutil
import subprocess
import sys
import tempfile


def find_blender() -> str:
    """Find a Blender executable — prefer flatpak, fall back to PATH."""
    result = subprocess.run(
        ['flatpak', 'list', '--columns=application'],
        capture_output=True,
        text=True,
    )
    if result.returncode == 0 and 'org.blender.Blender' in result.stdout:
        return 'flatpak'
    blender = shutil.which('blender')
    if blender:
        return blender
    print(
        'ERROR: Blender not found. Install via flatpak or add to PATH.', file=sys.stderr
    )
    sys.exit(1)


def main() -> None:
    parser = argparse.ArgumentParser(
        description='Prepare a 3D model for the WASM widget pipeline via Blender.'
    )
    parser.add_argument(
        'input', help='Input model file (STL, OBJ, FBX, PLY, glTF, etc.)'
    )
    parser.add_argument('output', help='Output .glb file')
    parser.add_argument(
        '--max-tris',
        type=int,
        default=3_000,
        help='Target triangle count (default: 3000)',
    )
    parser.add_argument(
        '--texture', help='Optional texture image to assign as base color'
    )
    args = parser.parse_args()

    blender = find_blender()

    # Write the Blender-side script to a temp file
    blender_script = _BLENDER_SCRIPT
    with tempfile.NamedTemporaryFile(mode='w', suffix='.py', delete=False) as f:
        f.write(blender_script)
        script_path = f.name

    try:
        blender_args = [
            os.path.abspath(args.input),
            os.path.abspath(args.output),
            '--max-tris',
            str(args.max_tris),
        ]
        if args.texture:
            blender_args.extend(['--texture', os.path.abspath(args.texture)])

        if blender == 'flatpak':
            cmd = [
                'flatpak',
                'run',
                'org.blender.Blender',
                '--background',
                '--python',
                script_path,
                '--',
                *blender_args,
            ]
        else:
            cmd = [
                blender,
                '--background',
                '--python',
                script_path,
                '--',
                *blender_args,
            ]

        result = subprocess.run(cmd)
        sys.exit(result.returncode)
    finally:
        os.unlink(script_path)


_BLENDER_SCRIPT = r'''
"""Blender-side model preprocessing (invoked headlessly)."""

import argparse
import os
import sys

import bpy
import mathutils

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
parser = argparse.ArgumentParser()
parser.add_argument("input")
parser.add_argument("output")
parser.add_argument("--max-tris", type=int, default=3_000)
parser.add_argument("--texture")
args = parser.parse_args(argv)

# Clear default scene
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete()

# Import model based on extension
ext = args.input.rsplit(".", 1)[-1].lower()
importers = {
    "stl": lambda: bpy.ops.import_mesh.stl(filepath=args.input),
    "obj": lambda: bpy.ops.wm.obj_import(filepath=args.input),
    "fbx": lambda: bpy.ops.import_scene.fbx(filepath=args.input),
    "ply": lambda: bpy.ops.import_mesh.ply(filepath=args.input),
    "glb": lambda: bpy.ops.import_scene.gltf(filepath=args.input),
    "gltf": lambda: bpy.ops.import_scene.gltf(filepath=args.input),
}

if ext not in importers:
    print(f"ERROR: unsupported format '.{ext}'", file=sys.stderr)
    sys.exit(1)

importers[ext]()

mesh_objs = [o for o in bpy.context.scene.objects if o.type == "MESH"]
if not mesh_objs:
    print("ERROR: no mesh objects found after import", file=sys.stderr)
    sys.exit(1)

# Join all meshes into one
bpy.ops.object.select_all(action="DESELECT")
for obj in mesh_objs:
    obj.select_set(True)
bpy.context.view_layer.objects.active = mesh_objs[0]
if len(mesh_objs) > 1:
    bpy.ops.object.join()

obj = bpy.context.active_object
bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)

# Center origin
bpy.ops.object.origin_set(type="ORIGIN_GEOMETRY", center="BOUNDS")
obj.location = (0, 0, 0)

# Normalize scale to unit bounding box
bbox = [obj.matrix_world @ mathutils.Vector(corner) for corner in obj.bound_box]
dims = [max(v[i] for v in bbox) - min(v[i] for v in bbox) for i in range(3)]
max_dim = max(dims)
if max_dim > 0:
    sf = 1.0 / max_dim
    obj.scale = (sf, sf, sf)
    bpy.ops.object.transform_apply(scale=True)

# Triangulate
bpy.ops.object.mode_set(mode="EDIT")
bpy.ops.mesh.select_all(action="SELECT")
bpy.ops.mesh.quads_convert_to_tris(quad_method="BEAUTY", ngon_method="BEAUTY")
bpy.ops.object.mode_set(mode="OBJECT")

tri_count = len(obj.data.polygons)
print(f"Triangles after triangulation: {tri_count}")

# Decimate if needed
if tri_count > args.max_tris:
    ratio = args.max_tris / tri_count
    print(f"Decimating: {tri_count} -> ~{args.max_tris} (ratio={ratio:.4f})")
    mod = obj.modifiers.new(name="Decimate", type="DECIMATE")
    mod.ratio = ratio
    bpy.ops.object.modifier_apply(modifier=mod.name)
    print(f"Triangles after decimation: {len(obj.data.polygons)}")

# Recalculate normals
bpy.ops.object.mode_set(mode="EDIT")
bpy.ops.mesh.select_all(action="SELECT")
bpy.ops.mesh.normals_make_consistent(inside=False)
bpy.ops.object.mode_set(mode="OBJECT")

# Smart UV project if no UVs
if not obj.data.uv_layers:
    print("No UV layers — running Smart UV Project")
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=66, island_margin=0.02)
    bpy.ops.object.mode_set(mode="OBJECT")

# Assign texture if provided
if args.texture:
    mat = bpy.data.materials.new(name="Material")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    tex_node = mat.node_tree.nodes.new("ShaderNodeTexImage")
    tex_node.image = bpy.data.images.load(os.path.abspath(args.texture))
    mat.node_tree.links.new(tex_node.outputs["Color"], bsdf.inputs["Base Color"])
    obj.data.materials.clear()
    obj.data.materials.append(mat)

bpy.ops.object.shade_smooth()

# Export
os.makedirs(os.path.dirname(os.path.abspath(args.output)), exist_ok=True)
bpy.ops.export_scene.gltf(
    filepath=args.output,
    export_format="GLB",
    export_apply=True,
    export_normals=True,
    export_texcoords=True,
    export_materials="EXPORT" if args.texture else "NONE",
)

final_tris = len(obj.data.polygons)
print(f"Exported {args.output}: {final_tris} triangles, {len(obj.data.vertices)} vertices")
'''

if __name__ == '__main__':
    main()
