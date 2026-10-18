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

"""Download earth texture candidates for the ISS widget.

Usage:
    python tools/texture_download.py [--force]

Downloads equirectangular earth textures from public sources.
Skips textures that are already downloaded unless --force is given.
See _textures.__doc__ for the UV mapping contract.
"""

import subprocess
from dataclasses import dataclass
from pathlib import Path

import tyro
from _textures import TARGET_H, TARGET_W, TEXTURE_DIR, TEXTURES, print_texture_contract


@dataclass
class Args:
    force: bool = False  # re-download all textures


def main() -> None:
    # tyro declares `cli`'s `default` sentinel as Any (tyro/_singleton.py `MISSING_NONPROP`).
    args = tyro.cli(Args, description=__doc__)  # pyright: ignore[reportAny]

    TEXTURE_DIR.mkdir(exist_ok=True)
    print_texture_contract((TARGET_W, TARGET_H))

    for tex in TEXTURES:
        if not tex['url']:
            continue

        dest: Path = TEXTURE_DIR / f'{tex["id"]}.{tex["ext"]}'

        if dest.exists() and not args.force:
            print(f'  {tex["name"]}: already downloaded')
            continue

        print(f'  Downloading {tex["name"]}...')
        result = subprocess.run(
            [
                'curl',
                '-fSL',
                '--connect-timeout',
                '15',
                '--max-time',
                '120',
                '-A',
                'Mozilla/5.0',
                '-o',
                str(dest),
                tex['url'],
            ],
            check=False,
            capture_output=True,
        )
        if result.returncode != 0:
            print(f'    FAILED: {result.stderr.decode().strip()}')
            dest.unlink(missing_ok=True)
            continue
        size_kb: float = dest.stat().st_size / 1024
        print(f'    -> {size_kb:.0f} KB')

    print('\nDone. Preview with: python tools/texture_preview.py')


if __name__ == '__main__':
    main()
