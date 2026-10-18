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

"""Render dark-themed equirectangular earth textures from Natural Earth data.

Usage:
    uv run texture_render.py [--output DIR] [--keep-lossless]

Generates equirectangular JPEG textures using Natural Earth vector data
(coastlines, borders, country labels) in multiple dark themes. Rasterizes at
RENDER and downsamples to TARGET, so the shipped texture is supersampled.
Resolution configured in _textures.py. See _textures.__doc__ for the UV contract.

Output is not reproducible across machines.
Cartopy fetches Natural Earth 50m/110m shapefiles from unversioned URLs,
so a rerun can pick up vector data that differs from the committed texture.
Only the encode step is pinned, by uv.lock.
"""

import io
from dataclasses import dataclass
from pathlib import Path
from typing import TypedDict, cast

from _textures import (
    BASE_TARGET_W,
    RENDER_H,
    RENDER_W,
    TARGET_H,
    TARGET_W,
    TEXTURE_DIR,
    downsample,
    print_texture_contract,
    write_texture,
)

import cartopy.crs as ccrs
import cartopy.feature as cfeature
import matplotlib
import matplotlib.patheffects as pe
import matplotlib.pyplot as plt
import tyro
from cartopy.io import shapereader
from cartopy.mpl.geoaxes import GeoAxes
from PIL import Image

matplotlib.use('Agg')


class Theme(TypedDict):
    id: str
    name: str
    ocean: str
    land: str
    border: str
    coast: str
    lake: str
    label: str
    label_halo: str


THEMES: list[Theme] = [
    {
        'id': 'natural-earth-dark',
        'name': 'Natural Earth Dark (Mapbox style)',
        'ocean': '#191a1a',
        'land': '#2b2b2b',
        'border': '#555555',
        'coast': '#555555',
        'lake': '#191a1a',
        'label': '#999999',
        'label_halo': '#191a1a',
    },
    {
        'id': 'natural-earth-gmaps',
        'name': 'Natural Earth Dark (Google Maps style)',
        'ocean': '#17263c',
        'land': '#242f3e',
        'border': '#4a5568',
        'coast': '#4a5568',
        'lake': '#17263c',
        'label': '#8e9bae',
        'label_halo': '#1a2332',
    },
]

FONT_SIZE = 6.5
MIN_LABEL_AREA = 25.0

# Countries to skip labeling (too small / overlaps with neighbors).
SKIP_LABELS: set[str] = {
    'Somaliland',  # overlaps Somalia / Ethiopia
    'N. Cyprus',  # overlaps Turkey
    'Kosovo',  # overlaps Serbia
}

# Manual centroid overrides (name → (lon, lat)) to fix edge-clips and overlaps.
LABEL_OVERRIDES: dict[str, tuple[float, float]] = {
    'New Zealand': (170.0, -42.0),  # shift west — default centroid clips at ±180° edge
    'Nigeria': (8.0, 8.5),  # nudge south to avoid Niger overlap
    'Dem. Rep. Congo': (24.0, -3.5),  # nudge to avoid overlap with neighbors
}


def render_texture(
    theme: Theme,
    width: int,
    height: int,
    target: tuple[int, int],
    output: Path,
    *,
    keep_lossless: bool,
) -> None:
    """Render one equirectangular earth texture, downsampled to `target`."""
    dpi = 100
    fig_w = width / dpi
    fig_h = height / dpi
    halo = [pe.withStroke(linewidth=2.5, foreground=theme['label_halo'])]

    projection = ccrs.PlateCarree()
    # The ignores in this file are library gaps: matplotlib's stubs leave `**kwargs`
    # unannotated, and cartopy ships neither `py.typed` nor stubs.
    fig = plt.figure(figsize=(fig_w, fig_h), dpi=dpi, facecolor=theme['ocean'])  # pyright: ignore[reportUnknownMemberType]
    ax = cast(
        GeoAxes,
        fig.add_axes((0, 0, 1, 1), projection=projection, facecolor=theme['ocean']),  # pyright: ignore[reportUnknownMemberType]
    )
    ax.set_global()
    ax.spines['geo'].set_visible(False)

    _ = ax.add_feature(  # pyright: ignore[reportUnknownMemberType]
        cfeature.NaturalEarthFeature('physical', 'land', '50m'),
        facecolor=theme['land'],
        edgecolor='none',
        zorder=1,
    )
    _ = ax.add_feature(  # pyright: ignore[reportUnknownMemberType]
        cfeature.NaturalEarthFeature('physical', 'lakes', '50m'),
        facecolor=theme['lake'],
        edgecolor='none',
        zorder=2,
    )
    _ = ax.add_feature(  # pyright: ignore[reportUnknownMemberType]
        cfeature.NaturalEarthFeature('physical', 'coastline', '50m'),
        facecolor='none',
        edgecolor=theme['coast'],
        linewidth=0.4,
        zorder=3,
    )
    _ = ax.add_feature(  # pyright: ignore[reportUnknownMemberType]
        cfeature.NaturalEarthFeature('cultural', 'admin_0_boundary_lines_land', '50m'),
        facecolor='none',
        edgecolor=theme['border'],
        linewidth=0.5,
        zorder=4,
    )

    _add_country_labels(ax, width, theme, halo)

    supersampled = io.BytesIO()
    fig.savefig(supersampled, format='png', dpi=dpi, facecolor=fig.get_facecolor())  # pyright: ignore[reportUnknownMemberType]
    plt.close(fig)

    _ = supersampled.seek(0)
    with Image.open(supersampled) as raster:
        texture = downsample(raster, target)

    if keep_lossless:
        texture.save(output.with_suffix('.png'))
    write_texture(texture, output)

    size_kb = output.stat().st_size / 1024
    print(
        f'  {output.name}: rendered {width}x{height} → {target[0]}x{target[1]}, {size_kb:.0f} KB'
    )


def _add_country_labels(
    ax: GeoAxes, img_width: int, theme: Theme, halo: list[pe.withStroke]
) -> None:
    """Add country name labels centered on each country."""
    scale = img_width / BASE_TARGET_W
    font_size = FONT_SIZE * scale

    shpfile = cast(
        str, shapereader.natural_earth('110m', 'cultural', 'admin_0_countries')
    )
    reader = shapereader.Reader(shpfile)

    for record in reader.records():
        name = str(record.attributes.get('NAME', ''))  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
        geom = record.geometry

        # Skip tiny countries and known overlap / clip offenders
        if geom is None or geom.area < MIN_LABEL_AREA or name in SKIP_LABELS:
            continue

        # Use manual position if the auto centroid clips or overlaps
        if name in LABEL_OVERRIDES:
            lx, ly = LABEL_OVERRIDES[name]
        else:
            centroid = geom.centroid
            lx, ly = centroid.x, centroid.y

        _ = ax.text(  # pyright: ignore[reportUnknownMemberType]
            lx,
            ly,
            name,
            transform=ccrs.PlateCarree(),
            fontsize=font_size,
            fontweight='normal',
            color=theme['label'],
            alpha=0.7,
            ha='center',
            va='center',
            path_effects=halo,
            zorder=10,
        )


@dataclass
class Args:
    output: Path = TEXTURE_DIR  # output directory
    keep_lossless: bool = (
        False  # also write the downsampled texture as PNG, for `just sweep`
    )


def main() -> None:
    # tyro declares `cli`'s `default` sentinel as Any (tyro/_singleton.py `MISSING_NONPROP`).
    args = tyro.cli(Args, description=__doc__)  # pyright: ignore[reportAny]
    args.output.mkdir(exist_ok=True)

    print_texture_contract((TARGET_W, TARGET_H))
    print(f'Rendering Natural Earth dark textures at {RENDER_W}x{RENDER_H}...')
    print('  Fetching Natural Earth data (cached after first run)...')

    for theme in THEMES:
        render_texture(
            theme,
            RENDER_W,
            RENDER_H,
            (TARGET_W, TARGET_H),
            args.output / f'{theme["id"]}.jpg',
            keep_lossless=args.keep_lossless,
        )

    print('\nDone. Preview with: uv run texture_preview.py')


if __name__ == '__main__':
    main()
