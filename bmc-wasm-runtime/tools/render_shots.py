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


"""
Render every viewport a WASM widget supports, populated with live data.

Delegates to the capture tool's `--online --all-targets` mode: the widget fetches
its own data source (non-hermetic), the capture iterates the configured targets
the widget's manifest declares, and waits for the response before each shot — so
the frames show real values instead of "-" placeholders. Each
`<platform>/<viewport>/<dataset>/frame_0000.png` is flattened up to
`<out>/<platform>-<viewport>.png`.

Usage:
    ./tools/render_shots.py halving-countdown
    ./tools/render_shots.py halving-countdown -o /tmp/shots
"""

import argparse
import subprocess
from pathlib import Path

from _common import build_example_wasm

PATH_REPO_ROOT = Path(__file__).resolve().parent.parent.parent
PATH_DEFAULT_OUT = PATH_REPO_ROOT / '.cache' / 'screenshots'


def main() -> None:
    parser = argparse.ArgumentParser(
        description='Render every viewport a WASM widget supports, with live data.'
    )
    parser.add_argument('widget', help='widget directory or package name')
    parser.add_argument(
        '-o',
        '--output',
        type=Path,
        help=f'output directory (default: {PATH_DEFAULT_OUT}/<widget>/)',
    )
    args = parser.parse_args()

    output = args.output or PATH_DEFAULT_OUT / args.widget
    wasm_file = build_example_wasm(args.widget)

    subprocess.run(
        [
            'cargo',
            'run',
            '-q',
            '--features',
            'capture',
            '--bin',
            'capture',
            '--manifest-path',
            str(PATH_REPO_ROOT / 'bmc-wasm-runtime' / 'Cargo.toml'),
            '--',
            'run',
            str(wasm_file),
            '--all-targets',
            '--online',
            '--output',
            str(output),
        ],
        check=True,
    )

    # The capture writes `<out>/<platform>/<viewport>/<dataset>/frame_0000.png`;
    # flatten single-frame shots up to `<out>/<platform>-<viewport>.png`.
    # An online sweep renders one dataset per target, so that name is unique.
    for frame_dir in sorted(p for p in output.glob('*/*/*') if p.is_dir()):
        frames = sorted(frame_dir.glob('*.png'))
        if len(frames) == 1:
            platform, viewport, _dataset = frame_dir.relative_to(output).parts
            frames[0].rename(output / f'{platform}-{viewport}.png')

    # Prune what the flatten emptied, deepest first.
    for path in sorted((p for p in output.rglob('*') if p.is_dir()), reverse=True):
        if not any(path.iterdir()):
            path.rmdir()

    print(f'done: {output}')


if __name__ == '__main__':
    main()
