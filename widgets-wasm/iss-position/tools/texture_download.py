#!/usr/bin/env python3

"""Download earth texture candidates for the ISS widget.

Usage:
    python tools/texture_download.py [--force]

Downloads equirectangular earth textures from public sources.
Skips textures that are already downloaded unless --force is given.
See _textures.__doc__ for the UV mapping contract.
"""

import argparse
import subprocess
from pathlib import Path

from _textures import TARGET_H, TARGET_W, TEXTURE_DIR, TEXTURES, print_texture_contract


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--force', action='store_true', help='Re-download all textures')
    args = parser.parse_args()

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
        try:
            subprocess.run(
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
                check=True,
                capture_output=True,
            )
            size_kb: float = dest.stat().st_size / 1024
            print(f'    -> {size_kb:.0f} KB')
        except subprocess.CalledProcessError as e:
            print(f'    FAILED: {e.stderr.decode().strip()}')
            dest.unlink(missing_ok=True)

    print('\nDone. Preview with: python tools/texture_preview.py')


if __name__ == '__main__':
    main()
