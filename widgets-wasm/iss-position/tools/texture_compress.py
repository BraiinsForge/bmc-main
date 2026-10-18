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

"""Choose and apply the JPEG encoding for shipped ISS globe textures.

`sweep` encodes a lossless reference at a range of qualities and reports size
against PSNR, to pick JPEG_QUALITY in _textures.py. It measures encoder loss
only: the reference must already sit at the shipped resolution,
so downsampling loss is deliberately absent from these numbers.

`downsample` brings a catalog texture fetched at native resolution
by texture_download.py down to the shipped size and encoding.
"""

import io
import math
import sys
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import tyro
from _textures import (
    JPEG_SUBSAMPLING,
    SUBSAMPLING_NAMES,
    TARGET_H,
    TARGET_W,
    Subsampling,
    downsample,
    encode_texture,
    write_texture,
)
from PIL import Image
from tyro.conf import Positional

SWEEP_QUALITIES = (60, 65, 70, 75, 80, 85, 90, 95)

# A source misses 2:1 only by carrying padding or a different latitude extent.
# One percent of that is ~390 km of misplaced geography at the equator,
# under half the ISS marker — so this catches a wrong projection,
# not a positional budget.
ASPECT_TOLERANCE = 0.01

LOSSLESS_FORMATS = frozenset({'PNG', 'TIFF', 'BMP'})


def psnr(reference: Image.Image, candidate: Image.Image) -> float:
    a = np.asarray(reference, dtype=np.float64)
    b = np.asarray(candidate, dtype=np.float64)
    mse = float(np.mean((a - b) ** 2))
    if mse == 0.0:
        return float('inf')
    return 10.0 * math.log10(255.0**2 / mse)


def dims(size: tuple[int, int]) -> str:
    return f'{size[0]}x{size[1]}'


def describe(path: Path, size: tuple[int, int], size_bytes: int) -> str:
    return f'{path}  {dims(size)}  {size_bytes / 1024:.1f} KB'


def resident_mib(size: tuple[int, int]) -> float:
    """Both decoded copies the host keeps for a texture of `size`, in MiB."""
    return 2 * size[0] * size[1] * 4 / 1024 / 1024


def sweep(path: Path, subsampling: Subsampling) -> None:
    with Image.open(path) as opened:
        source_format = opened.format
        reference = opened.convert('RGB')
    # Checked on the decoded format, not the suffix:
    # a JPEG renamed .png peaks at the quality it already carries.
    if source_format not in LOSSLESS_FORMATS:
        remedy = 'produce a reference with `just render-lossless`'
        raise ValueError(
            f'{path} is {source_format}, already lossy, so PSNR flatters every quality; {remedy}'
        )
    shipped = (TARGET_W, TARGET_H)
    if reference.size != shipped:
        raise ValueError(
            f'{path} is {dims(reference.size)}, not the shipped {dims(shipped)}; '
            + 'the sizes below would describe another image'
        )

    print(f'{path}  {dims(shipped)}  {SUBSAMPLING_NAMES[subsampling]}\n')
    print(f'{"quality":>7}  {"size":>10}  {"PSNR":>7}')
    for quality in SWEEP_QUALITIES:
        flat = encode_texture(reference, quality=quality, subsampling=subsampling)
        with Image.open(io.BytesIO(flat)) as encoded:
            decoded = encoded.convert('RGB')
        print(
            f'{quality:>7}  {len(flat) / 1024:>9.1f}K  {psnr(reference, decoded):>6.2f}dB'
        )


def _check_source(source: Path, size: tuple[int, int], target: tuple[int, int]) -> None:
    source_aspect = size[0] / size[1]
    target_aspect = target[0] / target[1]
    if abs(source_aspect - target_aspect) / target_aspect > ASPECT_TOLERANCE:
        found = f'{dims(size)} (aspect {source_aspect:.3f})'
        wanted = f'{dims(target)} (aspect {target_aspect:.3f})'
        raise ValueError(
            f'{source} is {found}, target is {wanted}; crop or reproject it to equirectangular'
        )
    # The aspect tolerance alone admits a source a percent short in height,
    # which would stretch the last rows, so check both dimensions.
    if size[0] < target[0] or size[1] < target[1]:
        raise ValueError(
            f'{source} is {dims(size)}, below the {dims(target)} target; upscaling would blur it'
        )


def downsample_file(source: Path, output: Path, *, force: bool) -> None:
    if output.exists() and not force:
        raise FileExistsError(f'{output} already exists; pass --force to overwrite')

    target = (TARGET_W, TARGET_H)
    source_bytes = source.stat().st_size
    with Image.open(source) as original:
        source_size = original.size
        _check_source(source, source_size, target)
        resized = downsample(original, target)

    write_texture(resized, output)

    print(describe(source, source_size, source_bytes))
    resident = f'{resident_mib(target):.1f} MiB decoded on the device'
    print(f'{describe(output, target, output.stat().st_size)}  {resident}')


@dataclass
class Sweep:
    """Size against PSNR over qualities."""

    reference: Positional[Path]
    subsampling: Subsampling = JPEG_SUBSAMPLING


@dataclass
class Downsample:
    """Resize an image and encode it as a shipped texture."""

    source: Positional[Path]
    output: Positional[Path]
    force: bool = False  # overwrite an existing output


def main() -> None:
    # tyro declares `cli`'s `default` sentinel as Any (tyro/_singleton.py `MISSING_NONPROP`).
    command: Sweep | Downsample = tyro.cli(Sweep | Downsample, description=__doc__)  # pyright: ignore[reportAny]
    try:
        match command:
            case Sweep():
                sweep(command.reference, command.subsampling)
            case Downsample():
                downsample_file(command.source, command.output, force=command.force)
    except (OSError, ValueError) as error:
        sys.exit(f'{Path(sys.argv[0]).name}: {error}')


if __name__ == '__main__':
    main()
