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
Shared utilities for WASM runtime development tools.
"""

import json
import re
import shutil
import subprocess
import sys
from collections import Counter
from pathlib import Path

from widget_root import find as find_widget

WASM_TARGET = 'wasm32-unknown-unknown'

# ── ANSI helpers ─────────────────────────────────────────────────────

IS_TTY = sys.stderr.isatty()

DIM = '\033[2m' if IS_TTY else ''
BOLD = '\033[1m' if IS_TTY else ''
RESET = '\033[0m' if IS_TTY else ''
GREEN = '\033[32m' if IS_TTY else ''
RED = '\033[31m' if IS_TTY else ''

# alignment column for right-side timings
COL_WIDTH = 50


def section(title: str) -> None:
    """Print a section header with a ruled line."""
    pad = max(0, COL_WIDTH - len(title) - 1)
    print(f'{BOLD}{title}{RESET} {DIM}{"─" * pad}{RESET}', file=sys.stderr)


def progress(msg: str) -> None:
    """Print a transient status line (overwritten by the next call)."""
    if IS_TTY:
        print(f'\r\033[K{DIM}{msg}{RESET}', end='', file=sys.stderr, flush=True)
    else:
        print(msg, file=sys.stderr)


def clear_progress() -> None:
    """Clear the transient status line."""
    if IS_TTY:
        print('\r\033[K', end='', file=sys.stderr, flush=True)


def format_time(seconds: float) -> str:
    """Format seconds into a human-readable string."""
    if seconds < 60:
        return f'{seconds:.1f}s'
    minutes = int(seconds // 60)
    secs = seconds % 60
    return f'{minutes}m {secs:.0f}s'


def require_tools(*tools: tuple[str, str]) -> None:
    """Exit if any required CLI tool is missing. Each item is (binary, package)."""
    for binary, package in tools:
        if not shutil.which(binary):
            print(
                f'Error: {binary} not found. Install package: {package}',
                file=sys.stderr,
            )
            sys.exit(1)


def build_example_wasm(
    example: str, profile: str = 'release', features: tuple[str, ...] = ()
) -> Path:
    """Build a widget and return the .wasm path.

    `profile` selects the cargo profile (e.g. 'profiling' keeps the wasm name
    section). `features` are extra cargo features, including dep features like
    'bmc-wasm-sdk/profiling'. The widget's workspace root is resolved via
    `widget_root.py`; works for SDK examples and production widgets alike.
    """

    workspace_dir, dir_name = find_widget(example)
    wasm_name = dir_name.replace('-', '_')

    feature_args = [arg for f in features for arg in ('--features', f)]
    # --message-format=json emits one compiler-artifact message per built
    # target; parse it so we get the exact .wasm path even with a custom
    # CARGO_TARGET_DIR.
    result = subprocess.run(
        [
            'cargo',
            'build',
            '--manifest-path',
            str(workspace_dir / dir_name / 'Cargo.toml'),
            '--profile',
            profile,
            '--target',
            WASM_TARGET,
            *feature_args,
            '--message-format=json-render-diagnostics',
        ],
        cwd=workspace_dir,
        check=True,
        # Capture only stdout (the JSON artifact stream we parse); let stderr
        # inherit so cargo's `Compiling …` progress is visible during the build.
        stdout=subprocess.PIPE,
        text=True,
    )

    for line in result.stdout.splitlines():
        msg = json.loads(line)
        if msg.get('reason') != 'compiler-artifact':
            continue
        if msg['target']['name'] != wasm_name:
            continue
        for path in msg.get('filenames') or []:
            if path.endswith('.wasm'):
                return Path(path)

    raise RuntimeError(f"no .wasm artifact found for widget '{example}'")


def extract_crate(sym: str) -> str:
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


def find_testbed_thread(data: dict) -> dict | None:
    """Find the main testbed thread (largest sample count)."""
    testbed_threads = [
        t
        for t in data['threads']
        if t['name'] == 'testbed' and t['samples']['length'] > 0
    ]
    if not testbed_threads:
        return None
    return max(testbed_threads, key=lambda t: t['samples']['length'])


def crate_breakdown_from_thread(
    thread: dict,
    symbols: dict[str, str],
) -> tuple[Counter[str], int]:
    """Compute inclusive crate-level breakdown from a samply thread.

    Returns (crate_counter, total_samples).
    The counter maps crate name to sample count, deduplicated per-stack
    (a crate appearing multiple times in one stack counts once).
    """
    strings: list[str] = thread['stringArray']
    func_table: dict = thread['funcTable']
    frame_table: dict = thread['frameTable']
    stack_table: dict = thread['stackTable']
    samples: dict = thread['samples']

    stack_counts: Counter[int | None] = Counter(samples['stack'])
    prefixes: list[int | None] = stack_table['prefix']
    frame_list: list[int] = stack_table['frame']

    total = sum(stack_counts.values())

    crate_time: Counter[str] = Counter()
    for stack_idx, count in stack_counts.items():
        if stack_idx is None:
            continue
        seen_crates: set[str] = set()
        s: int | None = stack_idx
        while s is not None:
            fi = frame_list[s]
            func_idx = frame_table['func'][fi]
            sym = strings[func_table['name'][func_idx]]
            crate = extract_crate(symbols[sym] if sym in symbols else sym)
            if crate not in seen_crates:
                crate_time[crate] += count
                seen_crates.add(crate)
            s = prefixes[s]

    return crate_time, total
