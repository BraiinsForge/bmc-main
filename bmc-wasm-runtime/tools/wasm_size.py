#!/usr/bin/env python3
"""Measure and optionally enforce WASM binary size limits.

Builds the widget in release mode, runs wasm-opt, and prints a size
breakdown. With --check, exits non-zero if the optimized binary exceeds
the limit.

Usage:
    ./tools/wasm_size.py                        # hello-widget, report only
    ./tools/wasm_size.py -e iss-position        # different widget
    ./tools/wasm_size.py --check                # enforce 1MB limit
    ./tools/wasm_size.py --check --limit 65536  # custom limit
"""

import argparse
import subprocess
import sys
from pathlib import Path

from _common import build_example_wasm, require_tools

DEFAULT_SIZE_LIMIT = 1_048_576


def build_and_optimize(example: str) -> tuple[Path, Path]:
    release = build_example_wasm(example)
    optimized = release.with_suffix('.opt.wasm')

    subprocess.run(
        ['wasm-opt', '-Oz', str(release), '-o', str(optimized)],
        check=True,
    )

    return release, optimized


def print_sizes(example: str, release: Path, optimized: Path) -> None:
    rel_size = release.stat().st_size
    opt_size = optimized.stat().st_size

    print(f'=== WASM Binary Size: {example} ===')
    print(f'Release:   {rel_size:,} bytes ({rel_size / 1024:.1f} KB)')
    print(f'Optimized: {opt_size:,} bytes ({opt_size / 1024:.1f} KB)')
    print()

    result = subprocess.run(
        ['wasm-objdump', '-h', str(optimized)],
        capture_output=True,
        text=True,
    )
    if result.returncode == 0:
        print('Sections:')
        print(result.stdout)


def main() -> None:
    parser = argparse.ArgumentParser(description='Measure WASM widget binary size.')
    parser.add_argument(
        '-e', '--example', default='hello-widget', help='widget example name'
    )
    parser.add_argument(
        '--check', action='store_true', help='enforce size limit (exit 1 if exceeded)'
    )
    parser.add_argument(
        '--limit',
        type=int,
        default=DEFAULT_SIZE_LIMIT,
        help=f'size limit in bytes (default: {DEFAULT_SIZE_LIMIT:,})',
    )
    args = parser.parse_args()

    require_tools(('wasm-opt', 'binaryen'), ('wasm-objdump', 'wabt'))
    release, optimized = build_and_optimize(args.example)
    print_sizes(args.example, release, optimized)

    if args.check:
        opt_size = optimized.stat().st_size
        if opt_size > args.limit:
            print(f'FAIL: optimized WASM is {opt_size:,} bytes (limit: {args.limit:,})')
            sys.exit(1)
        print(f'OK: optimized WASM is {opt_size:,} bytes (limit: {args.limit:,})')


if __name__ == '__main__':
    main()
