#!/usr/bin/env nix
#!nix shell nixpkgs#python312 nixpkgs#odiff
#!nix --command python3

"""
Analyze a combined profile and show hot functions.

Usage:
    ./perf_analyze.py <combined.json.gz>

Reads the funcTable symbolized in place by perf_finalize.py — the unresolved
frames stay as raw hex addresses. The symbols.json sidecar is not consumed.
"""

import gzip
import json
import sys
from collections import Counter
from pathlib import Path

from _common import (
    crate_breakdown_from_thread,
    find_testbed_thread,
)


def truncate(s: str, width: int = 80) -> str:
    return s[: width - 3] + '...' if len(s) > width else s


def main() -> None:
    if len(sys.argv) < 2:
        print(f'Usage: {sys.argv[0]} <combined.json.gz>', file=sys.stderr)
        sys.exit(1)

    path = Path(sys.argv[1])
    if not path.exists():
        print(f'Error: {path} does not exist', file=sys.stderr)
        sys.exit(1)
    with gzip.open(path, 'rt') as f:
        data = json.load(f)

    thread = find_testbed_thread(data)
    if thread is None:
        print('No testbed thread found in profile', file=sys.stderr)
        sys.exit(1)

    strings: list[str] = thread['stringArray']
    func_table: dict = thread['funcTable']
    frame_table: dict = thread['frameTable']
    stack_table: dict = thread['stackTable']
    samples: dict = thread['samples']

    stack_counts: Counter[int | None] = Counter(samples['stack'])
    func_self: Counter[str] = Counter()
    func_inclusive: Counter[str] = Counter()

    prefixes: list[int | None] = stack_table['prefix']
    frame_list: list[int] = stack_table['frame']

    for stack_idx, count in stack_counts.items():
        if stack_idx is None:
            continue

        fi = frame_list[stack_idx]
        func_idx = frame_table['func'][fi]
        sym = strings[func_table['name'][func_idx]]
        func_self[sym] += count

        seen: set[str] = set()
        s: int | None = stack_idx
        while s is not None:
            fi = frame_list[s]
            func_idx = frame_table['func'][fi]
            sym = strings[func_table['name'][func_idx]]
            if sym not in seen:
                func_inclusive[sym] += count
                seen.add(sym)
            s = prefixes[s]

    total = sum(func_self.values())

    # Crate-level breakdown (uses shared helper)
    crate_time, _ = crate_breakdown_from_thread(thread, {})

    print(f'=== Crate Breakdown — inclusive ({total} samples) ===\n')
    for crate, count in crate_time.most_common(15):
        pct = 100.0 * count / total
        print(f'{pct:5.1f}%  {crate}')

    print(f'\n=== Top Functions — inclusive ({total} samples) ===\n')
    for sym, count in func_inclusive.most_common(30):
        pct = 100.0 * count / total
        print(f'{pct:5.1f}% ({count:5d})  {truncate(sym)}')

    print(f'\n=== Top Functions — self ({total} samples) ===\n')
    for sym, count in func_self.most_common(25):
        pct = 100.0 * count / total
        print(f'{pct:5.1f}% ({count:5d})  {truncate(sym)}')


if __name__ == '__main__':
    main()
