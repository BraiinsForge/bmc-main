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

"""Shared texture catalog and paths for ISS widget texture tooling.

TEXTURE MAPPING CONTRACT — the 3D globe shader samples textures using
equirectangular UV coordinates. The convention MUST be:

    u = 0.0 → lon = -180°  (left edge)
    u = 0.5 → lon =    0°  (prime meridian, center)
    u = 1.0 → lon = +180°  (right edge)

    v = 0.0 → lat =  +90°  (north pole, top edge)
    v = 0.5 → lat =    0°  (equator, center)
    v = 1.0 → lat =  -90°  (south pole, bottom edge)

This matches the Cartopy PlateCarrée projection. If you change the projection
or crop the output, the globe shader will show misplaced geography.
See: bmc-render/src/gpu/sphere.rs (fragment shader UV sampling).
"""

import io
from pathlib import Path
from typing import TypedDict

from PIL import Image


class TextureEntry(TypedDict):
    id: str
    name: str
    url: str
    license: str
    native_res: str
    note: str
    projection: str
    ext: str


TOOLS_DIR: Path = Path(__file__).resolve().parent
ISS_ROOT: Path = TOOLS_DIR.parent
TEXTURE_DIR: Path = ISS_ROOT / 'textures'
SHIPPED_TEXTURE: Path = ISS_ROOT / 'src' / 'render' / 'texture.jpg'

BASE_TARGET_H = 512
# Unscaled baseline: label point sizes are calibrated against this width.
BASE_TARGET_W = BASE_TARGET_H * 2

# Single knob for texture size. 1.0 = 1024x512 target, 2.0 = 2048x1024 target.
#
# 1.3 trades sharpness for memory: the globe magnifies the map at disc centre,
# so the baked-in country labels soften.
# Decoded cost grows with the pixel count and then doubles,
# because registering an encoded bitmap keeps a CPU sampling copy
# beside the GL texture (bmc-render/src/gpu/bitmap.rs).
QUALITY_SCALE = 1.3

# Width is derived, never scaled independently:
# the contract above requires exactly 2:1, and cartopy letterboxes a 2:1 map
# inside an off-ratio canvas, which shifts u=0 off -180°.
TARGET_H = int(BASE_TARGET_H * QUALITY_SCALE)
TARGET_W = TARGET_H * 2

# Rasterizing at TARGET directly looks worse: halo and stroke widths
# are sized in points, so they do not shrink with the image
# and cover twice the proportion of a glyph.
RENDER_W = TARGET_W * 2
RENDER_H = TARGET_H * 2

SUBSAMPLING_NAMES: dict[int, str] = {0: '4:4:4', 1: '4:2:2', 2: '4:2:0'}

# The encoder stays baseline: progressive saves another ~4%
# but decodes by buffering the whole frame, the memory this size exists to save.
JPEG_QUALITY = 70
JPEG_SUBSAMPLING = 2

TEXTURE_SUFFIXES = frozenset({'.jpg', '.jpeg'})


def downsample(image: Image.Image, target: tuple[int, int]) -> Image.Image:
    """Resize to `target` with the filter the shipped texture is built with."""
    return image.convert('RGB').resize(target, Image.LANCZOS)


def encode_texture(
    image: Image.Image,
    *,
    quality: int = JPEG_QUALITY,
    subsampling: int = JPEG_SUBSAMPLING,
) -> bytes:
    buffer = io.BytesIO()
    image.save(buffer, 'JPEG', quality=quality, subsampling=subsampling, optimize=True)
    return buffer.getvalue()


def write_texture(image: Image.Image, output: Path) -> None:
    if output.suffix.lower() not in TEXTURE_SUFFIXES:
        raise ValueError(
            f'texture output must be one of {sorted(TEXTURE_SUFFIXES)}, got {output.name}'
        )
    # Staged and swapped, never written in place:
    # a full disk or a Ctrl-C would otherwise truncate the committed texture.
    staged = output.with_name(f'{output.name}.tmp')
    try:
        staged.write_bytes(encode_texture(image))
    except OSError:
        staged.unlink(missing_ok=True)
        raise
    staged.replace(output)


def print_texture_contract(resolution: tuple[int, int]) -> None:
    """Print the texture mapping contract with ANSI formatting."""
    w, h = resolution
    # ANSI: bold, reset, dim, black-on-yellow background, reset-bg
    B, R, D = '\033[1m', '\033[0m', '\033[2m'
    BG, FG = '\033[43;30m', '\033[49;39m'
    print(f"""
  {BG}{B} TEXTURE MAPPING CONTRACT {R}{FG} {D}(must match globe shader: bmc-render/src/gpu/sphere.rs){R}

  Resolution: {B}{w}×{h}{R}  Projection: {B}PlateCarrée (equirectangular){R}

  u = 0.0 → lon = -180°  {D}(left){R}    v = 0.0 → lat = +90°  {D}(top, north){R}
  u = 0.5 → lon =    0°  {D}(center){R}  v = 0.5 → lat =   0°  {D}(center, equator){R}
  u = 1.0 → lon = +180°  {D}(right){R}   v = 1.0 → lat = -90°  {D}(bottom, south){R}
""")


# Mapbox token from the ISS widget source
MAPBOX_TOKEN = (
    'pk.eyJ1IjoiYnJhaWluc2ZvcmdlIiwiYSI6ImNta3lmZmU0aTA1dzkzaHM2NGQ5Nmhhc2sifQ'
    '.HTDxIOIJ7g_9NvMLlVOuVQ'
)

TEXTURES: list[TextureEntry] = [
    {
        'id': 'natural-earth-dark',
        'name': 'Natural Earth Dark (Mapbox style)',
        'url': '',
        'license': 'Public domain (Natural Earth)',
        'native_res': '2048\u00d71024',
        'note': 'Custom dark render. Neutral gray palette similar to Mapbox dark-v11.',
        'projection': 'equirectangular',
        'ext': 'jpg',
    },
    {
        'id': 'natural-earth-gmaps',
        'name': 'Natural Earth Dark (Google Maps style)',
        'url': '',
        'license': 'Public domain (Natural Earth)',
        'native_res': '2048\u00d71024',
        'note': 'Custom dark render. Blue-tinted palette similar to Google Maps dark mode.',
        'projection': 'equirectangular',
        'ext': 'jpg',
    },
    {
        'id': 'black-marble-nasa',
        'name': 'NASA Black Marble 2016',
        'url': 'https://neo.gsfc.nasa.gov/archive/blackmarble/2016/global/BlackMarble_2016_01deg.jpg',
        'license': 'Public domain (NASA)',
        'native_res': '3600\u00d71800',
        'note': 'Genuine night lights composite. Best dark theme candidate.',
        'projection': 'equirectangular',
        'ext': 'jpg',
    },
    {
        'id': 'earth-night-2012',
        'name': 'NASA Earth at Night 2012',
        'url': 'https://eoimages.gsfc.nasa.gov/images/imagerecords/79000/79765/dnb_land_ocean_ice.2012.3600x1800.jpg',
        'license': 'Public domain (NASA)',
        'native_res': '3600\u00d71800',
        'note': 'Older night lights with ocean/ice detail.',
        'projection': 'equirectangular',
        'ext': 'jpg',
    },
    {
        'id': 'blue-marble-topo',
        'name': 'NASA Blue Marble (Topo + Shallow Water)',
        'url': 'https://eoimages.gsfc.nasa.gov/images/imagerecords/57000/57752/land_shallow_topo_2048.jpg',
        'license': 'Public domain (NASA)',
        'native_res': '2048\u00d71024',
        'note': 'Classic daytime composite. Reference for continent outlines.',
        'projection': 'equirectangular',
        'ext': 'jpg',
    },
    {
        'id': 'blue-marble-clouds',
        'name': 'NASA Blue Marble (Land, Ocean, Ice, Clouds)',
        'url': 'https://eoimages.gsfc.nasa.gov/images/imagerecords/57000/57735/land_ocean_ice_cloud_2048.jpg',
        'license': 'Public domain (NASA)',
        'native_res': '2048\u00d71024',
        'note': 'Photorealistic with clouds.',
        'projection': 'equirectangular',
        'ext': 'jpg',
    },
    {
        'id': 'sss-day',
        'name': 'Solar System Scope Day',
        'url': 'https://www.solarsystemscope.com/textures/download/2k_earth_daymap.jpg',
        'license': 'CC-BY 4.0 (Solar System Scope)',
        'native_res': '2048\u00d71024',
        'note': 'Clean daytime composite for reference.',
        'projection': 'equirectangular',
        'ext': 'jpg',
    },
    {
        'id': 'mapbox-dark',
        'name': 'Mapbox dark-v11',
        'url': (
            'https://api.mapbox.com/styles/v1/mapbox/dark-v11/static/0,0,1'
            f'/1024x1024@2x?logo=false&attribution=false&access_token={MAPBOX_TOKEN}'
        ),
        'license': 'Mapbox ToS (current API)',
        'native_res': '2048\u00d72048',
        'note': 'Reference: current Mapbox tile. Web Mercator \u2014 distorted on globe.',
        'projection': 'mercator',
        # Mapbox returns PNG for this endpoint, not JPEG.
        'ext': 'png',
    },
]
