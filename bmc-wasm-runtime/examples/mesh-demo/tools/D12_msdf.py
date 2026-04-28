#!/usr/bin/env python3
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
