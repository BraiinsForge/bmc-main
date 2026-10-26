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

"""Cut a Noto Sans SC face to the code points of the shipped GB2312 subset.

The sources are noto-cjk's Sans 2.004 release, `Sans/SubsetOTF/SC/`, fetched with
`nix store prefetch-file https://github.com/notofonts/noto-cjk/raw/Sans2.004/Sans/SubsetOTF/SC/<file>`:

    NotoSansSC-Regular.otf  sha256-+qbJ32UhFt3nidNRNZ89fl0ihaKyofBKLXJE33BtXqk=
    NotoSansSC-Bold.otf     sha256-xstak6uqntyO50Y7frt/QtYY1A5u0velNxyXsLZHZ8A=

usage: uv run scripts/subset_gb2312.py SOURCE OUTPUT
"""

from pathlib import Path
from typing import Annotated

import tyro

# fontTools ships no type stubs, so every call into it reads as untyped.
from fontTools import subset  # pyright: ignore[reportMissingTypeStubs]
from fontTools.ttLib import TTFont  # pyright: ignore[reportMissingTypeStubs]
from tyro.conf import Positional, arg

SHIPPED = Path(__file__).resolve().parent.parent / 'assets/fonts/NotoSansSC-GB2312.otf'


def main(
    source: Annotated[Positional[Path], arg(metavar='SOURCE')],
    output: Annotated[Positional[Path], arg(metavar='OUTPUT')],
) -> None:
    """Cut a Noto Sans SC face to the code points of assets/fonts/NotoSansSC-GB2312.otf,
    so every weight covers the same characters.
    Layout features, the BASE table and every name record are kept.

    Args:
        source: a noto-cjk 2.004 `SubsetOTF/SC` face
        output: where to write the cut face
    """
    options = subset.Options()
    options.layout_features = ['*']
    options.drop_tables = [table for table in options.drop_tables if table != 'BASE']
    # `'*'` keeps every record: fontTools' own sentinel, which its `list[int]` annotation leaves out.
    options.name_IDs = ['*']  # pyright: ignore[reportAttributeAccessIssue]
    options.name_languages = ['*']  # pyright: ignore[reportAttributeAccessIssue]
    options.notdef_outline = True
    options.glyph_names = False

    font = TTFont(source, recalcTimestamp=False)
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=TTFont(SHIPPED).getBestCmap())  # pyright: ignore[reportUnknownMemberType]
    subsetter.subset(font)  # pyright: ignore[reportUnknownMemberType]
    font.save(output)


if __name__ == '__main__':
    tyro.cli(main)
