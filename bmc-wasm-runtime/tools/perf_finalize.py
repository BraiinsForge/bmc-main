#!/usr/bin/env nix
#!nix shell ../..#pkgs.python312
#!nix --command python3
# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.


"""
Combine a report's samply profile, symbols, and frame timings into one file.

Reads `profile.json.gz`, `symbols.json`, and `perf.json` from a report dir and
writes `combined.json.gz` (Firefox Profiler format): the original CPU samples
with the wasm funcTable symbolized in place, plus a counter per frame-timing
phase and profiling section. The source files are left untouched.

Usage:
    ./tools/perf_finalize.py reports/03-extended-format/

Schema references:
  - Firefox Profiler format types (Counter, RawCounterSamplesTable), living spec:
    https://github.com/firefox-devtools/profiler/blob/main/src/types/profile.ts
  - Exact emitted shape mirrored by `make_counter` — fxprof-processed-profile 0.8,
    the writer samply 0.13.1 uses (simpler than profiler `main`: no `display` block):
    https://github.com/mstange/samply/blob/samply-v0.13.1/fxprof-processed-profile/src/counters.rs
"""

import gzip
import json
import sys
from pathlib import Path

from _common import find_testbed_thread


def symbolize_in_place(profile: dict, symbols: dict[str, str]) -> int:
    """Replace hex-address strings in every thread's stringArray with resolved
    symbols. Returns the number of strings rewritten."""
    rewritten = 0
    for thread in profile['threads']:
        strings: list[str] = thread['stringArray']
        for i, s in enumerate(strings):
            resolved = symbols.get(s)
            if resolved is not None and resolved != s:
                strings[i] = resolved
                rewritten += 1
    return rewritten


def frame_times(thread: dict, frames: int) -> list[float]:
    """Distribute `frames` evenly across the thread's sample time range. Frame
    pacing is ~uniform at steady state, so this aligns the counter timeline with
    the CPU samples closely enough for inspection."""
    if frames == 0:
        return []
    samples = thread['samples']
    if 'time' in samples:
        times = samples['time']
    elif 'timeDeltas' in samples:
        # samply's processed format stores per-sample deltas, not absolute
        # timestamps; absolute time is the running sum (delta[0] is the
        # offset from profile start).
        times = []
        acc = 0.0
        for delta in samples['timeDeltas']:
            acc += delta
            times.append(acc)
    else:
        return [0.0] * frames

    if not times:
        return [0.0] * frames

    first, last = times[0], times[-1]
    if frames == 1:
        return [first]
    step = (last - first) / (frames - 1)
    return [first + step * i for i in range(frames)]


def make_counter(
    name: str,
    category: str,
    values: list[int],
    times: list[float],
    pid,
    main_idx: int,
    color: str,
) -> dict:
    """A counter in the exact shape `fxprof-processed-profile` (samply's writer)
    emits. `count` is a per-sample delta the profiler accumulates, so the graph's
    slope is the per-frame cost; `number` is the per-sample operation count."""
    if len(times) != len(values):
        raise ValueError(
            f'counter {name!r}: {len(values)} values but {len(times)} timestamps'
        )
    return {
        'category': category,
        'name': name,
        'description': f'{name} per frame',
        'mainThreadIndex': main_idx,
        'pid': pid,
        'samples': {
            'length': len(values),
            'count': [float(v) for v in values],
            'number': [1] * len(values),
            'time': times,
        },
        'color': color,
    }


def build_counters(profile: dict, perf: dict) -> list[dict]:
    thread = find_testbed_thread(profile)
    if thread is None:
        print('Warning: no testbed thread; skipping counters', file=sys.stderr)
        return []
    main_idx = profile['threads'].index(thread)
    pid = thread['pid']
    per_frame = perf.get('per_frame', {})
    frames = perf.get('frames', 0)
    times = frame_times(thread, frames)

    counters: list[dict] = []
    timing_colors = ('blue', 'teal', 'green', 'purple', 'grey')
    for phase, color in zip(
        ('wasm_us', 'deserialize_us', 'layout_us', 'render_us', 'flush_us'),
        timing_colors,
    ):
        values = per_frame.get(phase)
        if values:
            counters.append(
                make_counter(
                    phase, 'Frame timing (µs)', values, times, pid, main_idx, color
                )
            )
    fuel_colors = ('orange', 'red', 'magenta', 'ink')
    for (name, values), color in zip(
        (per_frame.get('fuel') or {}).items(), fuel_colors
    ):
        counters.append(make_counter(name, 'Fuel', values, times, pid, main_idx, color))
    return counters


def main() -> None:
    if len(sys.argv) != 2:
        print(f'Usage: {sys.argv[0]} <report-dir>', file=sys.stderr)
        sys.exit(1)

    report_dir = Path(sys.argv[1])
    profile_path = report_dir / 'profile.json.gz'
    symbols_path = report_dir / 'symbols.json'
    perf_path = report_dir / 'perf.json'
    out_path = report_dir / 'combined.json.gz'

    for p in (profile_path, symbols_path, perf_path):
        if not p.exists():
            print(f'Error: {p} does not exist', file=sys.stderr)
            sys.exit(1)

    with gzip.open(profile_path, 'rt') as f:
        profile = json.load(f)
    symbols = json.loads(symbols_path.read_text())
    perf = json.loads(perf_path.read_text())

    rewritten = symbolize_in_place(profile, symbols)
    counters = build_counters(profile, perf)
    profile.setdefault('counters', []).extend(counters)

    with gzip.open(out_path, 'wt') as f:
        json.dump(profile, f)

    print(f'Symbolized {rewritten} strings, added {len(counters)} counters.')
    print(f'Wrote {out_path}')


if __name__ == '__main__':
    main()
