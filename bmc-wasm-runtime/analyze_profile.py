#!/usr/bin/env python3
"""Analyze samply profile and show hot functions."""

import json
import gzip
import subprocess
from collections import Counter
from pathlib import Path

PROFILE = Path('profile.json.gz')
BINARY = Path('../target/profiling/testbed')


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
        addr_self = Counter()

        for stack_idx, count in stack_counts.items():
            if stack_idx is None:
                continue
            frame_idx = stack_table['frame'][stack_idx]
            func_idx = frame_table['func'][frame_idx]
            addr_self[strings[func_table['name'][func_idx]]] += count

        total = sum(addr_self.values())

        # Batch symbolicate hex addresses
        addrs = [a for a, _ in addr_self.most_common(60) if a.startswith('0x')]
        symbols = {}

        if addrs and BINARY.exists():
            proc = subprocess.run(
                ['addr2line', '-f', '-C', '-e', str(BINARY)] + addrs,
                capture_output=True,
                text=True,
            )
            lines = proc.stdout.strip().split('\n')
            for i in range(0, len(lines), 2):
                if i // 2 < len(addrs):
                    symbols[addrs[i // 2]] = lines[i]

        print(f'=== Top Functions by SELF Time ({total} samples) ===\n')
        for addr, count in addr_self.most_common(40):
            pct = 100.0 * count / total
            name = symbols.get(addr, addr)
            if len(name) > 70:
                name = name[:67] + '...'
            print(f'{pct:5.1f}% ({count:4d})  {name}')


if __name__ == '__main__':
    main()
