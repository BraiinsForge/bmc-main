#!/usr/bin/env -S uv run
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
Record a full performance profile for a WASM widget.

Collects samply CPU profile, symbolication, and internal frame timing
into a single report directory under reports/.

Usage:
    ./tools/perf_record.py                          # reports/00-latest/, hello-widget
    ./tools/perf_record.py -e iss-position          # different widget
    ./tools/perf_record.py -r 05-my-change          # named report
    ./tools/perf_record.py -r 05-my-change -n 1200  # 1200 frames
"""

import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Annotated

import tyro
from tyro.conf import arg

from _common import build_example_wasm, cargo_messages, require_tools

REPORTS_DIR = Path('reports')


def check_prerequisites() -> None:
    require_tools(('samply', 'cargo install samply'))

    paranoid = Path('/proc/sys/kernel/perf_event_paranoid')
    if paranoid.exists():
        val = paranoid.read_text().strip()
        if val != '-1':
            print(
                f'Error: perf_event_paranoid={val} (expected -1).\n'
                "Run: echo '-1' | sudo tee /proc/sys/kernel/perf_event_paranoid",
                file=sys.stderr,
            )
            sys.exit(1)


def prepare_report_dir(report_dir: Path) -> None:
    if report_dir.is_dir():
        ans = input(f'Directory {report_dir} already exists. Remove it? [y/N] ')
        if not ans.lower().startswith('y'):
            print('Aborted.')
            sys.exit(1)
        shutil.rmtree(report_dir)
    report_dir.mkdir(parents=True)


def build(example: str) -> tuple[Path, Path]:
    """Build wasm widget + testbed and return (wasm_file, testbed_binary)."""
    print(f'Building {example} wasm...')
    # 'profiling' profile keeps the wasm name section; the SDK profiling feature
    # turns on `profile::span`/`report` (fuel-based section attribution).
    wasm_file = build_example_wasm(
        example, profile='profiling', features=('bmc-wasm-sdk/profiling',)
    )

    print('Building testbed for profiling...')
    result = subprocess.run(
        [
            'cargo',
            'build',
            '--profile',
            'profiling',
            '--features',
            'testbed',
            '--bin',
            'testbed',
            '--message-format=json-render-diagnostics',
        ],
        check=True,
        # Capture only stdout (parsed for the testbed artifact path); stderr
        # inherits so the build's `Compiling …` progress is visible.
        stdout=subprocess.PIPE,
        text=True,
    )

    for msg in cargo_messages(result.stdout):
        target = msg.get('target')
        if (
            msg['reason'] != 'compiler-artifact'
            or target is None
            or target['name'] != 'testbed'
        ):
            continue
        executable = msg.get('executable')
        if executable:
            return wasm_file, Path(executable)

    raise RuntimeError('no testbed executable found in cargo build output')


def record(
    wasm_file: Path,
    testbed_binary: Path,
    report_dir: Path,
    perf_frames: int,
    extra_args: list[str],
) -> None:
    profile_gz = report_dir / 'profile.json.gz'
    perf_json = report_dir / 'perf.json'

    cmd = [
        'samply',
        'record',
        '--save-only',
        '-o',
        str(profile_gz),
        str(testbed_binary),
        str(wasm_file),
        f'--perf-report={perf_json}',
        f'--perf-frames={perf_frames}',
        *extra_args,
    ]
    subprocess.run(cmd, check=True)

    print('Symbolicating profile...')
    subprocess.run(
        [
            sys.executable,
            'tools/perf_symbolicate.py',
            str(profile_gz),
            str(testbed_binary),
        ],
        check=True,
    )

    print('Combining into final profile...')
    subprocess.run(
        [sys.executable, 'tools/perf_finalize.py', str(report_dir)],
        check=True,
    )


def print_summary(report_dir: Path) -> None:
    print(f'\nSaved to {report_dir}/:')
    for f in sorted(report_dir.iterdir()):
        size_kb = f.stat().st_size / 1024
        print(f'  {f.name:30s} {size_kb:8.1f} KB')

    # Absolute paths so the hints work regardless of the caller's cwd.
    combined = (report_dir / 'combined.json.gz').resolve()
    print(f'\nView:    samply load {combined}')
    print(f'Analyze: uv run tools/perf_analyze.py {combined}')
    print('Compare: uv run tools/perf_compare.py reports/*/')


@dataclass(frozen=True)
class Options:
    """Record a WASM widget performance profile; unknown arguments reach the testbed."""

    example: Annotated[str, arg(aliases=('-e',))] = (
        'hello-widget'  # widget example name
    )
    report: Annotated[str, arg(aliases=('-r',))] = (
        '00-latest'  # report directory under reports/
    )
    perf_frames: Annotated[int, arg(aliases=('-n',))] = (
        600  # number of frames to capture
    )


def main() -> None:
    # tyro's overloads take a `TypeForm[OutT]`, which basedpyright leaves unsolved.
    options, extra = tyro.cli(Options, return_unknown_args=True)  # pyright: ignore[reportAny]

    check_prerequisites()
    report_dir = REPORTS_DIR / options.report
    prepare_report_dir(report_dir)
    wasm_file, testbed_binary = build(options.example)
    record(wasm_file, testbed_binary, report_dir, options.perf_frames, extra)
    print_summary(report_dir)


if __name__ == '__main__':
    main()
