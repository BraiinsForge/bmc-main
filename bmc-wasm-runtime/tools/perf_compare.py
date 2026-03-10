#!/usr/bin/env nix
#!nix shell nixpkgs#python312 nixpkgs#odiff
#!nix --command python3

"""
Compare perf reports and profiles across optimization phases.

Each phase is a directory containing (all optional, at least one required):
  perf.json          — testbed --perf-report output
  profile.json.gz    — samply profile
  symbols.json       — symbolication map from perf_symbolicate.py

Usage:
    # Compare all phases:
    ./perf_compare.py reports/00-baseline reports/01-render-loop-fix reports/02-cached-tree

    # Or with glob:
    ./perf_compare.py reports/*/
"""

import gzip
import json
import sys
from collections.abc import Callable
from pathlib import Path

from _common import crate_breakdown_from_thread, find_testbed_thread, load_symbols

# ANSI colors
RED = '\033[31m'
GREEN = '\033[32m'
BOLD = '\033[1m'
DIM = '\033[2m'
RESET = '\033[0m'

PERF_FILE = 'perf.json'
PROFILE_FILE = 'profile.json.gz'

METRICS: list[tuple[str, str]] = [
    ('avg_frame_us', 'Frame'),
    ('avg_wasm_us', 'WASM'),
    ('avg_tree_us', 'Tree'),
    ('avg_layout_us', 'Layout'),
    ('avg_render_us', 'Render'),
    ('avg_flush_us', 'Flush'),
]

PERCENTILES: list[tuple[str, str]] = [
    ('p50_frame_us', 'p50'),
    ('p95_frame_us', 'p95'),
    ('p99_frame_us', 'p99'),
]


def fmt_us(us: float) -> str:
    """Format microseconds as ms with 1 decimal."""
    return f'{us / 1_000:.1f}ms'


def phase_name(dir_path: str) -> str:
    """Extract human-readable phase name from directory name."""
    name = Path(dir_path).name
    return name.replace('-', ' ', 1).replace('-', ' ')


def load_phase(dir_path: str) -> dict:
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

    report: dict = {}
    if perf.exists():
        with open(perf) as f:
            report = json.load(f)

    report['_dir'] = str(d)
    report['_has_perf'] = perf.exists()
    report['_has_profile'] = profile.exists()

    return report


def print_comparison_table(reports: list[dict]) -> None:
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

    def print_row(
        row_label: str,
        get_val: Callable[[dict], float],
        fmt_val: Callable[[float], str],
        *,
        better_lower: bool = True,
    ) -> None:
        print(f'{row_label:<12}', end='')
        prev_val: float | None = None
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


def load_profile_crate_breakdown(phase_dir: str) -> tuple[dict[str, float], int]:
    """Load a samply profile and return crate-level inclusive breakdown."""
    profile_path = Path(phase_dir) / PROFILE_FILE

    with gzip.open(profile_path, 'rt') as f:
        data: dict = json.load(f)

    symbols = load_symbols(profile_path)

    thread = find_testbed_thread(data)
    if thread is None:
        return {}, 0

    crate_time, total = crate_breakdown_from_thread(thread, symbols)

    return {
        crate: 100.0 * count / total for crate, count in crate_time.most_common(15)
    }, total


def print_profile_comparison(
    reports: list[dict],
    profile_data: list[tuple[dict[str, float], int]],
) -> None:
    """Print crate-level profile comparison."""
    names = [phase_name(r['_dir']) for r in reports]
    col_w = max(14, max(len(n) for n in names) + 2)

    print(f'\n{BOLD}=== Profile: Crate Breakdown (inclusive %) ==={RESET}\n')

    # Gather all crate names from phases that have profiles
    all_crates: set[str] = set()
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
        prev_pct: float | None = None
        for i, (pd, total) in enumerate(profile_data):
            if total == 0:
                print(f'{DIM}{"—":>{col_w}}{RESET}', end='')
                if i > 0:
                    print(' ' * 10, end='')
                prev_pct = None
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


def main() -> None:
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
        profile_data: list[tuple[dict[str, float], int]] = []
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
