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
See: bmc-wasm-runtime/src/gpu/sphere.rs (fragment shader UV sampling).
"""

from pathlib import Path
from typing import TypedDict


class TextureEntry(TypedDict):
    id: str
    name: str
    url: str
    license: str
    native_res: str
    note: str
    projection: str


TOOLS_DIR: Path = Path(__file__).resolve().parent
ISS_ROOT: Path = TOOLS_DIR.parent
TEXTURE_DIR: Path = ISS_ROOT / 'textures'

# Base target resolution for the ISS widget (zoom 1, full-world viewport)
BASE_TARGET_W = 1024
BASE_TARGET_H = 512

# Single knob for texture size. 1.0 = 1024x512 target, 2.0 = 2048x1024 target.
QUALITY_SCALE = 1.3

# Target resolution for the ISS widget (scaled)
TARGET_W = int(BASE_TARGET_W * QUALITY_SCALE)
TARGET_H = int(BASE_TARGET_H * QUALITY_SCALE)

# Render resolution for locally generated textures (2x target for quality)
RENDER_W = TARGET_W * 2
RENDER_H = TARGET_H * 2


def print_texture_contract(resolution: tuple[int, int]) -> None:
    """Print the texture mapping contract with ANSI formatting."""
    w, h = resolution
    # ANSI: bold, reset, dim, black-on-yellow background, reset-bg
    B, R, D = '\033[1m', '\033[0m', '\033[2m'
    BG, FG = '\033[43;30m', '\033[49;39m'
    print(f"""
  {BG}{B} TEXTURE MAPPING CONTRACT {R}{FG} {D}(must match globe shader: src/gpu/sphere.rs){R}

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
    },
    {
        'id': 'natural-earth-gmaps',
        'name': 'Natural Earth Dark (Google Maps style)',
        'url': '',
        'license': 'Public domain (Natural Earth)',
        'native_res': '2048\u00d71024',
        'note': 'Custom dark render. Blue-tinted palette similar to Google Maps dark mode.',
        'projection': 'equirectangular',
    },
    {
        'id': 'black-marble-nasa',
        'name': 'NASA Black Marble 2016',
        'url': 'https://neo.gsfc.nasa.gov/archive/blackmarble/2016/global/BlackMarble_2016_01deg.jpg',
        'license': 'Public domain (NASA)',
        'native_res': '3600\u00d71800',
        'note': 'Genuine night lights composite. Best dark theme candidate.',
        'projection': 'equirectangular',
    },
    {
        'id': 'earth-night-2012',
        'name': 'NASA Earth at Night 2012',
        'url': 'https://eoimages.gsfc.nasa.gov/images/imagerecords/79000/79765/dnb_land_ocean_ice.2012.3600x1800.jpg',
        'license': 'Public domain (NASA)',
        'native_res': '3600\u00d71800',
        'note': 'Older night lights with ocean/ice detail.',
        'projection': 'equirectangular',
    },
    {
        'id': 'blue-marble-topo',
        'name': 'NASA Blue Marble (Topo + Shallow Water)',
        'url': 'https://eoimages.gsfc.nasa.gov/images/imagerecords/57000/57752/land_shallow_topo_2048.jpg',
        'license': 'Public domain (NASA)',
        'native_res': '2048\u00d71024',
        'note': 'Classic daytime composite. Reference for continent outlines.',
        'projection': 'equirectangular',
    },
    {
        'id': 'blue-marble-clouds',
        'name': 'NASA Blue Marble (Land, Ocean, Ice, Clouds)',
        'url': 'https://eoimages.gsfc.nasa.gov/images/imagerecords/57000/57735/land_ocean_ice_cloud_2048.jpg',
        'license': 'Public domain (NASA)',
        'native_res': '2048\u00d71024',
        'note': 'Photorealistic with clouds.',
        'projection': 'equirectangular',
    },
    {
        'id': 'sss-day',
        'name': 'Solar System Scope Day',
        'url': 'https://www.solarsystemscope.com/textures/download/2k_earth_daymap.jpg',
        'license': 'CC-BY 4.0 (Solar System Scope)',
        'native_res': '2048\u00d71024',
        'note': 'Clean daytime composite for reference.',
        'projection': 'equirectangular',
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
    },
]
