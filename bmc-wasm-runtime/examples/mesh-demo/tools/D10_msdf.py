#!/usr/bin/env python3
"""Generate D10 MSDF assets — 5×3 atlas, faces 0-9 (cell index = value)."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _msdf_text import main_for_die

if __name__ == '__main__':
    main_for_die(
        img_size=512,
        cols=5,
        rows=3,
        face_values=range(0, 10),
        # Body matches D10.py BG_COLOR (0.18, 0.10, 0.22, 1.0).
        body_color=(46, 26, 56, 255),
        label_color=(242, 230, 204, 255),
    )
