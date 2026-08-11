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
Shared utilities for dice Blender export scripts.

Provides: save_png, digit painting, Sobel normal map generation,
material setup, normalization, and glTF export boilerplate.
"""

import math
import os
import struct
import sys
import zlib

# --- PNG export (no dependencies) ---


def save_png(path: str, px_buf: list[float], img_size: int) -> None:
    """Save a pixel buffer as PNG (no dependencies)."""
    raw = bytearray(max(0, min(255, int(v * 255))) for v in px_buf)
    filtered = bytearray()
    stride = img_size * 4
    for row in range(img_size):
        filtered.append(0)
        filtered.extend(raw[row * stride : (row + 1) * stride])
    idat = zlib.compress(bytes(filtered), 9)

    def png_chunk(tag: bytes, data: bytes) -> bytes:
        body = tag + data
        return (
            struct.pack('>I', len(data))
            + body
            + struct.pack('>I', zlib.crc32(body) & 0xFFFFFFFF)
        )

    ihdr = struct.pack('>IIBBBBB', img_size, img_size, 8, 6, 0, 0, 0)
    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n')
        f.write(png_chunk(b'IHDR', ihdr))
        f.write(png_chunk(b'IDAT', idat))
        f.write(png_chunk(b'IEND', b''))


# --- Font rendering ---
# Uses BraiinsSans-Bold via PIL for anti-aliased text on dice faces.
# PIL is imported lazily so _common.py can still be loaded inside Blender
# (which lacks PIL) — Blender only needs the non-texture helpers.

# Path to font relative to this file (tools/ → ../../assets/fonts/)
_TOOLS_DIR = os.path.dirname(os.path.abspath(__file__))
_FONT_DIR = os.path.normpath(
    os.path.join(_TOOLS_DIR, '..', '..', '..', 'assets', 'fonts')
)
_FONT_PATH = os.path.join(_FONT_DIR, 'BraiinsSans-Bold.otf')


def _font_size(img_size: int, cols: int) -> int:
    """Compute font point size so numbers fill ~40% of cell height."""
    cell_h = img_size // max(cols, 1)
    return max(12, cell_h * 2 // 5)


def paint_number(
    buf: list[float],
    num: int,
    cx: int,
    cy: int,
    img_size: int,
    text_color: tuple[float, ...],
    cols: int = 3,
) -> None:
    """Paint a number centered at (cx, cy) using BraiinsSans-Bold, anti-aliased."""
    from PIL import Image, ImageDraw, ImageFont  # noqa: PLC0415

    label = str(num)
    size = _font_size(img_size, cols)
    font = ImageFont.truetype(_FONT_PATH, size)

    # Render to a small grayscale image, then blit alpha-blended into buf
    bbox = font.getbbox(label)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    pad = 4
    glyph_img = Image.new('L', (tw + pad * 2, th + pad * 2), 0)
    draw = ImageDraw.Draw(glyph_img)
    draw.text((pad - bbox[0], pad - bbox[1]), label, fill=255, font=font)

    gw, gh = glyph_img.size
    ox = cx - gw // 2
    oy = cy - gh // 2
    pixels = glyph_img.load()

    for py in range(gh):
        iy = oy + py
        if iy < 0 or iy >= img_size:
            continue
        for px in range(gw):
            ix = ox + px
            if ix < 0 or ix >= img_size:
                continue
            alpha = pixels[px, py] / 255.0
            if alpha < 0.004:
                continue
            i = (iy * img_size + ix) * 4
            buf[i] += (text_color[0] - buf[i]) * alpha
            buf[i + 1] += (text_color[1] - buf[i + 1]) * alpha
            buf[i + 2] += (text_color[2] - buf[i + 2]) * alpha


# --- Texture generation helpers ---


def fill_background(buf: list[float], img_size: int, color: tuple[float, ...]) -> None:
    """Fill buffer with a solid background color."""
    for i in range(img_size * img_size):
        off = i * 4
        buf[off] = color[0]
        buf[off + 1] = color[1]
        buf[off + 2] = color[2]
        buf[off + 3] = color[3] if len(color) > 3 else 1.0


def fill_neutral_normal(buf: list[float], img_size: int) -> None:
    """Fill buffer with neutral normal (0.5, 0.5, 1.0)."""
    for i in range(img_size * img_size):
        off = i * 4
        buf[off] = 0.5
        buf[off + 1] = 0.5
        buf[off + 2] = 1.0
        buf[off + 3] = 1.0


def paint_grid_lines(
    buf: list[float],
    img_size: int,
    cols: int,
    rows: int,
    color: tuple[float, ...],
) -> None:
    """Draw grid lines at cell boundaries."""
    cell_w = img_size // cols
    cell_h = img_size // rows
    for c in range(cols + 1):
        gx = min(c * cell_w, img_size - 1)
        for gy in range(img_size):
            i = (gy * img_size + gx) * 4
            buf[i] = color[0]
            buf[i + 1] = color[1]
            buf[i + 2] = color[2]
    for r in range(rows + 1):
        gy = min(r * cell_h, img_size - 1)
        for gx in range(img_size):
            i = (gy * img_size + gx) * 4
            buf[i] = color[0]
            buf[i + 1] = color[1]
            buf[i + 2] = color[2]


def generate_sobel_normal_map(
    albedo: list[float], img_size: int, indent_depth: float = 0.6
) -> list[float]:
    """Generate a normal map from albedo using Sobel edge detection.

    Creates engraved/beveled edges around text and features in the albedo.
    """
    buf = [0.0] * (img_size * img_size * 4)
    fill_neutral_normal(buf, img_size)

    step = max(1, img_size // 512)
    for y in range(step, img_size - step):
        for x in range(step, img_size - step):
            idx = (y * img_size + x) * 4

            lum_l = (
                albedo[(y * img_size + x - step) * 4] * 0.299
                + albedo[(y * img_size + x - step) * 4 + 1] * 0.587
                + albedo[(y * img_size + x - step) * 4 + 2] * 0.114
            )
            lum_r = (
                albedo[(y * img_size + x + step) * 4] * 0.299
                + albedo[(y * img_size + x + step) * 4 + 1] * 0.587
                + albedo[(y * img_size + x + step) * 4 + 2] * 0.114
            )
            lum_u = (
                albedo[((y - step) * img_size + x) * 4] * 0.299
                + albedo[((y - step) * img_size + x) * 4 + 1] * 0.587
                + albedo[((y - step) * img_size + x) * 4 + 2] * 0.114
            )
            lum_d = (
                albedo[((y + step) * img_size + x) * 4] * 0.299
                + albedo[((y + step) * img_size + x) * 4 + 1] * 0.587
                + albedo[((y + step) * img_size + x) * 4 + 2] * 0.114
            )

            dx = (lum_l - lum_r) * indent_depth
            dy = (lum_u - lum_d) * indent_depth

            if abs(dx) > 0.001 or abs(dy) > 0.001:
                nz = math.sqrt(max(0.0, 1.0 - dx * dx - dy * dy))
                buf[idx] = dx * 0.5 + 0.5
                buf[idx + 1] = dy * 0.5 + 0.5
                buf[idx + 2] = nz * 0.5 + 0.5

    return buf


# --- Reset-texture entry point ---


def handle_reset_texture(
    script_name: str,
    generate_albedo_fn,
    generate_normal_fn,
) -> bool:
    """Handle --reset-texture mode. Returns True if handled (caller should exit)."""
    if '--reset-texture' not in sys.argv:
        return False
    idx = sys.argv.index('--reset-texture')
    if idx + 1 >= len(sys.argv):
        print(
            f'Usage: python tools/{script_name} --reset-texture assets/{script_name.replace(".py", "")}',
            file=sys.stderr,
        )
        sys.exit(1)
    base_path = os.path.abspath(sys.argv[idx + 1])
    img_size = generate_albedo_fn.__code__.co_varnames  # hacky, use actual call
    albedo = generate_albedo_fn()
    normal = generate_normal_fn()
    # Infer img_size from buffer length
    pixel_count = len(albedo) // 4
    img_size = int(math.sqrt(pixel_count))
    save_png(base_path + '.albedo.png', albedo, img_size)
    save_png(base_path + '.normal.png', normal, img_size)
    print(f'Reset textures: {base_path}.albedo.png, {base_path}.normal.png')
    sys.exit(0)


# --- Blender helpers (only usable when running inside Blender) ---


def parse_blender_args() -> str:
    """Parse Blender CLI args, return output path."""
    argv = sys.argv[sys.argv.index('--') + 1 :] if '--' in sys.argv else []
    if not argv:
        print('Usage: ... -- output.glb', file=sys.stderr)
        sys.exit(1)
    return os.path.abspath(argv[0])


def ensure_textures(
    output_path: str, generate_albedo_fn, generate_normal_fn, img_size: int
):
    """Generate textures if they don't exist. Returns (albedo_path, normal_path)."""
    base_name = os.path.splitext(output_path)[0]
    albedo_path = base_name + '.albedo.png'
    normal_path = base_name + '.normal.png'
    if not os.path.exists(albedo_path):
        save_png(albedo_path, generate_albedo_fn(), img_size)
        print(f'Generated default albedo: {albedo_path}')
    if not os.path.exists(normal_path):
        save_png(normal_path, generate_normal_fn(), img_size)
        print(f'Generated default normal map: {normal_path}')
    return albedo_path, normal_path


def setup_material(obj, albedo_path: str, normal_path: str, name: str = 'DieMaterial'):
    """Create principled BSDF material with albedo + normal map textures."""
    import bpy  # noqa: PLC0415  # type: ignore[import-not-found]

    mat = bpy.data.materials.new(name=name)
    nodes = mat.node_tree.nodes
    links = mat.node_tree.links
    for node in nodes:
        nodes.remove(node)

    output_node = nodes.new('ShaderNodeOutputMaterial')
    bsdf = nodes.new('ShaderNodeBsdfPrincipled')
    links.new(bsdf.outputs['BSDF'], output_node.inputs['Surface'])
    bsdf.inputs['Roughness'].default_value = 0.3

    albedo_img = bpy.data.images.load(albedo_path)
    albedo_node = nodes.new('ShaderNodeTexImage')
    albedo_node.image = albedo_img
    links.new(albedo_node.outputs['Color'], bsdf.inputs['Base Color'])

    if os.path.exists(normal_path):
        nmap_img = bpy.data.images.load(normal_path)
        nmap_img.colorspace_settings.name = 'Non-Color'
        nmap_node = nodes.new('ShaderNodeTexImage')
        nmap_node.image = nmap_img
        normal_map_node = nodes.new('ShaderNodeNormalMap')
        normal_map_node.space = 'TANGENT'
        links.new(nmap_node.outputs['Color'], normal_map_node.inputs['Color'])
        links.new(normal_map_node.outputs['Normal'], bsdf.inputs['Normal'])

    obj.data.materials.clear()
    obj.data.materials.append(mat)


def normalize_scale(obj):
    """Normalize object to fit in a unit bounding box."""
    import bpy  # noqa: PLC0415  # type: ignore[import-not-found]
    import mathutils  # noqa: PLC0415  # type: ignore[import-not-found]

    bbox = [obj.matrix_world @ mathutils.Vector(c) for c in obj.bound_box]
    dims = [max(v[i] for v in bbox) - min(v[i] for v in bbox) for i in range(3)]
    max_dim = max(dims)
    if max_dim > 0:
        sf = 1.0 / max_dim
        obj.scale = (sf, sf, sf)
        bpy.ops.object.transform_apply(scale=True)


def finalize_and_export(
    obj, output_path: str, face_normals_flat: list[float], face_count: int
):
    """Mark seams, triangulate, normalize, embed extras, export GLB."""
    import bpy  # noqa: PLC0415  # type: ignore[import-not-found]

    # Mark UV seams for glTF vertex split
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_all(action='SELECT')
    bpy.ops.uv.seams_from_islands()

    # Triangulate
    bpy.ops.mesh.select_all(action='SELECT')
    bpy.ops.mesh.quads_convert_to_tris(quad_method='BEAUTY', ngon_method='BEAUTY')
    bpy.ops.object.mode_set(mode='OBJECT')

    normalize_scale(obj)

    # Embed face normals in glTF extras
    obj['face_normals'] = face_normals_flat
    obj['face_count'] = face_count

    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    bpy.ops.export_scene.gltf(
        filepath=output_path,
        export_format='GLB',
        export_apply=True,
        export_extras=True,
        export_normals=True,
        export_texcoords=True,
        export_tangents=True,
        export_materials='EXPORT',
        export_image_format='AUTO',
    )

    print(
        f'Exported {output_path}: {len(obj.data.polygons)} tris, {len(obj.data.vertices)} verts'
    )
