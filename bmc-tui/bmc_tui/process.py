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

import subprocess
from collections.abc import Callable, Mapping
from typing import IO


def stream_lines(
    argv: list[str], on_line: Callable[[str], None], *, env: Mapping[str, str] | None = None
) -> int:
    """Run `argv`, handing `on_line` each output line as it arrives.
    Stderr is merged in, so progress and errors keep their relative order.
    A nonzero exit is returned, not raised."""
    with subprocess.Popen(
        argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", env=env
    ) as proc:
        stdout: IO[str] | None = proc.stdout
        if stdout is None:
            msg = "BUG: Popen(stdout=PIPE) produced no stdout"
            raise RuntimeError(msg)
        for line in stdout:
            on_line(line.rstrip("\n"))
    return proc.returncode
