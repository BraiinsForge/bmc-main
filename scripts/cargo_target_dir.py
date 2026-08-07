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

"""Print the cargo target directory for a workspace.

Runs `cargo metadata` for the workspace rooted at the given directory and
prints its absolute `target_directory`. Single source of truth for where
build artifacts land, shared by the wasm testbed/capture flows and the
gallery's hot reload so they all agree on the active target dir.

  cargo_target_dir.py WORKSPACE_ROOT    Print the absolute target dir for
                                        the workspace whose `Cargo.toml`
                                        lives at WORKSPACE_ROOT.
"""

import json
import os
import subprocess
import sys
from pathlib import Path


def fallback_target_dir(root: Path) -> str:
    target_dir_env = os.environ.get('CARGO_TARGET_DIR')
    if target_dir_env is None:
        return str(root / 'target')

    target_dir = Path(target_dir_env)
    if target_dir.is_absolute():
        return str(target_dir)
    return str(root / target_dir)


def target_dir(root: Path) -> str:
    try:
        out = subprocess.run(
            [
                'cargo',
                'metadata',
                '--manifest-path',
                str(root / 'Cargo.toml'),
                '--format-version=1',
                '--no-deps',
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    except FileNotFoundError:
        return fallback_target_dir(root)

    return json.loads(out)['target_directory']


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(f'Usage: {sys.argv[0]} WORKSPACE_ROOT')
    print(target_dir(Path(sys.argv[1]).resolve()))


if __name__ == '__main__':
    main()
