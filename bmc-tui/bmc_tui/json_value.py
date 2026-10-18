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

"""JSON whose shape the tools do not model.

A document with a known shape decodes into a TypedDict through `msgspec.json.decode`.
What is left — device config read tolerantly, or an envelope that varies per call —
is a `Json` value, narrowed with `isinstance` where it is used.
msgspec cannot decode into this recursive alias, so these stay on `json.loads`.
"""

import json
from typing import cast

type Json = dict[str, Json] | list[Json] | str | int | float | bool | None


def loads(text: str | bytes) -> Json:
    """`json.loads`, typed as what JSON can hold rather than `Any`."""
    return cast("Json", json.loads(text))
