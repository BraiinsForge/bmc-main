#!/usr/bin/env nix
#!nix shell nixpkgs#python312
#!nix --command python3
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
Batch-symbolicate a samply profile using addr2line.

Extracts all hex addresses from the profile's funcTable, resolves them
against the binary that was used during recording, and writes a
symbols.json mapping file. This must be run while the binary still
matches the profile (i.e. right after recording, before rebuilding).

Usage:
    ./perf_symbolicate.py <profile.json.gz> <binary> [symbols.json]

The output defaults to symbols.json next to the profile.
"""

import gzip
import json
import subprocess
import sys
from pathlib import Path


def main() -> None:
    if len(sys.argv) < 3:
        print(
            f'Usage: {sys.argv[0]} <profile.json.gz> <binary> [symbols.json]',
            file=sys.stderr,
        )
        sys.exit(1)

    profile_path = Path(sys.argv[1])
    binary_path = Path(sys.argv[2])

    if len(sys.argv) > 3:
        output_path = Path(sys.argv[3])
    else:
        output_path = profile_path.parent / 'symbols.json'

    if not profile_path.exists():
        print(f'Error: {profile_path} does not exist', file=sys.stderr)
        sys.exit(1)
    if not binary_path.exists():
        print(f'Error: {binary_path} does not exist', file=sys.stderr)
        sys.exit(1)

    with gzip.open(profile_path, 'rt') as f:
        data: dict = json.load(f)

    # Collect all unique hex addresses across all threads
    addrs: set[str] = set()
    for t in data['threads']:
        if t['samples']['length'] == 0:
            continue
        strings: list[str] = t['stringArray']
        for name_idx in t['funcTable']['name']:
            name = strings[name_idx]
            if name.startswith('0x'):
                addrs.add(name)

    if not addrs:
        print('No hex addresses found — profile may already be symbolicated')
        sys.exit(0)

    print(f'Resolving {len(addrs)} addresses against {binary_path.name}...')

    addr_list = sorted(addrs)
    proc = subprocess.run(
        ['addr2line', '-f', '-C', '-e', str(binary_path), *addr_list],
        capture_output=True,
        text=True,
    )

    lines = proc.stdout.strip().split('\n')
    symbols: dict[str, str] = {}
    for i in range(0, len(lines), 2):
        idx = i // 2
        if idx < len(addr_list):
            func_name = lines[i]
            if func_name not in ('??', ''):
                symbols[addr_list[idx]] = func_name

    resolved = len(symbols)
    total = len(addr_list)
    print(f'Resolved {resolved}/{total} addresses ({100 * resolved // total}%)')

    with open(output_path, 'w') as f:
        json.dump(symbols, f, separators=(',', ':'))

    print(f'Wrote {output_path}')


if __name__ == '__main__':
    main()
