#!/usr/bin/env python3
"""Compare perf reports and profiles across optimization phases.

Each phase is a directory containing (all optional, at least one required):
  perf.json          — testbed --perf-report output
  profile.json.gz    — samply profile
  symbols.json       — symbolication map from symbolicate_profile.py

Usage:
    # Compare all phases:
    ./compare_reports.py reports/00-baseline reports/01-render-loop-fix reports/02-cached-tree

    # Or with glob:
    ./compare_reports.py reports/*/
"""

import json
import sys
import gzip
import re
from collections import Counter
from pathlib import Path

# ANSI colors
RED = '\033[31m'
GREEN = '\033[32m'
YELLOW = '\033[33m'
BOLD = '\033[1m'
DIM = '\033[2m'
RESET = '\033[0m'

PERF_FILE = 'perf.json'
PROFILE_FILE = 'profile.json.gz'
SYMBOLS_FILE = 'symbols.json'

METRICS = [
    ('avg_frame_us', 'Frame'),
    ('avg_wasm_us', 'WASM'),
    ('avg_tree_us', 'Tree'),
    ('avg_layout_us', 'Layout'),
    ('avg_render_us', 'Render'),
    ('avg_flush_us', 'Flush'),
]

PERCENTILES = [
    ('p50_frame_us', 'p50'),
    ('p95_frame_us', 'p95'),
    ('p99_frame_us', 'p99'),
]


def fmt_us(us):
    """Format microseconds as ms with 1 decimal."""
    return f'{us / 1_000:.1f}ms'


def fmt_delta(before, after):
    """Format a delta with color: green if improved, red if regressed."""
    if before == 0:
        return f'{DIM}n/a{RESET}'
    delta = after - before
    pct = 100.0 * delta / before
    sign = '+' if delta >= 0 else ''
    color = GREEN if delta <= 0 else RED
    return f'{color}{sign}{pct:.1f}%{RESET}'


def phase_name(dir_path):
    """Extract human-readable phase name from directory name."""
    name = Path(dir_path).name
    return name.replace('-', ' ', 1).replace('-', ' ')


def load_phase(dir_path):
    """Load a phase directory."""
    d = Path(dir_path)
    if not d.is_dir():
        print(f'{RED}Error: {d} is not a directory{RESET}', file=sys.stderr)
        sys.exit(1)

    perf = d / PERF_FILE
    profile = d / PROFILE_FILE

    if not perf.exists() and not profile.exists():
        print(
            f'{RED}Error: {d} has neither {PERF_FILE} nor {PROFILE_FILE}{RESET}',
            file=sys.stderr,
        )
        sys.exit(1)

    report = {}
    if perf.exists():
        with open(perf) as f:
            report = json.load(f)

    report['_dir'] = str(d)
    report['_has_perf'] = perf.exists()
    report['_has_profile'] = profile.exists()

    return report


def print_comparison_table(reports):
    """Print side-by-side comparison of all reports (perf.json data)."""
    names = [phase_name(r['_dir']) for r in reports]
    col_w = max(14, max(len(n) for n in names) + 2)
    ansi_pad = len(GREEN) + len(RESET)

    # Header
    print(f'\n{BOLD}{"Metric":<12}', end='')
    for i, name in enumerate(names):
        print(f'{name:>{col_w}}', end='')
        if i > 0:
            print(f'{"Δ prev":>10}', end='')
    print(RESET)
    print('─' * (12 + len(names) * col_w + (len(names) - 1) * 10))

    def print_row(label, get_val, fmt_val, better_lower=True):
        print(f'{label:<12}', end='')
        prev_val = None
        for i, r in enumerate(reports):
            if not r['_has_perf']:
                print(f'{DIM}{"—":>{col_w}}{RESET}', end='')
                if i > 0:
                    print(' ' * 10, end='')
                prev_val = None
            else:
                val = get_val(r)
                print(f'{fmt_val(val):>{col_w}}', end='')
                if i > 0 and prev_val is not None:
                    delta = val - prev_val
                    pct = 100.0 * delta / prev_val if prev_val else 0
                    sign = '+' if delta >= 0 else ''
                    improved = delta <= 0 if better_lower else delta >= 0
                    color = GREEN if improved else RED
                    print(
                        f'{color}{sign}{pct:.1f}%{RESET}'.rjust(10 + ansi_pad), end=''
                    )
                elif i > 0:
                    print(' ' * 10, end='')
                prev_val = val
        print()

    print_row('FPS', lambda r: r['avg_fps'], lambda v: f'{v:.1f}', better_lower=False)
    for key, label in METRICS:
        print_row(label, lambda r, k=key: r.get(k, 0), fmt_us)
    print()
    for key, label in PERCENTILES:
        print_row(label, lambda r, k=key: r.get(k, 0), fmt_us)
    print()
    print_row(
        'Anim-only', lambda r: r.get('animation_only_pct', 0), lambda v: f'{v:.1f}%'
    )


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


def resolve_sym(symbols, sym):
    """Resolve a symbol address using a symbol map."""
    return symbols.get(sym, sym)


def load_profile_crate_breakdown(phase_dir):
    """Load a samply profile and return crate-level inclusive breakdown."""
    profile_path = Path(phase_dir) / PROFILE_FILE
    symbols_path = Path(phase_dir) / SYMBOLS_FILE

    with gzip.open(profile_path, 'rt') as f:
        data = json.load(f)

    symbols = {}
    if symbols_path.exists():
        with open(symbols_path) as f:
            symbols = json.load(f)

    # Find the main testbed thread (largest sample count)
    testbed_threads = [
        t
        for t in data['threads']
        if t['name'] == 'testbed' and t['samples']['length'] > 0
    ]
    if not testbed_threads:
        return {}, 0
    t = max(testbed_threads, key=lambda t: t['samples']['length'])

    strings = t['stringArray']
    func_table = t['funcTable']
    frame_table = t['frameTable']
    stack_table = t['stackTable']
    samples = t['samples']

    stack_counts = Counter(samples['stack'])

    prefixes = stack_table['prefix']
    frame_list = stack_table['frame']

    total = sum(stack_counts.values())

    # Crate-level breakdown (inclusive, deduplicated per-stack)
    crate_time = Counter()
    for stack_idx, count in stack_counts.items():
        if stack_idx is None:
            continue
        seen_crates = set()
        s = stack_idx
        while s is not None:
            fi = frame_list[s]
            func_idx = frame_table['func'][fi]
            sym = strings[func_table['name'][func_idx]]
            crate = extract_crate(resolve_sym(symbols, sym))
            if crate not in seen_crates:
                crate_time[crate] += count
                seen_crates.add(crate)
            s = prefixes[s]

    return {
        crate: 100.0 * count / total for crate, count in crate_time.most_common(15)
    }, total


def print_profile_comparison(reports, profile_data):
    """Print crate-level profile comparison."""
    names = [phase_name(r['_dir']) for r in reports]
    col_w = max(14, max(len(n) for n in names) + 2)

    print(f'\n{BOLD}=== Profile: Crate Breakdown (inclusive %) ==={RESET}\n')

    # Gather all crate names from phases that have profiles
    all_crates = set()
    for pd, total in profile_data:
        if total > 0:
            all_crates.update(pd.keys())

    # Sort by first non-empty profile's percentage (descending)
    first_valid = next((pd for pd, total in profile_data if total > 0), {})
    sorted_crates = sorted(
        all_crates, key=lambda c: first_valid.get(c, 0), reverse=True
    )

    # Header
    print(f'{"Crate":<20}', end='')
    for i, name in enumerate(names):
        print(f'{name:>{col_w}}', end='')
        if i > 0:
            print(f'{"Δ prev":>10}', end='')
    print()
    print('─' * (20 + len(names) * col_w + (len(names) - 1) * 10))

    for crate in sorted_crates[:12]:
        print(f'{crate:<20}', end='')
        prev_pct = None
        for i, (pd, total) in enumerate(profile_data):
            if total == 0:
                print(f'{DIM}{"—":>{col_w}}{RESET}', end='')
                if i > 0:
                    print(' ' * 10, end='')
            else:
                pct = pd.get(crate, 0)
                print(f'{pct:>{col_w - 1}.1f}%', end='')
                if i > 0 and prev_pct is not None and prev_pct > 0.5:
                    delta = pct - prev_pct
                    sign = '+' if delta >= 0 else ''
                    color = GREEN if delta <= 0 else RED
                    s = f' {color}{sign}{delta:.1f}pp{RESET}'
                    print(s, end='')
                    pad = 10 - len(f' {sign}{delta:.1f}pp') - 1
                    print(' ' * max(0, pad), end='')
                elif i > 0:
                    print(' ' * 10, end='')
                prev_pct = pct
                continue
            prev_pct = None
        print()

    # Sample counts
    print()
    print(f'{"Samples":<20}', end='')
    for _, total in profile_data:
        if total > 0:
            print(f'{total:>{col_w}}', end='')
        else:
            print(f'{DIM}{"—":>{col_w}}{RESET}', end='')
    print()


def main():
    if len(sys.argv) < 3:
        print(f'Usage: {sys.argv[0]} <phase_dir> <phase_dir> [...]')
        print(f'       {sys.argv[0]} reports/*/')
        print()
        print('Each phase directory should contain at least one of:')
        print(f'  {PERF_FILE:<24} — testbed --perf-report output')
        print(f'  {PROFILE_FILE:<24} — samply profile')
        sys.exit(1)

    # Sort directories for consistent ordering
    dirs = sorted(sys.argv[1:])

    reports = [load_phase(d) for d in dirs]
    has_perf = any(r['_has_perf'] for r in reports)
    has_profiles = any(r['_has_profile'] for r in reports)

    print(f'{BOLD}=== Performance Report Comparison ==={RESET}')
    print(f'{DIM}{len(reports)} phases{RESET}')

    if has_perf:
        print_comparison_table(reports)

    if has_profiles:
        profile_data = []
        for r in reports:
            if r['_has_profile']:
                pd, total = load_profile_crate_breakdown(r['_dir'])
                profile_data.append((pd, total))
            else:
                profile_data.append(({}, 0))

        if any(total > 0 for _, total in profile_data):
            print_profile_comparison(reports, profile_data)

    print()


if __name__ == '__main__':
    main()
