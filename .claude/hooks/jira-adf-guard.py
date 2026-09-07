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

"""PreToolUse(Bash) hook: a Jira description arrives as ADF, never as markdown.

Jira Cloud stores rich text as Atlassian Document Format. Markdown handed to a
description field is stored *literally* — the reader sees `## Goal` and `**bold**`
as characters. Nothing errors, so the mistake only surfaces when someone opens the
issue, by which point the body is already published.

`--description`/`-d` and `--from-file`/`-f` take that text inline. `--description-file`
is the one that carries a converted ADF document, so it is the only form allowed here.

Fail-open: any parse problem allows the call, so the hook can never wedge Bash.
"""

import json
import shlex
import sys

# Inline-text flags. `--description-file` is deliberately absent: it is the escape.
PLAIN_TEXT_FLAGS = frozenset({'-d', '--description', '-f', '--from-file'})

SUBCOMMANDS = frozenset({'create', 'edit'})

MESSAGE = """jira-adf-guard: {flag} takes plain text, and Jira renders markdown literally — readers see `##` and `**`.
Convert first, then pass the document:
  just md-to-adf <ticket.md> --skip-title > .tmp/jira-adf-<key>.json
  acli jira workitem {subcommand} … --description-file .tmp/jira-adf-<key>.json
Only --description-file carries ADF. Plain prose with no markup is still safer sent this way."""


def offending_flag(tokens: list[str]) -> tuple[str, str] | None:
    """The inline-text flag this `acli jira workitem create|edit` passes, if any."""
    for index, token in enumerate(tokens):
        if token != 'acli':
            continue
        rest = tokens[index + 1 :]
        if (
            rest[:2] != ['jira', 'workitem']
            or len(rest) < 3
            or rest[2] not in SUBCOMMANDS
        ):
            continue
        subcommand = rest[2]
        for flag in rest[3:]:
            name = flag.split('=', 1)[0]
            if name in PLAIN_TEXT_FLAGS:
                return name, subcommand
    return None


def main() -> int:
    try:
        command = json.load(sys.stdin).get('tool_input', {}).get('command', '')
    except (json.JSONDecodeError, AttributeError, ValueError):
        return 0
    if not command:
        return 0

    try:
        tokens = shlex.split(command)
    except ValueError:
        return 0

    found = offending_flag(tokens)
    if found:
        flag, subcommand = found
        print(MESSAGE.format(flag=flag, subcommand=subcommand), file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
