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

"""Export Suzanne (Blender monkey head) as a .glb with a baked procedural texture.

Usage:
    flatpak run org.blender.Blender --background --python suzanne.py -- assets/suzanne.glb
"""

import os
import sys

import bpy  # type: ignore[import-not-found]  # Blender-embedded module
import mathutils  # type: ignore[import-not-found]  # Blender-embedded module

argv = sys.argv[sys.argv.index('--') + 1 :] if '--' in sys.argv else []
if not argv:
    print('Usage: ... -- output.glb', file=sys.stderr)
    sys.exit(1)

output_path = os.path.abspath(argv[0])

# Clear default scene
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete()

# Add Suzanne
bpy.ops.mesh.primitive_monkey_add(size=1.0, location=(0, 0, 0))
obj = bpy.context.active_object

# Subdivision for smoother normals
mod = obj.modifiers.new(name='Subsurf', type='SUBSURF')
mod.levels = 2
mod.render_levels = 2
bpy.ops.object.modifier_apply(modifier=mod.name)

# Triangulate
bpy.ops.object.mode_set(mode='EDIT')
bpy.ops.mesh.select_all(action='SELECT')
bpy.ops.mesh.quads_convert_to_tris(quad_method='BEAUTY', ngon_method='BEAUTY')
bpy.ops.object.mode_set(mode='OBJECT')

tri_count = len(obj.data.polygons)
print(f'Triangle count: {tri_count}')

# Decimate if over 3000 triangles
if tri_count > 3000:
    ratio = 3000 / tri_count
    mod = obj.modifiers.new(name='Decimate', type='DECIMATE')
    mod.ratio = ratio
    bpy.ops.object.modifier_apply(modifier=mod.name)
    tri_count = len(obj.data.polygons)
    print(f'After decimation: {tri_count}')

# Recalculate normals
bpy.ops.object.mode_set(mode='EDIT')
bpy.ops.mesh.select_all(action='SELECT')
bpy.ops.mesh.normals_make_consistent(inside=False)
bpy.ops.object.mode_set(mode='OBJECT')

# Smart UV project
bpy.ops.object.mode_set(mode='EDIT')
bpy.ops.mesh.select_all(action='SELECT')
bpy.ops.uv.smart_project(angle_limit=66, island_margin=0.02)
bpy.ops.object.mode_set(mode='OBJECT')

# Create material with a procedural noise texture baked to image
mat = bpy.data.materials.new(name='SuzanneMaterial')
nodes = mat.node_tree.nodes
links = mat.node_tree.links

# Clear default nodes
for node in nodes:
    nodes.remove(node)

# Create nodes for baking
output_node = nodes.new('ShaderNodeOutputMaterial')
bsdf = nodes.new('ShaderNodeBsdfPrincipled')
links.new(bsdf.outputs['BSDF'], output_node.inputs['Surface'])

# Create a procedural noise texture for visual interest
tex_coord = nodes.new('ShaderNodeTexCoord')
noise = nodes.new('ShaderNodeTexNoise')
noise.inputs['Scale'].default_value = 4.0
noise.inputs['Detail'].default_value = 6.0
noise.inputs['Roughness'].default_value = 0.6
links.new(tex_coord.outputs['Object'], noise.inputs['Vector'])

# Color ramp for a nice warm clay look
ramp = nodes.new('ShaderNodeValToRGB')
ramp.color_ramp.elements[0].position = 0.3
ramp.color_ramp.elements[0].color = (0.45, 0.28, 0.18, 1.0)  # dark clay
ramp.color_ramp.elements[1].position = 0.7
ramp.color_ramp.elements[1].color = (0.75, 0.52, 0.35, 1.0)  # light clay
links.new(noise.outputs['Fac'], ramp.inputs['Fac'])

# Image texture node for bake target
bake_img = bpy.data.images.new('SuzanneBake', width=512, height=512)
img_node = nodes.new('ShaderNodeTexImage')
img_node.image = bake_img
img_node.select = True
nodes.active = img_node

# Connect noise to BSDF for baking
links.new(ramp.outputs['Color'], bsdf.inputs['Base Color'])
bsdf.inputs['Roughness'].default_value = 0.8

obj.data.materials.clear()
obj.data.materials.append(mat)

# Bake the procedural texture to the image
bpy.context.scene.render.engine = 'CYCLES'
bpy.context.scene.cycles.device = 'CPU'
bpy.context.scene.cycles.samples = 4  # low samples for speed
bpy.context.scene.cycles.bake_type = 'DIFFUSE'

# Select object for baking
bpy.ops.object.select_all(action='DESELECT')
obj.select_set(True)
bpy.context.view_layer.objects.active = obj

bpy.ops.object.bake(type='DIFFUSE', pass_filter={'COLOR'})

# Now rewire material to use the baked image instead of procedural
for node in [noise, ramp, tex_coord]:
    nodes.remove(node)
links.new(img_node.outputs['Color'], bsdf.inputs['Base Color'])

# Shade smooth
bpy.ops.object.shade_smooth()

# Normalize scale to unit bounding box
bbox = [obj.matrix_world @ mathutils.Vector(corner) for corner in obj.bound_box]
dims = [max(v[i] for v in bbox) - min(v[i] for v in bbox) for i in range(3)]
max_dim = max(dims)
if max_dim > 0:
    scale_factor = 1.0 / max_dim
    obj.scale = (scale_factor, scale_factor, scale_factor)
    bpy.ops.object.transform_apply(scale=True)

# Export as .glb
os.makedirs(os.path.dirname(output_path), exist_ok=True)
bpy.ops.export_scene.gltf(
    filepath=output_path,
    export_format='GLB',
    export_apply=True,
    export_normals=True,
    export_texcoords=True,
    export_materials='EXPORT',
    export_image_format='AUTO',
)

final_tris = len(obj.data.polygons)
print(
    f'Exported {output_path}: {final_tris} triangles, {len(obj.data.vertices)} vertices'
)
