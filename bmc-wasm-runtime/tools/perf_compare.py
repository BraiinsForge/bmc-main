#!/usr/bin/env nix
#!nix shell nixpkgs#python312 nixpkgs#odiff
#!nix --command python3

"""
Compare combined performance profiles across optimization phases.

Each phase is a report directory holding `combined.json.gz` (produced by
perf_record.py / perf_finalize.py): a Firefox-format profile carrying the CPU
samples plus a counter per frame-timing phase and profiling section. Everything
compared here is read from that one file; perf.json / symbols.json stay in the
dir as provenance but are not consumed.

Usage:
    ./perf_compare.py reports/04-baseline reports/05-cached
    ./perf_compare.py reports/*/
"""

import gzip
import json
import sys
from collections.abc import Callable
from pathlib import Path

from _common import crate_breakdown_from_thread, find_testbed_thread

RED = '\033[31m'
GREEN = '\033[32m'
BOLD = '\033[1m'
DIM = '\033[2m'
RESET = '\033[0m'

COMBINED_FILE = 'combined.json.gz'

# Timing rows (µs), in display order. `Tree` is the tree-deserialize phase.
TIMING_METRICS: list[tuple[str, str]] = [
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
    return f'{us / 1_000:.1f}ms'


def fmt_fuel(f: float) -> str:
    """Fuel (instruction count) as thousands; the raw numbers are large."""
    return f'{f / 1_000:.1f}k'


def phase_name(dir_path: str) -> str:
    name = Path(dir_path).name
    return name.replace('-', ' ', 1).replace('-', ' ')


def _avg(a: list[float]) -> float:
    return sum(a) / len(a) if a else 0.0


def _pct(a: list[float], p: float) -> float:
    if not a:
        return 0.0
    s = sorted(a)
    return s[min(len(s) - 1, int(len(s) * p))]


def load_phase(dir_path: str) -> dict:
    """Read a phase's combined profile and derive its comparison metrics."""
    d = Path(dir_path)
    combined = d / COMBINED_FILE
    if not combined.exists():
        print(f'{RED}Error: {d}/{COMBINED_FILE} not found{RESET}', file=sys.stderr)
        sys.exit(1)

    with gzip.open(combined, 'rt') as f:
        profile = json.load(f)

    # Split counters by category: timing phases (µs) vs profiling sections (fuel).
    timing: dict[str, list[float]] = {}
    fuel: dict[str, list[float]] = {}
    for c in profile.get('counters', []):
        target = timing if c['category'].startswith('Frame timing') else fuel
        target[c['name']] = c['samples']['count']

    n = max((len(v) for v in timing.values()), default=0)
    frame_us = [sum(v[i] for v in timing.values() if i < len(v)) for i in range(n)]

    # FPS from the profile's own sample time span — no separate timing source.
    fps = 0.0
    thread = find_testbed_thread(profile)
    if thread is not None and n:
        t = thread['samples']['time']
        span_s = (t[-1] - t[0]) / 1_000 if len(t) > 1 else 0
        fps = n / span_s if span_s else 0.0

    return {
        '_dir': str(d),
        '_has_data': n > 0,
        '_profile': profile,
        '_fuel': {name: _avg(vals) for name, vals in fuel.items()},
        'avg_fps': fps,
        'avg_frame_us': _avg(frame_us),
        'avg_wasm_us': _avg(timing.get('wasm_us', [])),
        'avg_tree_us': _avg(timing.get('deserialize_us', [])),
        'avg_layout_us': _avg(timing.get('layout_us', [])),
        'avg_render_us': _avg(timing.get('render_us', [])),
        'avg_flush_us': _avg(timing.get('flush_us', [])),
        'p50_frame_us': _pct(frame_us, 0.50),
        'p95_frame_us': _pct(frame_us, 0.95),
        'p99_frame_us': _pct(frame_us, 0.99),
    }


def _print_delta_row(
    reports: list[dict],
    col_w: int,
    label: str,
    get_val: Callable[[dict], float | None],
    fmt_val: Callable[[float], str],
    *,
    label_w: int,
    better_lower: bool = True,
) -> None:
    """One comparison row: value per phase plus a coloured Δ vs the previous."""
    ansi_pad = len(GREEN) + len(RESET)
    print(f'{label:<{label_w}}', end='')
    prev: float | None = None
    for i, r in enumerate(reports):
        val = get_val(r)
        if val is None:
            print(f'{DIM}{"—":>{col_w}}{RESET}', end='')
            if i > 0:
                print(' ' * 10, end='')
            prev = None
            continue
        print(f'{fmt_val(val):>{col_w}}', end='')
        if i > 0 and prev:
            delta = val - prev
            pct = 100.0 * delta / prev if prev else 0
            sign = '+' if delta >= 0 else ''
            improved = delta <= 0 if better_lower else delta >= 0
            color = GREEN if improved else RED
            print(f'{color}{sign}{pct:.1f}%{RESET}'.rjust(10 + ansi_pad), end='')
        elif i > 0:
            print(' ' * 10, end='')
        prev = val
    print()


def _print_header(reports: list[dict], col_w: int, label: str, label_w: int) -> None:
    names = [phase_name(r['_dir']) for r in reports]
    print(f'{BOLD}{label:<{label_w}}', end='')
    for i, name in enumerate(names):
        print(f'{name:>{col_w}}', end='')
        if i > 0:
            print(f'{"Δ prev":>10}', end='')
    print(RESET)
    print('─' * (label_w + len(reports) * col_w + (len(reports) - 1) * 10))


def _col_w(reports: list[dict]) -> int:
    return max(14, max(len(phase_name(r['_dir'])) for r in reports) + 2)


def print_timing_comparison(reports: list[dict]) -> None:
    col_w = _col_w(reports)

    def val(r: dict, key: str) -> float | None:
        return r.get(key) if r['_has_data'] else None

    print('\n', end='')
    _print_header(reports, col_w, 'Metric', label_w=12)
    _print_delta_row(
        reports,
        col_w,
        'FPS',
        lambda r: val(r, 'avg_fps'),
        lambda v: f'{v:.1f}',
        label_w=12,
        better_lower=False,
    )
    for key, label in TIMING_METRICS:
        _print_delta_row(
            reports, col_w, label, lambda r, k=key: val(r, k), fmt_us, label_w=12
        )
    print()
    for key, label in PERCENTILES:
        _print_delta_row(
            reports, col_w, label, lambda r, k=key: val(r, k), fmt_us, label_w=12
        )


def print_fuel_comparison(reports: list[dict]) -> None:
    sections: set[str] = set()
    for r in reports:
        sections.update(r['_fuel'].keys())
    if not sections:
        return
    col_w = _col_w(reports)
    print(f'\n{BOLD}=== Fuel: per-frame instruction cost (k = ×1000) ==={RESET}\n')
    _print_header(reports, col_w, 'Section', label_w=20)
    for section in sorted(sections):
        _print_delta_row(
            reports,
            col_w,
            section,
            lambda r, s=section: r['_fuel'].get(s),
            fmt_fuel,
            label_w=20,
        )


def print_profile_comparison(reports: list[dict]) -> None:
    """Crate-level inclusive breakdown from each phase's combined profile."""
    breakdowns: list[tuple[dict[str, float], int]] = []
    for r in reports:
        thread = find_testbed_thread(r['_profile'])
        if thread is None:
            breakdowns.append(({}, 0))
            continue
        # Inline-symbolized funcTable: no symbols.json overlay needed.
        crate_time, total = crate_breakdown_from_thread(thread, {})
        breakdowns.append(
            (
                {c: 100.0 * n / total for c, n in crate_time.most_common(15)},
                total,
            )
        )
    if not any(total > 0 for _, total in breakdowns):
        return

    names = [phase_name(r['_dir']) for r in reports]
    col_w = _col_w(reports)
    print(f'\n{BOLD}=== Profile: Crate Breakdown (inclusive %) ==={RESET}\n')

    all_crates: set[str] = set()
    for pd, total in breakdowns:
        if total > 0:
            all_crates.update(pd.keys())
    first_valid = next((pd for pd, total in breakdowns if total > 0), {})
    sorted_crates = sorted(
        all_crates, key=lambda c: first_valid.get(c, 0), reverse=True
    )

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
        for i, (pd, total) in enumerate(breakdowns):
            if total == 0:
                print(f'{DIM}{"—":>{col_w}}{RESET}', end='')
                if i > 0:
                    print(' ' * 10, end='')
                prev_pct = None
                continue
            pct = pd.get(crate, 0)
            print(f'{pct:>{col_w - 1}.1f}%', end='')
            if i > 0 and prev_pct is not None and prev_pct > 0.5:
                delta = pct - prev_pct
                sign = '+' if delta >= 0 else ''
                color = GREEN if delta <= 0 else RED
                print(f' {color}{sign}{delta:.1f}pp{RESET}', end='')
                pad = 10 - len(f' {sign}{delta:.1f}pp') - 1
                print(' ' * max(0, pad), end='')
            elif i > 0:
                print(' ' * 10, end='')
            prev_pct = pct
        print()

    print()
    print(f'{"Samples":<20}', end='')
    for _, total in breakdowns:
        cell = f'{total}' if total > 0 else '—'
        print(f'{cell:>{col_w}}', end='')
    print()


def main() -> None:
    if len(sys.argv) < 3:
        print(f'Usage: {sys.argv[0]} <phase_dir> <phase_dir> [...]')
        print(f'       {sys.argv[0]} reports/*/')
        print()
        print(f'Each phase directory must contain {COMBINED_FILE}.')
        sys.exit(1)

    dirs = sorted(sys.argv[1:])
    reports = [load_phase(d) for d in dirs]

    print(f'{BOLD}=== Performance Report Comparison ==={RESET}')
    print(f'{DIM}{len(reports)} phases{RESET}')

    print_timing_comparison(reports)
    print_fuel_comparison(reports)
    print_profile_comparison(reports)
    print()


if __name__ == '__main__':
    main()
