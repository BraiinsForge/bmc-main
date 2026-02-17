#!/usr/bin/env python3
"""Analyze samply profile and show hot functions.

Usage:
    ./analyze_profile.py <profile.json.gz>

Symbols are loaded from symbols.json next to the profile (produced by
symbolicate_profile.py or ``make profile``). Without it, raw hex addresses
are shown.
"""

import gzip
import json
import re
import sys
from collections import Counter
from pathlib import Path


def extract_crate(sym):
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


def load_symbols(profile_path):
    """Load symbols.json sidecar if it exists next to the profile."""
    symbols_path = Path(profile_path).parent / 'symbols.json'
    if symbols_path.exists():
        with open(symbols_path) as f:
            return json.load(f)
    return {}


def truncate(s, width=80):
    return s[: width - 3] + '...' if len(s) > width else s


def load_profile(profile_path):
    """Load profile data from path or stdin."""
    if profile_path == '-':
        return json.load(sys.stdin), {}

    p = Path(profile_path)
    if not p.exists():
        print(f'Error: {p} does not exist', file=sys.stderr)
        sys.exit(1)
    with gzip.open(p, 'rt') as f:
        data = json.load(f)
    return data, load_symbols(p)


def main():
    if len(sys.argv) < 2:
        print(f'Usage: {sys.argv[0]} <profile.json.gz>', file=sys.stderr)
        sys.exit(1)

    data, symbols = load_profile(sys.argv[1])

    if symbols:
        print(f'Loaded {len(symbols)} symbols from symbols.json\n', file=sys.stderr)

    def resolve(sym):
        return symbols.get(sym, sym)

    # Find the main testbed thread (largest sample count)
    testbed_threads = [
        t for t in data['threads']
        if t['name'] == 'testbed' and t['samples']['length'] > 0
    ]
    if not testbed_threads:
        print('No testbed thread found in profile', file=sys.stderr)
        sys.exit(1)
    main_thread = max(testbed_threads, key=lambda t: t['samples']['length'])

    for t in [main_thread]:

        strings = t['stringArray']
        func_table = t['funcTable']
        frame_table = t['frameTable']
        stack_table = t['stackTable']
        samples = t['samples']

        stack_counts = Counter(samples['stack'])
        func_self = Counter()
        func_inclusive = Counter()

        prefixes = stack_table['prefix']
        frame_list = stack_table['frame']

        for stack_idx, count in stack_counts.items():
            if stack_idx is None:
                continue

            fi = frame_list[stack_idx]
            func_idx = frame_table['func'][fi]
            sym = strings[func_table['name'][func_idx]]
            func_self[sym] += count

            seen = set()
            s = stack_idx
            while s is not None:
                fi = frame_list[s]
                func_idx = frame_table['func'][fi]
                sym = strings[func_table['name'][func_idx]]
                if sym not in seen:
                    func_inclusive[sym] += count
                    seen.add(sym)
                s = prefixes[s]

        total = sum(func_self.values())

        # Crate-level breakdown (inclusive)
        crate_time = Counter()
        for sym, count in func_inclusive.items():
            crate_time[extract_crate(resolve(sym))] += count

        print(f'=== Crate Breakdown — inclusive ({total} samples) ===\n')
        for crate, count in crate_time.most_common(15):
            pct = 100.0 * count / total
            print(f'{pct:5.1f}%  {crate}')

        # Top functions by inclusive time
        merged_inclusive = Counter()
        for sym, count in func_inclusive.items():
            merged_inclusive[resolve(sym)] += count

        print(f'\n=== Top Functions — inclusive ({total} samples) ===\n')
        for sym, count in merged_inclusive.most_common(30):
            pct = 100.0 * count / total
            print(f'{pct:5.1f}% ({count:5d})  {truncate(sym)}')

        # Top functions by self time
        merged_self = Counter()
        for sym, count in func_self.items():
            merged_self[resolve(sym)] += count

        print(f'\n=== Top Functions — self ({total} samples) ===\n')
        for sym, count in merged_self.most_common(25):
            pct = 100.0 * count / total
            print(f'{pct:5.1f}% ({count:5d})  {truncate(sym)}')


if __name__ == '__main__':
    main()
