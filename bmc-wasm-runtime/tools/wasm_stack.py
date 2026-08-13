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

"""Read the stack reservation baked into a wasm module by the linker.

`-zstack-size` is a link-time argument with no runtime representation, so the
only way to confirm it survived is to read the linked module back. wasm-ld lays
the shadow stack out downwards from its initial `__stack_pointer`, placing
static data immediately above, so that pointer is the reservation.

The capture profiler in `src/stack_profile.rs` validates the same global
with `wasmparser`. Keep the two sets of assumptions aligned.

Reports every module unless `--expect` is given, which turns the report into an
assertion and exits 1 on the first module that disagrees.

Usage:
    wasm_stack.py MODULE...
    wasm_stack.py --expect 65536 MODULE...
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import NoReturn

WASM_MAGIC = b'\x00asm'
GLOBAL_SECTION = 6
VALTYPE_I32 = 0x7F
OP_I32_CONST = 0x41
OP_END = 0x0B


def _fail(message: str) -> NoReturn:
    print(message, file=sys.stderr)
    sys.exit(1)


def _uleb(data: bytes, pos: int) -> tuple[int, int]:
    value = 0
    shift = 0
    while True:
        byte = data[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            return value, pos


def _sleb(data: bytes, pos: int) -> tuple[int, int]:
    value = 0
    shift = 0
    while True:
        byte = data[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            if shift < 64 and byte & 0x40:
                value -= 1 << shift
            return value, pos


def stack_size(module: Path) -> int:
    """Initial value of the module's `__stack_pointer` global."""
    data = module.read_bytes()
    if data[:4] != WASM_MAGIC:
        _fail(f'{module}: not a wasm module')

    pos = 8
    while pos < len(data):
        section_id = data[pos]
        pos += 1
        size, pos = _uleb(data, pos)
        end = pos + size
        if section_id == GLOBAL_SECTION:
            count, pos = _uleb(data, pos)
            if count == 0:
                _fail(f'{module}: empty global section, no __stack_pointer')
            # wasm-ld emits `__stack_pointer` as the module's first global.
            # Release modules carry no name section to confirm that by name,
            # so the shape is asserted instead.
            valtype = data[pos]
            mutable = data[pos + 1]
            pos += 2
            if valtype != VALTYPE_I32 or not mutable:
                _fail(f'{module}: first global is not a mutable i32')
            if data[pos] != OP_I32_CONST:
                _fail(f'{module}: __stack_pointer is not an i32.const')
            value, pos = _sleb(data, pos + 1)
            if data[pos] != OP_END:
                _fail(f'{module}: malformed __stack_pointer initialiser')
            return value
        pos = end

    _fail(f'{module}: no global section')


def main() -> None:
    args = sys.argv[1:]
    expect: int | None = None
    if args and args[0] == '--expect':
        if len(args) < 2:
            sys.exit(f'Usage: {sys.argv[0]} [--expect BYTES] MODULE...')
        expect = int(args[1])
        args = args[2:]
    if not args:
        sys.exit(f'Usage: {sys.argv[0]} [--expect BYTES] MODULE...')

    for name in args:
        module = Path(name)
        size = stack_size(module)
        if expect is None:
            print(f'{module.name}: {size} bytes')
        elif size != expect:
            _fail(
                f'{module.name}: stack reservation is {size} bytes, expected {expect}. '
                'The linker argument in workspace.nix did not reach this module.'
            )


if __name__ == '__main__':
    main()
