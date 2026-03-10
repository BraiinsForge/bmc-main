#!/usr/bin/env nix
#!nix shell nixpkgs#python312
#!nix --command python3

"""
Capture widget screenshots at all standard sizes.

Builds the WASM widget (release), builds the capture binary, then runs it
once per size via headless EGL (no xvfb needed).

When capturing all examples, WASM builds run in parallel.

Usage:
    capture_run.py -e <example> [extra args...]
    capture_run.py                              # all examples
"""

import argparse
import json
import shutil
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from signal import SIGINT, signal

from _common import CAPTURE_SIZES, WASM_TARGET, build_example_wasm

signal(SIGINT, lambda *_: sys.exit(130))


ROOT = Path(__file__).resolve().parent.parent


def target_dir() -> Path:
    result = subprocess.run(
        ['cargo', 'metadata', '--format-version=1', '--no-deps'],
        capture_output=True,
        text=True,
        cwd=ROOT,
        check=True,
    )
    return Path(json.loads(result.stdout)['target_directory'])


def wasm_path(example: str) -> Path:
    name = example.replace('-', '_')
    return (
        ROOT
        / 'examples'
        / example
        / 'target'
        / WASM_TARGET
        / 'release'
        / f'{name}.wasm'
    )


def build_one(example: str) -> tuple[str, bool]:
    """Build a single example WASM. Returns (name, success)."""
    try:
        build_example_wasm(example)
        return example, True
    except subprocess.CalledProcessError:
        return example, False


def main() -> int:
    parser = argparse.ArgumentParser(description='Capture widget screenshots')
    parser.add_argument(
        '-e',
        '--example',
        help='Example widget name (omit to capture all examples)',
    )
    parser.add_argument('extra', nargs='*', help='Extra args passed to capture binary')
    args = parser.parse_args()

    examples = [args.example] if args.example else all_examples()

    # Build capture binary once
    subprocess.run(
        ['cargo', 'build', '--features', 'capture', '--bin', 'capture'],
        cwd=ROOT,
        check=True,
    )
    binary = target_dir() / 'debug' / 'capture'

    # Build WASM examples (parallel when multiple)
    if len(examples) > 1:
        print(f'Building {len(examples)} WASM examples in parallel...', file=sys.stderr)
        with ThreadPoolExecutor() as pool:
            futures = {pool.submit(build_one, ex): ex for ex in examples}
            for future in as_completed(futures):
                name, ok = future.result()
                if not ok:
                    print(f'Error: failed to build {name}', file=sys.stderr)
                    return 1
    else:
        build_example_wasm(examples[0])

    # Run captures sequentially (each needs the GPU)
    for example in examples:
        if rc := capture_example(binary, wasm_path(example), example, args.extra):
            return rc

    return 0


def all_examples() -> list[str]:
    examples_dir = ROOT / 'examples'
    return sorted(
        p.name
        for p in examples_dir.iterdir()
        if p.is_dir() and (p / 'Cargo.toml').exists()
    )


def list_variants(binary: Path, wasm: Path) -> list[str]:
    """Query the capture binary for available KV variants."""
    result = subprocess.run(
        [str(binary), str(wasm), '--list-variants'],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return ['_default']
    return [v.strip() for v in result.stdout.strip().splitlines() if v.strip()]


def capture_example(
    binary: Path,
    wasm: Path,
    example: str,
    extra: list[str],
) -> int:
    capture_dir = ROOT / 'captures' / example
    current_dir = capture_dir / 'current'

    # Wipe previous captures so stale frames don't linger.
    if current_dir.exists():
        shutil.rmtree(current_dir)

    variants = list_variants(binary, wasm)

    for variant in variants:
        for size_name, dimensions in CAPTURE_SIZES:
            # _default variant goes directly into current/<size>/
            # named variants go into current/<variant>/<size>/
            if variant == '_default':
                output = capture_dir / 'current' / size_name
            else:
                output = capture_dir / 'current' / variant / size_name

            cmd = [
                str(binary),
                str(wasm),
                f'--size={dimensions}',
                f'--output={output}',
                *extra,
            ]
            if variant != '_default':
                cmd.append(f'--variant={variant}')

            result = subprocess.run(cmd)
            if result.returncode != 0:
                label = (
                    f'{example}/{variant}/{size_name}'
                    if variant != '_default'
                    else f'{example}/{size_name}'
                )
                print(
                    f'Error: capture failed for {label} (exit {result.returncode})',
                    file=sys.stderr,
                )
                return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
