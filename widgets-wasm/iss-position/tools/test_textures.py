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

"""Guards on the invariants the globe shader depends on.

Run with `just test`, or: uv run --project tools pytest tools

Deliberately outside `just validate` and the GitLab pipeline:
this covers one contained asset pipeline, hand-regenerated and seldom touched,
so a job on every push buys little.
`nix build .#checks.<system>.iss-texture-tools` runs it when a gate is wanted.
"""

from pathlib import Path

import numpy as np
import pytest
from _textures import (
    JPEG_QUALITY,
    JPEG_SUBSAMPLING,
    TARGET_H,
    TARGET_W,
    downsample,
    encode_texture,
    write_texture,
)
from PIL import Image
from texture_compress import SWEEP_QUALITIES, downsample_file, sweep


def flat_image(width: int, height: int) -> Image.Image:
    return Image.new('RGB', (width, height), (12, 24, 48))


def checkerboard(width: int, height: int) -> Image.Image:
    alternating = np.indices((height, width)).sum(axis=0) % 2
    return Image.fromarray((alternating * 255).astype(np.uint8), 'L').convert('RGB')


@pytest.mark.parametrize('suffix', ['.png', '.webp', ''])
def test_write_texture_rejects_non_jpeg_output(tmp_path: Path, suffix: str) -> None:
    """It always encodes JPEG, so another suffix would only mislabel the file."""
    with pytest.raises(ValueError, match='must be one of'):
        write_texture(flat_image(64, 32), tmp_path / f'texture{suffix}')


def test_write_texture_keeps_the_previous_texture_when_the_encode_fails(
    tmp_path: Path,
) -> None:
    """Encoding into a buffer first is what makes a failed regeneration harmless."""
    output = tmp_path / 'texture.jpg'
    output.write_bytes(b'previous')

    with pytest.raises(OSError):
        write_texture(Image.new('RGBA', (64, 32)), output)

    assert output.read_bytes() == b'previous'


def test_write_texture_keeps_the_previous_texture_when_the_write_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A full disk partway through must not truncate the texture already shipped."""
    output = tmp_path / 'texture.jpg'
    output.write_bytes(b'previous')

    def full_disk(self: Path, data: bytes) -> int:
        with open(self, 'wb'):  # a real write truncates before it runs out of room
            pass
        raise OSError('No space left on device')

    monkeypatch.setattr(Path, 'write_bytes', full_disk)

    with pytest.raises(OSError):
        write_texture(flat_image(64, 32), output)

    assert output.read_bytes() == b'previous'
    assert list(tmp_path.iterdir()) == [output], 'staging file left behind'


def test_downsample_rejects_a_differently_shaped_source(tmp_path: Path) -> None:
    """The catalog holds square Mercator tiles; resizing one stretches geography."""
    source = tmp_path / 'mercator.png'
    flat_image(512, 512).save(source)

    with pytest.raises(ValueError, match='reproject'):
        downsample_file(source, tmp_path / 'out.jpg', force=False)


def test_downsample_rejects_a_source_below_the_target(tmp_path: Path) -> None:
    """Enlarging a thumbnail yields a blurred texture at the full memory cost."""
    source = tmp_path / 'small.png'
    flat_image(320, 160).save(source)

    with pytest.raises(ValueError, match='below the'):
        downsample_file(source, tmp_path / 'out.jpg', force=False)


def test_downsample_rejects_a_source_short_of_the_target_height(tmp_path: Path) -> None:
    """A percent short in height passes the aspect gate and stretches the last rows."""
    source = tmp_path / 'squat.png'
    flat_image(TARGET_W, TARGET_H - 5).save(source)

    with pytest.raises(ValueError, match='below the'):
        downsample_file(source, tmp_path / 'out.jpg', force=False)


def test_downsample_accepts_a_source_the_shipped_aspect(tmp_path: Path) -> None:
    source = tmp_path / 'world.png'
    flat_image(2048, 1024).save(source)
    output = tmp_path / 'out.jpg'

    downsample_file(source, output, force=False)

    with Image.open(output) as written:
        assert written.size == (TARGET_W, TARGET_H)


def test_downsample_refuses_to_clobber_an_existing_texture(tmp_path: Path) -> None:
    """`downsample src shipped.jpg` on a typo would otherwise destroy the asset."""
    source = tmp_path / 'world.png'
    flat_image(2048, 1024).save(source)
    output = tmp_path / 'out.jpg'
    output.write_bytes(b'existing')

    with pytest.raises(FileExistsError, match='--force'):
        downsample_file(source, output, force=False)
    assert output.read_bytes() == b'existing'

    downsample_file(source, output, force=True)
    with Image.open(output) as written:
        assert written.size == (TARGET_W, TARGET_H)


def test_downsample_averages_neighbours_instead_of_dropping_them() -> None:
    """Point sampling would alias the coastlines the supersampled render smoothed."""
    reduced = downsample(checkerboard(64, 32), (32, 16))

    assert not set(np.asarray(reduced).ravel().tolist()).issubset({0, 255})


def test_sweep_rejects_a_lossy_reference_behind_a_lossless_suffix(
    tmp_path: Path,
) -> None:
    """PSNR peaks at the quality the reference already carries, reading as a sweet spot."""
    reference = tmp_path / 'reference.png'
    reference.write_bytes(encode_texture(flat_image(TARGET_W, TARGET_H)))

    with pytest.raises(ValueError, match='already lossy'):
        sweep(reference, JPEG_SUBSAMPLING)


def test_sweep_rejects_a_reference_off_the_shipped_size(tmp_path: Path) -> None:
    """A reference at render resolution reports sizes for an image nobody ships."""
    reference = tmp_path / 'reference.png'
    flat_image(TARGET_W * 2, TARGET_H * 2).save(reference)

    with pytest.raises(ValueError, match='not the shipped'):
        sweep(reference, JPEG_SUBSAMPLING)


def test_sweep_reports_the_size_each_quality_would_actually_ship(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """A loop that dropped its encode settings would print one reading several times."""
    image = checkerboard(TARGET_W, TARGET_H)
    reference = tmp_path / 'reference.png'
    image.save(reference)

    sweep(reference, JPEG_SUBSAMPLING)

    reported = [
        line.split()[1]
        for line in capsys.readouterr().out.splitlines()
        if line.endswith('dB')
    ]
    encoded = (
        encode_texture(image, quality=quality, subsampling=JPEG_SUBSAMPLING)
        for quality in SWEEP_QUALITIES
    )
    assert reported == [f'{len(flat) / 1024:.1f}K' for flat in encoded]


def test_quality_is_one_the_sweep_measured() -> None:
    """Off the sweep grid there is no size/PSNR reading behind the choice."""
    assert JPEG_QUALITY in SWEEP_QUALITIES
