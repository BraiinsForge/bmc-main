"""
Shared utilities for WASM runtime development tools.
"""

import gzip
import json
import re
import shutil
import subprocess
import sys
from collections import Counter
from pathlib import Path

WASM_TARGET = 'wasm32-unknown-unknown'

# Standard widget capture sizes: (name, WxH)
CAPTURE_SIZES = [
    ('full', '1280x480'),
    ('large', '638x480'),
    ('medium', '638x238'),
    ('small', '317x238'),
]


def require_tools(*tools: tuple[str, str]) -> None:
    """Exit if any required CLI tool is missing. Each item is (binary, package)."""
    for binary, package in tools:
        if not shutil.which(binary):
            print(
                f'Error: {binary} not found. Install package: {package}',
                file=sys.stderr,
            )
            sys.exit(1)


def build_example_wasm(example: str) -> Path:
    """Build a widget example in release mode and return the .wasm path."""
    example_dir = Path(f'examples/{example}')
    wasm_name = example.replace('-', '_')

    if not example_dir.is_dir():
        available = sorted(p.name for p in Path('examples').iterdir() if p.is_dir())
        print(
            f"Error: example '{example}' not found (no directory {example_dir}).\n"
            f'Available examples: {", ".join(available)}',
            file=sys.stderr,
        )
        sys.exit(1)

    subprocess.run(
        ['cargo', 'build', '--release', '--target', WASM_TARGET],
        cwd=example_dir,
        check=True,
    )

    return example_dir / 'target' / WASM_TARGET / 'release' / f'{wasm_name}.wasm'


def extract_crate(sym: str) -> str:
    """Extract crate name from a Rust symbol."""
    if sym.startswith('0x'):
        return '[unsymbolized]'
    m = re.match(r'<?(\w+)(?:::|_ir::)', sym)
    if m:
        crate = m.group(1)
        if crate in ('wasmi', 'wasmi_ir'):
            return 'wasmi'
        return crate
    if '::' not in sym and '<' not in sym:
        return '[system]'
    return '[other]'


def load_symbols(profile_path: Path) -> dict[str, str]:
    """Load symbols.json sidecar if it exists next to the profile."""
    symbols_path = profile_path.parent / 'symbols.json'
    if symbols_path.exists():
        with open(symbols_path) as f:
            return json.load(f)
    return {}


def load_profile_data(profile_path: Path) -> tuple[dict, dict[str, str]]:
    """Load a gzipped samply profile and its symbols sidecar."""
    p = Path(profile_path)
    if not p.exists():
        print(f'Error: {p} does not exist', file=sys.stderr)
        sys.exit(1)
    with gzip.open(p, 'rt') as f:
        data = json.load(f)
    return data, load_symbols(p)


def find_testbed_thread(data: dict) -> dict | None:
    """Find the main testbed thread (largest sample count)."""
    testbed_threads = [
        t
        for t in data['threads']
        if t['name'] == 'testbed' and t['samples']['length'] > 0
    ]
    if not testbed_threads:
        return None
    return max(testbed_threads, key=lambda t: t['samples']['length'])


def crate_breakdown_from_thread(
    thread: dict,
    symbols: dict[str, str],
) -> tuple[Counter[str], int]:
    """Compute inclusive crate-level breakdown from a samply thread.

    Returns (crate_counter, total_samples). The counter maps crate name to
    sample count, deduplicated per-stack (a crate appearing multiple times
    in one stack counts once).
    """
    strings: list[str] = thread['stringArray']
    func_table: dict = thread['funcTable']
    frame_table: dict = thread['frameTable']
    stack_table: dict = thread['stackTable']
    samples: dict = thread['samples']

    stack_counts: Counter[int | None] = Counter(samples['stack'])
    prefixes: list[int | None] = stack_table['prefix']
    frame_list: list[int] = stack_table['frame']

    total = sum(stack_counts.values())

    crate_time: Counter[str] = Counter()
    for stack_idx, count in stack_counts.items():
        if stack_idx is None:
            continue
        seen_crates: set[str] = set()
        s: int | None = stack_idx
        while s is not None:
            fi = frame_list[s]
            func_idx = frame_table['func'][fi]
            sym = strings[func_table['name'][func_idx]]
            crate = extract_crate(symbols.get(sym, sym))
            if crate not in seen_crates:
                crate_time[crate] += count
                seen_crates.add(crate)
            s = prefixes[s]

    return crate_time, total
