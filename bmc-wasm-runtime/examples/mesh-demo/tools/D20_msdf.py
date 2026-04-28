#!/usr/bin/env python3
"""Generate D20 MSDF assets — 5×5 atlas (4 used rows + 1 dead), faces 1-20."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _msdf_text import main_for_die

if __name__ == '__main__':
    main_for_die(
        img_size=512,
        cols=5,
        rows=5,
        face_values=range(1, 21),
        # Body matches D20.py BG_COLOR (0.15, 0.08, 0.22, 1.0).
        body_color=(38, 20, 56, 255),
        label_color=(242, 230, 204, 255),
    )
