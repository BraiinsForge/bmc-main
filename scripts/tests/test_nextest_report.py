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

"""The shapes nextest's `libtest-json-plus` stream takes, and what each reads as."""

import importlib.util
import json
import sys
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    'nextest_report', Path(__file__).resolve().parents[1] / 'nextest_report.py'
)
assert _SPEC is not None and _SPEC.loader is not None, (
    'BUG: nextest_report.py must be importable'
)
nextest_report = importlib.util.module_from_spec(_SPEC)
sys.modules['nextest_report'] = nextest_report
_SPEC.loader.exec_module(nextest_report)


def event(kind: str, name: str, **fields: object) -> str:
    return json.dumps({'type': 'test', 'event': kind, 'name': name, **fields})


SUITE = json.dumps(
    {
        'type': 'suite',
        'event': 'started',
        'test_count': 3,
        'nextest': {'crate': 'miner-info', 'test_binary': 'miner_info', 'kind': 'lib'},
    }
)


def test_passes_group_by_module_on_the_binary_line() -> None:
    report, status = nextest_report.render(
        [
            SUITE,
            event('started', 'miner-info::miner_info$api::tests::parses_block_height'),
            event('ok', 'miner-info::miner_info$api::tests::parses_block_height'),
            event('ok', 'miner-info::miner_info$api::tests::sums_chip_count'),
            event('ok', 'miner-info::miner_info$face::bmm101::tests::the_grid_spreads'),
            json.dumps({'type': 'suite', 'event': 'ok', 'passed': 3, 'failed': 0}),
        ]
    )
    assert status == 0
    assert report.splitlines() == [
        'miner-info 3 passed · api::{parses_block_height, sums_chip_count}'
        ' · face::bmm101::{the_grid_spreads}',
        '3 run · 3 passed · 0 failed',
    ]


def test_a_failure_prints_in_full_and_fails_the_run() -> None:
    report, status = nextest_report.render(
        [
            event('ok', 'miner-info::miner_info$api::tests::parses_block_height'),
            event(
                'failed',
                'miner-info::miner_info$face::bmm101::tests::the_grid_spreads',
                stdout='assertion `left == right` failed\n  left: 1\n right: 2\n',
            ),
        ]
    )
    assert status == 1
    assert report.splitlines() == [
        'miner-info 1 passed · api::{parses_block_height}',
        'FAIL miner-info face::bmm101::tests::the_grid_spreads',
        'assertion `left == right` failed',
        '  left: 1',
        ' right: 2',
        '2 run · 1 passed · 1 failed',
    ]


def test_an_integration_binary_keeps_its_name() -> None:
    report, _ = nextest_report.render(
        [event('ok', 'bmc-wasm-runtime::asset_registration$tests::restores_after_wake')]
    )
    assert report.splitlines()[0] == (
        'bmc-wasm-runtime::asset_registration 1 passed · (root)::{restores_after_wake}'
    )


def test_ignored_tests_count_in_the_summary_only() -> None:
    report, status = nextest_report.render(
        [
            event('ok', 'c::c$tests::a'),
            event('ignored', 'c::c$tests::b'),
        ]
    )
    assert status == 0
    assert report.splitlines()[-1] == '1 run · 1 passed · 0 failed · 1 ignored'


def test_lines_that_are_not_json_are_passed_over() -> None:
    report, status = nextest_report.render(['   Compiling miner-info v0.1.0', ''])
    assert (report, status) == ('0 run · 0 passed · 0 failed', 0)
