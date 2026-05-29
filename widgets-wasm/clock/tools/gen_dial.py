#!/usr/bin/env python3
"""Generate a rectangular analog-clock dial SVG.

The dial is 60 radial tick slivers emanating from the panel centre: the 12
hour ticks (every 5th, on the "5s") are white, the 48 minor ticks are grey.
Each tick runs from an inset inner rectangle out to the panel edge, so ticks
bunch toward the corners exactly like the hand-authored dials in
``assets/analog/dial-rect-{317x238,638x238,638x485,1280x480}.svg``.

The renderer (``src/analog/rect.rs``) stretches whichever dial it is handed
to the full widget viewport and recolours the two named paths (``major`` /
``minor``) to the active palette, so only the geometry matters here.

Reference geometry extracted from the existing dials:

    size    panel       inset L/R   inset T/B
    small   317x238     30          20
    medium  638x238     40          30
    large   638x485     50          60
    full    1280x480    95          60

Tick width is ~1px in all sizes. ``inset`` is the cardinal tick length and
is a per-size design choice; pass it explicitly.

Usage:
    gen_dial.py --width 480 --height 320 --inset-lr 38 --inset-tb 34 \
        > ../assets/analog/dial-rect-480x320.svg
    gen_dial.py --width 317 --height 238 --inset-lr 30 --inset-tb 20 --verify
"""

import argparse
import math
import sys

MINOR_COLOR = '#262626'
MAJOR_COLOR = '#fff'
TICK_COUNT = 60
HOUR_STEP = 5  # every 5th tick is a major (hour) tick


def ray_exit(cx, cy, dx, dy, xmin, xmax, ymin, ymax):
    """Point where the ray from (cx,cy) in dir (dx,dy) leaves the rectangle."""
    ts = []
    if dx > 1e-12:
        ts.append((xmax - cx) / dx)
    elif dx < -1e-12:
        ts.append((xmin - cx) / dx)
    if dy > 1e-12:
        ts.append((ymax - cy) / dy)
    elif dy < -1e-12:
        ts.append((ymin - cy) / dy)
    t = min(t for t in ts if t > 0)
    return cx + t * dx, cy + t * dy


def _fmt(v):
    s = f'{v:.3f}'.rstrip('0').rstrip('.')
    return '0' if s in ('-0', '') else s


def _tick_path(cx, cy, dx, dy, inner, panel, width):
    ix, iy = ray_exit(cx, cy, dx, dy, *inner)
    ox, oy = ray_exit(cx, cy, dx, dy, *panel)
    nx, ny = -dy, dx  # unit perpendicular
    hw = width / 2
    pts = [
        (ix + nx * hw, iy + ny * hw),
        (ox + nx * hw, oy + ny * hw),
        (ox - nx * hw, oy - ny * hw),
        (ix - nx * hw, iy - ny * hw),
    ]
    return 'M' + ' L'.join(f'{_fmt(x)} {_fmt(y)}' for x, y in pts) + 'Z'


def generate(width, height, inset_lr, inset_tb, tick_width=1.0):
    cx, cy = width / 2, height / 2
    inner = (inset_lr, width - inset_lr, inset_tb, height - inset_tb)
    panel = (0.0, width, 0.0, height)
    minor, major = [], []
    for k in range(TICK_COUNT):
        th = math.radians(k * (360 / TICK_COUNT))
        dx, dy = math.sin(th), -math.cos(th)  # k=0 -> 12 o'clock (up)
        path = _tick_path(cx, cy, dx, dy, inner, panel, tick_width)
        (major if k % HOUR_STEP == 0 else minor).append(path)
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" '
        f'height="{height:.0f}" fill="none" viewBox="0 0 {width:.0f} {height:.0f}">\n'
        f'    <path id="minor" fill="{MINOR_COLOR}" d="{"".join(minor)}"/>\n'
        f'    <path id="major" fill="{MAJOR_COLOR}" d="{"".join(major)}"/>\n'
        f'</svg>\n'
    )


def verify(width, height, inset_lr, inset_tb, tick_width=1.0):
    """Assert 60 equally-spaced ticks split 12 major / 48 minor."""
    cx, cy = width / 2, height / 2
    inner = (inset_lr, width - inset_lr, inset_tb, height - inset_tb)
    panel = (0.0, width, 0.0, height)
    angles_major, angles_minor = [], []
    for k in range(TICK_COUNT):
        th = math.radians(k * (360 / TICK_COUNT))
        dx, dy = math.sin(th), -math.cos(th)
        ix, iy = ray_exit(cx, cy, dx, dy, *inner)
        # recover the clock angle of the inner endpoint from the centre
        ang = (math.degrees(math.atan2(ix - cx, -(iy - cy))) + 360) % 360
        (angles_major if k % HOUR_STEP == 0 else angles_minor).append(ang)
    total = len(angles_major) + len(angles_minor)
    assert total == 60, f'expected 60 ticks, got {total}'
    assert len(angles_major) == 12, f'expected 12 major, got {len(angles_major)}'
    assert len(angles_minor) == 48, f'expected 48 minor, got {len(angles_minor)}'
    allang = sorted(angles_major + angles_minor)
    diffs = [round((allang[i + 1] - allang[i]), 4) for i in range(len(allang) - 1)]
    assert all(abs(d - 6.0) < 1e-4 for d in diffs), f'uneven spacing: {set(diffs)}'
    assert all(min(a % 30.0, 30.0 - a % 30.0) < 1e-4 for a in angles_major), (
        'major not on 30deg'
    )
    print(
        f'OK  {width:.0f}x{height:.0f}  60 ticks @ 6.0deg  '
        f'(12 major #fff on the 5s, 48 minor {MINOR_COLOR})',
        file=sys.stderr,
    )


def main():
    ap = argparse.ArgumentParser(description='Generate a rectangular clock dial SVG.')
    ap.add_argument('--width', type=float, required=True)
    ap.add_argument('--height', type=float, required=True)
    ap.add_argument(
        '--inset-lr',
        type=float,
        required=True,
        help='left/right cardinal tick length (px)',
    )
    ap.add_argument(
        '--inset-tb',
        type=float,
        required=True,
        help='top/bottom cardinal tick length (px)',
    )
    ap.add_argument('--tick-width', type=float, default=1.0)
    ap.add_argument(
        '--verify',
        action='store_true',
        help='check 60 equally-spaced ticks (12 major / 48 minor)',
    )
    a = ap.parse_args()
    if a.verify:
        verify(a.width, a.height, a.inset_lr, a.inset_tb, a.tick_width)
    sys.stdout.write(generate(a.width, a.height, a.inset_lr, a.inset_tb, a.tick_width))


if __name__ == '__main__':
    main()
