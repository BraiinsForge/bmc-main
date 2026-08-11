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

"""Resolve a wasm widget identifier to its workspace root and directory.

The identifier may be the widget's directory name OR its cargo package name.
The two differ when a crate is renamed to dodge a dependency clash — e.g. the
`image` widget lives in `widgets-wasm/image/` but its package is `image-widget`
(the bare `image` name collides with the `image` crate).

Modes:

  widget_root.py NAME          Print the workspace root holding the widget
                               (the directory with the workspace `Cargo.toml`).
  widget_root.py --dir NAME    Print the widget's directory name.
  widget_root.py               Print all known workspace roots, space-separated.

Exits 1 if NAME is unknown or ambiguous.

Source of truth for which roots host wasm widgets. Adding a new workspace means
adding it here and (separately) wiring the Nix-side equivalent in
`workspace.nix` / `nix/wasm-widgets.nix`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path
from typing import NoReturn

REPO_ROOT = Path(__file__).resolve().parent.parent.parent

WIDGET_ROOTS = [
    REPO_ROOT / 'widgets-wasm-examples',
    REPO_ROOT / 'widgets-wasm',
]


def _fail(message: str) -> NoReturn:
    print(message, file=sys.stderr)
    sys.exit(1)


def _package_name(cargo_toml: Path) -> str | None:
    """Read `[package].name` from a Cargo.toml without a TOML dependency."""
    in_package = False
    for line in cargo_toml.read_text().splitlines():
        stripped = line.strip()
        if stripped.startswith('['):
            in_package = stripped == '[package]'
        elif in_package:
            match = re.match(r'name\s*=\s*"([^"]+)"', stripped)
            if match:
                return match.group(1)
    return None


def find(name: str) -> tuple[Path, str]:
    """Resolve a directory name or package name to (workspace_root, dir_name)."""
    direct = [r for r in WIDGET_ROOTS if (r / name / 'Cargo.toml').is_file()]
    if len(direct) > 1:
        _fail(
            f"widget '{name}' is ambiguous; found in: {', '.join(str(r) for r in direct)}"
        )
    if direct:
        return direct[0], name

    by_package: list[tuple[Path, str]] = []
    for root in WIDGET_ROOTS:
        if not root.is_dir():
            continue
        for crate in sorted(root.iterdir()):
            cargo = crate / 'Cargo.toml'
            if cargo.is_file() and _package_name(cargo) == name:
                by_package.append((root, crate.name))
    if len(by_package) > 1:
        locations = ', '.join(f'{r}/{d}' for r, d in by_package)
        _fail(f"package '{name}' is ambiguous; found in: {locations}")
    if by_package:
        return by_package[0]

    roots = ', '.join(str(r) for r in WIDGET_ROOTS)
    _fail(f"widget '{name}' not found by directory or package name in: {roots}")


def resolve(name: str) -> Path:
    """Workspace root holding the widget `name` (directory or package name)."""
    return find(name)[0]


def main() -> None:
    args = sys.argv[1:]
    if not args:
        print(' '.join(str(r) for r in WIDGET_ROOTS))
        return
    if args[0] == '--dir':
        if len(args) != 2:
            sys.exit(f'Usage: {sys.argv[0]} --dir WIDGET_NAME')
        print(find(args[1])[1])
        return
    if len(args) != 1:
        sys.exit(f'Usage: {sys.argv[0]} [--dir] [WIDGET_NAME]')
    print(find(args[0])[0])


if __name__ == '__main__':
    main()
