#!/usr/bin/env python3
"""Analyze samply profile and show hot functions."""

import json
import gzip
import subprocess
import re
from collections import Counter
from pathlib import Path

PROFILE = Path('profile.json.gz')
BINARY = Path('../target/profiling/testbed')


def extract_crate(name):
    """Extract crate name from a Rust symbol."""
    # Skip hex addresses
    if name.startswith('0x'):
        return '[unsymbolized]'
    # Match crate::module::... pattern
    m = re.match(r'<?(\w+)(?:::|_ir::)', name)
    if m:
        crate = m.group(1)
        # Merge wasmi variants
        if crate in ('wasmi', 'wasmi_ir'):
            return 'wasmi'
        return crate
    # Bare function names (C/system)
    if '::' not in name and '<' not in name:
        return '[system]'
    return '[other]'


def symbolize(names, binary):
    """Batch symbolize hex addresses with addr2line."""
    addrs = [a for a in names if a.startswith('0x')]
    if not addrs or not binary.exists():
        return {}
    proc = subprocess.run(
        ['addr2line', '-f', '-C', '-e', str(binary)] + addrs,
        capture_output=True,
        text=True,
    )
    symbols = {}
    lines = proc.stdout.strip().split('\n')
    for i in range(0, len(lines), 2):
        if i // 2 < len(addrs):
            symbols[addrs[i // 2]] = lines[i]
    return symbols


def main():
    with gzip.open(PROFILE, 'rt') as f:
        data = json.load(f)

    for t in data['threads']:
        if t['name'] != 'testbed' or t['samples']['length'] == 0:
            continue

        strings = t['stringArray']
        func_table = t['funcTable']
        frame_table = t['frameTable']
        stack_table = t['stackTable']
        samples = t['samples']

        stack_counts = Counter(samples['stack'])
        func_self = Counter()  # leaf/self time
        func_inclusive = Counter()  # anywhere in stack

        prefixes = stack_table['prefix']
        frames = stack_table['frame']

        for stack_idx, count in stack_counts.items():
            if stack_idx is None:
                continue

            # Self time: leaf frame only
            frame_idx = frames[stack_idx]
            func_idx = frame_table['func'][frame_idx]
            name = strings[func_table['name'][func_idx]]
            func_self[name] += count

            # Inclusive time: walk the full stack
            seen = set()
            s = stack_idx
            while s is not None:
                frame_idx = frames[s]
                func_idx = frame_table['func'][frame_idx]
                name = strings[func_table['name'][func_idx]]
                if name not in seen:
                    func_inclusive[name] += count
                    seen.add(name)
                s = prefixes[s]

        total = sum(func_self.values())

        # Symbolize hex addresses
        all_names = set(func_self.keys()) | set(func_inclusive.keys())
        symbols = symbolize(all_names, BINARY)

        def resolve(name):
            return symbols.get(name, name)

        def truncate(name, width=80):
            return name[: width - 3] + '...' if len(name) > width else name

        # === Crate-level breakdown (inclusive) ===
        crate_time = Counter()
        for name, count in func_inclusive.items():
            crate_time[extract_crate(resolve(name))] += count

        print(f'=== Crate Breakdown — inclusive ({total} samples) ===\n')
        for crate, count in crate_time.most_common(15):
            pct = 100.0 * count / total
            print(f'{pct:5.1f}%  {crate}')

        # === Top functions by INCLUSIVE time ===
        # Deduplicate: merge same resolved name at different addresses
        merged_inclusive = Counter()
        for name, count in func_inclusive.items():
            merged_inclusive[resolve(name)] += count

        print(f'\n=== Top Functions — inclusive ({total} samples) ===\n')
        for name, count in merged_inclusive.most_common(30):
            pct = 100.0 * count / total
            print(f'{pct:5.1f}% ({count:5d})  {truncate(name)}')

        # === Top functions by SELF time ===
        merged_self = Counter()
        for name, count in func_self.items():
            merged_self[resolve(name)] += count

        print(f'\n=== Top Functions — self ({total} samples) ===\n')
        for name, count in merged_self.most_common(25):
            pct = 100.0 * count / total
            print(f'{pct:5.1f}% ({count:5d})  {truncate(name)}')


if __name__ == '__main__':
    main()
