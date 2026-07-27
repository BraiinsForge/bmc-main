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

"""PreToolUse(Bash) hook: prepared `just`/`make` targets run bare, or not at all.

A prepared target wrapped in redirects, pipes or chained commands disguises a trivial
command as something worth blanket-approving, and reads as massaging the output. Long
output is persisted to a file by the harness, so the honest shape is to run the target
alone and then check that file with one plain grep.

Shell-aware rather than regex-based, which is what keeps the false-positive rate down:

- `shlex` collapses a quoted string into one token, so `git commit -m 'run just x'`
  never sees `just` as a command;
- heredoc bodies are dropped before analysis, so a commit message body mentioning a
  target does not trip it;
- a runner only counts in *command position* — first token, or after `|`, `&&`, `;`,
  `(` — so `git log --grep make` is left alone;
- `VAR=value` prefixes stay transparent, so `BMC_PROFILE=x86_64-rr make -C x run`
  is still recognised as a bare run and allowed.

Fail-open: any parse problem allows the call, so the hook can never wedge Bash.
"""

import json
import os
import re
import shlex
import sys

RUNNERS = frozenset({'just', 'make'})

# `shlex(punctuation_chars=True)` emits runs of these as standalone operator tokens.
PUNCTUATION = frozenset('();<>|&')

# Operators that start a fresh command, so the next token is in command position.
# A bare redirect is excluded: what follows it is a filename, not a command.
COMMAND_OPENERS = frozenset('|&;(')

ENV_ASSIGNMENT = re.compile(r'^[A-Za-z_][A-Za-z0-9_]*=')
HEREDOC_START = re.compile(r"<<-?\s*[\"']?([A-Za-z_][A-Za-z0-9_]*)[\"']?")

MESSAGE = """prepared-target-guard: run the {runner} target BARE — the command alone, nothing appended.
No redirects, no pipes, no and/semicolon chains, no trailing grep/tail/head, no echo dressing.
The harness persists long output to a file; afterwards check THAT with ONE plain grep."""


def strip_heredoc_bodies(command: str) -> str:
    """Drop heredoc bodies, keeping the line that opens them.

    The body is data, not commands, so a message mentioning a target must not register.
    """
    lines = command.splitlines()
    kept: list[str] = []
    index = 0
    while index < len(lines):
        line = lines[index]
        kept.append(line)
        index += 1
        match = HEREDOC_START.search(line)
        if not match:
            continue
        delimiter = match.group(1)
        while index < len(lines) and lines[index].strip() != delimiter:
            index += 1
        index += 1  # the delimiter line itself
    return '\n'.join(kept)


def is_operator(token: str) -> bool:
    return bool(token) and all(char in PUNCTUATION for char in token)


def offending_runner(line: str) -> str | None:
    """The runner this line invokes with scaffolding around it, if any."""
    lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    tokens = list(lexer)  # raises ValueError on unbalanced quotes

    scaffolded = any(is_operator(token) for token in tokens) or '`' in line
    if not scaffolded:
        return None

    at_command_position = True
    for token in tokens:
        if is_operator(token):
            at_command_position = any(char in COMMAND_OPENERS for char in token)
            continue
        if at_command_position and ENV_ASSIGNMENT.match(token):
            continue
        if at_command_position and os.path.basename(token) in RUNNERS:
            return os.path.basename(token)
        at_command_position = False
    return None


def main() -> int:
    try:
        command = json.load(sys.stdin).get('tool_input', {}).get('command', '')
    except (json.JSONDecodeError, AttributeError, ValueError):
        return 0
    if not command:
        return 0

    for line in strip_heredoc_bodies(command).splitlines():
        try:
            runner = offending_runner(line)
        except ValueError:
            continue
        if runner:
            print(MESSAGE.format(runner=runner), file=sys.stderr)
            return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
