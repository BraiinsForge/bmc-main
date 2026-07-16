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

"""Generate D12 MSDF assets — 4×3 atlas, faces 1-12."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _msdf_text import main_for_die

if __name__ == '__main__':
    main_for_die(
        img_size=512,
        cols=4,
        rows=3,
        face_values=range(1, 13),
        # Body matches D12.py BG_COLOR (0.08, 0.18, 0.12, 1.0).
        body_color=(20, 46, 31, 255),
        label_color=(242, 230, 204, 255),
    )
