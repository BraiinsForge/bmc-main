#!/usr/bin/env python3
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

"""Condense nextest's `libtest-json-plus` stream to a line per test binary.

nextest's own reporter is a line per test or nothing, and a full run is 1 300 of them.
That's enough for a coding agent's transcript to truncate, losing the failure.
With `--message-format libtest-json-plus` nextest emits one JSON event per test instead
and this script is the reporter: every passing test still shows by name, grouped by module
on its binary's line, and a failure prints in full with its captured output.

The exit status follows the tests; a build that never ran them is nextest's
own non-zero status, which the recipe's `pipefail` carries through.

Stdlib only, read from stdin:

    cargo nextest run … --message-format libtest-json-plus | scripts/nextest_report.py
"""

import json
import sys
from collections import defaultdict
from typing import Iterable

Passed = dict[str, dict[str, list[str]]]
Failure = tuple[str, str, str]


def binary_label(binary_id: str) -> str:
    """`crate::binary` as nextest names it, shortened to the crate for its lib."""
    crate, _, binary = binary_id.partition('::')
    return crate if binary == crate.replace('-', '_') else binary_id


def module_label(module: str) -> str:
    """The module a test lives in, without the `tests` module it sits in."""
    if module == 'tests':
        return '(root)'
    return module.removesuffix('::tests') or '(root)'


def render(events: Iterable[str]) -> tuple[str, int]:
    """The report for a stream of JSON lines, and the exit status it implies."""
    passed: Passed = defaultdict(lambda: defaultdict(list))
    failures: list[Failure] = []
    ignored = 0
    for line in events:
        if not line.startswith('{'):
            continue
        event = json.loads(line)
        if event.get('type') != 'test' or event.get('event') == 'started':
            continue
        binary_id, _, test = event['name'].partition('$')
        module, _, name = test.rpartition('::')
        match event['event']:
            case 'ok':
                passed[binary_label(binary_id)][module_label(module)].append(name)
            case 'failed':
                output = event.get('stdout') or event.get('stderr') or ''
                failures.append((binary_label(binary_id), test, output))
            case 'ignored':
                ignored += 1
            case _:
                pass

    lines: list[str] = []
    for binary, modules in passed.items():
        count = sum(len(names) for names in modules.values())
        groups = ' · '.join(
            f'{module}::{{{", ".join(names)}}}' for module, names in modules.items()
        )
        lines.append(f'{binary} {count} passed · {groups}')
    for binary, test, output in failures:
        lines.append(f'FAIL {binary} {test}')
        lines.extend(output.rstrip().splitlines())
    total_passed = sum(
        len(names) for modules in passed.values() for names in modules.values()
    )
    run = total_passed + len(failures)
    summary = f'{run} run · {total_passed} passed · {len(failures)} failed'
    if ignored:
        summary += f' · {ignored} ignored'
    lines.append(summary)
    return '\n'.join(lines), 1 if failures else 0


def main() -> int:
    report, status = render(sys.stdin)
    print(report)
    return status


if __name__ == '__main__':
    sys.exit(main())
