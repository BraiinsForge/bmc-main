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

use super::linear_endpoints;

fn approx(a: (f32, f32), b: (f32, f32)) {
    assert!(
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01,
        "{a:?} != {b:?}"
    );
}

#[test]
fn zero_degrees_is_top_to_bottom() {
    let (start, end) = linear_endpoints(0.0, 0.0, 100.0, 100.0, 0.0);
    approx(start, (50.0, 0.0));
    approx(end, (50.0, 100.0));
}

#[test]
fn ninety_degrees_is_left_to_right() {
    let (start, end) = linear_endpoints(0.0, 0.0, 100.0, 100.0, 90.0);
    approx(start, (0.0, 50.0));
    approx(end, (100.0, 50.0));
}

#[test]
fn forty_five_degrees_spans_the_diagonal() {
    let (start, end) = linear_endpoints(0.0, 0.0, 100.0, 100.0, 45.0);
    approx(start, (0.0, 0.0));
    approx(end, (100.0, 100.0));
}

#[test]
fn non_square_box_does_not_land_on_corners() {
    // 200x100 box at 45 deg: endpoints fall off the corners, exercising
    // the general projection path rather than the square special case.
    let (start, end) = linear_endpoints(0.0, 0.0, 200.0, 100.0, 45.0);
    approx(start, (25.0, -25.0));
    approx(end, (175.0, 125.0));
}
