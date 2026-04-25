#!/usr/bin/env nix
#!nix shell nixpkgs#python312
#!nix --command python3

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

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

from _common import build_example_wasm, require_tools

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
    wasm_file = build_example_wasm(example)

    print('Building testbed (profiling profile)...')
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
        capture_output=True,
        text=True,
    )

    for line in result.stdout.splitlines():
        msg = json.loads(line)
        if msg.get('reason') != 'compiler-artifact':
            continue
        if msg['target']['name'] != 'testbed':
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


def print_summary(report_dir: Path) -> None:
    print(f'\nSaved to {report_dir}/:')
    for f in sorted(report_dir.iterdir()):
        size_kb = f.stat().st_size / 1024
        print(f'  {f.name:30s} {size_kb:8.1f} KB')

    print(f'\nView:    samply load {report_dir}/profile.json.gz')
    print(
        f'Analyze: uv run --python 3.12 tools/perf_analyze.py {report_dir}/profile.json.gz'
    )
    print('Compare: uv run --python 3.12 tools/perf_compare.py reports/*/')


def main() -> None:
    parser = argparse.ArgumentParser(
        description='Record a WASM widget performance profile.'
    )
    parser.add_argument(
        '-e', '--example', default='hello-widget', help='widget example name'
    )
    parser.add_argument(
        '-r',
        '--report',
        default='00-latest',
        help='report directory name (under reports/)',
    )
    parser.add_argument(
        '-n', '--perf-frames', type=int, default=600, help='number of frames to capture'
    )
    args, extra = parser.parse_known_args()

    check_prerequisites()
    report_dir = REPORTS_DIR / args.report
    prepare_report_dir(report_dir)
    wasm_file, testbed_binary = build(args.example)
    record(wasm_file, testbed_binary, report_dir, args.perf_frames, extra)
    print_summary(report_dir)


if __name__ == '__main__':
    main()
