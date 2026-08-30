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

use bmc_wasm_sdk::{Color, Draw};

/// Deck panels render the reference design's 0.4 closed alpha unreadably dark.
pub const CLOSED_CHART_ALPHA: f32 = 0.7;

const MARKER_DISC_ALPHA: f32 = 0.9;
const MARKER_BOX_WIDTH: f32 = 0.40;
const MARKER_BOX_HEIGHT: f32 = 0.60;
const MARKER_BAR_WIDTH: f32 = 0.35;
const MARKER_BAR_GAP: f32 = 0.15;
const MARKER_BAR_SIDE_PADDING: f32 = (1.0 - MARKER_BAR_WIDTH * 2.0 - MARKER_BAR_GAP) / 2.0;
const MARKER_BAR_OFFSETS: [f32; 2] = [
    MARKER_BAR_SIDE_PADDING,
    MARKER_BAR_SIDE_PADDING + MARKER_BAR_WIDTH + MARKER_BAR_GAP,
];

/// Draws the bars as geometry because the embedded fonts lack U+23F8 PAUSE BUTTON.
#[must_use]
pub fn pause_marker(diameter: f32, disc_color: Color, background: Color) -> Vec<Draw> {
    let box_width = diameter * MARKER_BOX_WIDTH;
    let box_height = diameter * MARKER_BOX_HEIGHT;
    let left = (diameter - box_width) / 2.0;
    let top = (diameter - box_height) / 2.0;
    let mut draws = vec![Draw::circle(
        diameter / 2.0,
        diameter / 2.0,
        diameter / 2.0,
        disc_color.with_alpha(MARKER_DISC_ALPHA),
    )];
    draws.extend(MARKER_BAR_OFFSETS.iter().map(|&x| {
        Draw::rect(
            left + box_width * x,
            top,
            box_width * MARKER_BAR_WIDTH,
            box_height,
            background,
        )
    }));
    draws
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_marker_centers_equal_width_bars() {
        let diameter = 20.0;
        let draws = pause_marker(
            diameter,
            Color::from_rgb(0xc6, 0xc6, 0xc6),
            Color::from_rgb(0, 0, 0),
        );
        let [Draw::Circle { .. }, first, second] = draws.as_slice() else {
            panic!("BUG: the pause marker must contain one disc and two bars");
        };
        let Draw::Rect {
            x: first_x,
            w: first_width,
            ..
        } = first
        else {
            panic!("BUG: the first pause bar must be a rectangle");
        };
        let Draw::Rect {
            x: second_x,
            w: second_width,
            ..
        } = second
        else {
            panic!("BUG: the second pause bar must be a rectangle");
        };

        let box_width = diameter * MARKER_BOX_WIDTH;
        let left = (diameter - box_width) / 2.0;
        let first_padding = *first_x - left;
        let gap = *second_x - (*first_x + *first_width);
        let second_padding = left + box_width - (*second_x + *second_width);
        let tolerance = f32::EPSILON * diameter;
        assert!(
            (*first_width - *second_width).abs() < tolerance,
            "pause bars must have equal widths"
        );
        assert!(
            (first_padding - second_padding).abs() < tolerance,
            "pause bars must be centered within the marker box"
        );
        assert!(
            (gap - box_width * MARKER_BAR_GAP).abs() < tolerance,
            "pause bar gap must match the authored fraction"
        );
    }
}
