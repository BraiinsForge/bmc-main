#!/usr/bin/env python3
"""Generate D8 MSDF assets — 3×3 atlas (8 cells used + 1 dead), faces 1-8."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _msdf_text import main_for_die

if __name__ == '__main__':
    main_for_die(
        img_size=256,
        cols=3,
        rows=3,
        face_values=range(1, 9),
        # Body matches D8.py BG_COLOR (0.12, 0.12, 0.18, 1.0).
        body_color=(31, 31, 46, 255),
        label_color=(242, 230, 204, 255),
    )
