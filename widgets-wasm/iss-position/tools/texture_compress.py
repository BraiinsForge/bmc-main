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

import argparse
import io
import sys
from pathlib import Path

import numpy as np
from _textures import (
    JPEG_SUBSAMPLING,
    SUBSAMPLING_NAMES,
    TARGET_H,
    TARGET_W,
    downsample,
    encode_texture,
    write_texture,
)
from PIL import Image

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
    return 10.0 * float(np.log10(255.0**2 / mse))


def describe(path: Path, size: tuple[int, int], size_bytes: int) -> str:
    return f'{path}  {size[0]}x{size[1]}  {size_bytes / 1024:.1f} KB'


def resident_mib(size: tuple[int, int]) -> float:
    """Both decoded copies the host keeps for a texture of `size`, in MiB."""
    return 2 * size[0] * size[1] * 4 / 1024 / 1024


def sweep(path: Path, subsampling: int) -> None:
    with Image.open(path) as opened:
        source_format = opened.format
        reference = opened.convert('RGB')
    # Checked on the decoded format, not the suffix:
    # a JPEG renamed .png peaks at the quality it already carries.
    if source_format not in LOSSLESS_FORMATS:
        raise ValueError(
            f'{path} is {source_format}, already lossy, so PSNR against it flatters'
            ' every quality; produce a reference with `just render-lossless`'
        )
    if reference.size != (TARGET_W, TARGET_H):
        raise ValueError(
            f'{path} is {reference.size[0]}x{reference.size[1]}, not the shipped'
            f' {TARGET_W}x{TARGET_H}; the sizes below would describe another image'
        )

    print(f'{path}  {TARGET_W}x{TARGET_H}  {SUBSAMPLING_NAMES[subsampling]}\n')
    print(f'{"quality":>7}  {"size":>10}  {"PSNR":>7}')
    for quality in SWEEP_QUALITIES:
        flat = encode_texture(reference, quality=quality, subsampling=subsampling)
        with Image.open(io.BytesIO(flat)) as encoded:
            decoded = encoded.convert('RGB')
        print(
            f'{quality:>7}  {len(flat) / 1024:>9.1f}K'
            f'  {psnr(reference, decoded):>6.2f}dB'
        )


def _check_source(source: Path, size: tuple[int, int], target: tuple[int, int]) -> None:
    source_aspect = size[0] / size[1]
    target_aspect = target[0] / target[1]
    if abs(source_aspect - target_aspect) / target_aspect > ASPECT_TOLERANCE:
        raise ValueError(
            f'{source} is {size[0]}x{size[1]} (aspect {source_aspect:.3f}), target is'
            f' {target[0]}x{target[1]} (aspect {target_aspect:.3f});'
            ' crop or reproject it to equirectangular first'
        )
    # The aspect tolerance alone admits a source a percent short in height,
    # which would stretch the last rows, so check both dimensions.
    if size[0] < target[0] or size[1] < target[1]:
        raise ValueError(
            f'{source} is {size[0]}x{size[1]}, below the {target[0]}x{target[1]}'
            ' target; upscaling it would ship a blurred texture'
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
    print(
        f'{describe(output, target, output.stat().st_size)}'
        f'  {resident_mib(target):.1f} MiB decoded on the device'
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = parser.add_subparsers(dest='command', required=True)

    sweep_parser = sub.add_parser('sweep', help='size against PSNR over qualities')
    sweep_parser.add_argument('reference', type=Path)
    sweep_parser.add_argument(
        '--subsampling',
        type=int,
        default=JPEG_SUBSAMPLING,
        choices=sorted(SUBSAMPLING_NAMES),
    )

    downsample_parser = sub.add_parser(
        'downsample', help='resize an image and encode it as a shipped texture'
    )
    downsample_parser.add_argument('source', type=Path)
    downsample_parser.add_argument('output', type=Path)
    downsample_parser.add_argument(
        '--force', action='store_true', help='overwrite an existing output'
    )

    args = parser.parse_args()
    try:
        if args.command == 'sweep':
            sweep(args.reference, args.subsampling)
        elif args.command == 'downsample':
            downsample_file(args.source, args.output, force=args.force)
        else:
            raise AssertionError(f'BUG: unhandled subcommand {args.command}')
    except (OSError, ValueError) as error:
        sys.exit(f'{parser.prog}: {error}')


if __name__ == '__main__':
    main()
