// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use super::scissor_box;
use crate::interaction::Rect;

const W: i32 = 1280;
const H: i32 = 480;

#[test]
fn a_fractional_right_edge_is_covered_not_clipped() {
    let (x, _, w, _) = scissor_box(Rect::new(10.2, 0.0, 5.2, 4.0), 1.0, W, H);
    assert_eq!(
        (x, x + w),
        (10, 16),
        "10.2..15.4 must cover columns 10..=15"
    );
}

#[test]
fn a_fractional_bottom_edge_is_covered_not_clipped() {
    let (_, y, _, h) = scissor_box(Rect::new(0.0, 10.2, 4.0, 5.2), 1.0, W, H);
    assert_eq!(
        (H - (y + h), H - y),
        (10, 16),
        "top-down 10.2..15.4 must cover rows 10..=15"
    );
}

#[test]
fn a_whole_pixel_rect_gains_nothing() {
    let (x, y, w, h) = scissor_box(Rect::new(10.0, 20.0, 5.0, 6.0), 1.0, W, H);
    assert_eq!((x, w, h), (10, 5, 6));
    assert_eq!(y, H - 26, "bottom-left origin, from the rect's lower edge");
}

#[test]
fn dpi_scales_before_rounding() {
    let (x, _, w, _) = scissor_box(Rect::new(10.2, 0.0, 5.2, 4.0), 2.0, W, H);
    assert_eq!(
        (x, x + w),
        (20, 31),
        "20.4..30.8 must cover columns 20..=30"
    );
}

/// Unclamped, a rect past the target hands GL a box beyond it — and a
/// negative origin, a negative width.
#[test]
fn a_rect_outside_the_target_is_clamped_to_it() {
    let (x, y, w, h) = scissor_box(Rect::new(-20.0, -20.0, 40.0, 40.0), 1.0, W, H);
    assert_eq!((x, w), (0, 20));
    assert_eq!((y + h, h), (H, 20));

    let (x, _, w, _) = scissor_box(Rect::new(1270.0, 0.0, 40.0, 4.0), 1.0, W, H);
    assert_eq!(
        (x, w),
        (1270, 10),
        "clipped at the right edge, never past it"
    );

    let (x, y, w, h) = scissor_box(Rect::new(2000.0, 2000.0, 10.0, 10.0), 1.0, W, H);
    assert!(
        w >= 0 && h >= 0,
        "a fully offscreen rect stays non-negative"
    );
    assert_eq!((x, y, w, h), (W, H - H, 0, 0));
}
