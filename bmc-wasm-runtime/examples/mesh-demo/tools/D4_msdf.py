#!/usr/bin/env python3
"""Generate D4 MSDF assets — 2×2 atlas, faces 1-4."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _msdf_text import main_for_die

if __name__ == '__main__':
    main_for_die(
        img_size=256,
        cols=2,
        rows=2,
        face_values=range(1, 5),
        # Body matches D4.py BG_COLOR (0.20, 0.08, 0.08, 1.0).
        body_color=(51, 20, 20, 255),
        # Label matches D4.py TEXT_COLOR (0.95, 0.90, 0.80, 1.0).
        label_color=(242, 230, 204, 255),
    )
