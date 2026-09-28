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

/// Draws the bars as geometry because the embedded fonts lack U+23F8 PAUSE BUTTON.
///
/// The disc is opaque, in the shade it takes at [`MARKER_DISC_ALPHA`] over `background`,
/// so an icon it covers cannot show through.
/// The bars sit on whole pixels, the pair centred on the disc:
/// the renderer snaps each fractional edge on its own,
/// which closed a 16 px marker's sub-pixel gap into a single slot,
/// and anti-aliasing half-fills a gap that narrow instead.
#[must_use]
pub fn pause_marker(diameter: f32, disc_color: Color, background: Color) -> Vec<Draw> {
    let disc = whole_px(diameter);
    let box_width = diameter * MARKER_BOX_WIDTH;
    let bar_width = whole_px(box_width * MARKER_BAR_WIDTH).max(1);
    let gap = matching_parity(box_width * MARKER_BAR_GAP, disc);
    let bar_height = matching_parity(diameter * MARKER_BOX_HEIGHT, disc);
    let left = disc.saturating_sub(2 * bar_width + gap) / 2;
    let top = disc.saturating_sub(bar_height) / 2;
    let mut draws = vec![Draw::circle(
        diameter / 2.0,
        diameter / 2.0,
        diameter / 2.0,
        composite(disc_color, MARKER_DISC_ALPHA, background),
    )];
    draws.extend(
        [left, left + bar_width + gap]
            .map(|x| Draw::rect(px(x), px(top), px(bar_width), px(bar_height), background)),
    );
    draws
}

/// `color` at `alpha` over `background`, blended per sRGB channel as the renderer does.
fn composite(color: Color, alpha: f32, background: Color) -> Color {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a blend of two bytes, rounded and clamped to a byte"
    )]
    let channel = |over: u8, under: u8| {
        let (over, under) = (f32::from(over), f32::from(under));
        (under + (over - under) * alpha).round().clamp(0.0, 255.0) as u8
    };
    Color::from_rgb(
        channel(color.red(), background.red()),
        channel(color.green(), background.green()),
        channel(color.blue(), background.blue()),
    )
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a marker is a few dozen pixels across, rounded first"
)]
fn whole_px(length: f32) -> u32 {
    length.round().max(0.0) as u32
}

fn px(length: u32) -> f32 {
    f32::from(u16::try_from(length).expect("BUG: a marker is a few dozen pixels across"))
}

/// The whole length nearest `target` that centres in `disc` on whole pixels,
/// which takes the disc's parity; never below one pixel.
fn matching_parity(target: f32, disc: u32) -> u32 {
    let nearest = whole_px(target).max(1);
    if (nearest + disc).is_multiple_of(2) {
        nearest
    } else if target >= px(nearest) || nearest == 1 {
        nearest + 1
    } else {
        nearest - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYMBOL: Color = Color::from_rgb(0xc6, 0xc6, 0xc6);
    const BLACK: Color = Color::from_rgb(0, 0, 0);

    /// Each bar of a `diameter` marker as `(x, y, w, h)`.
    fn bars(diameter: f32) -> [(f32, f32, f32, f32); 2] {
        let draws = pause_marker(diameter, SYMBOL, BLACK);
        let [Draw::Circle { .. }, first, second] = draws.as_slice() else {
            panic!("BUG: the pause marker must contain one disc and two bars");
        };
        [first, second].map(|bar| {
            let Draw::Rect { x, y, w, h, .. } = bar else {
                panic!("BUG: a pause bar must be a rectangle");
            };
            (*x, *y, *w, *h)
        })
    }

    #[test]
    fn marker_bars_sit_apart_on_whole_pixels_centred_at_every_size() {
        for diameter in 10..=48_u16 {
            let diameter = f32::from(diameter);
            let [first, second] = bars(diameter);
            for edge in [first.0, first.1, first.2, first.3, second.0] {
                assert_eq!(
                    edge.fract(),
                    0.0,
                    "{diameter} px: {edge} is off the pixel grid"
                );
            }
            assert_eq!(first.2, second.2, "{diameter} px: the bars differ in width");
            let gap = second.0 - (first.0 + first.2);
            assert!(gap >= 1.0, "{diameter} px: the bars touch");
            let right = diameter - (second.0 + second.2);
            assert_eq!(first.0, right, "{diameter} px: the bars sit off centre");
            assert_eq!(
                first.1 * 2.0 + first.3,
                diameter,
                "{diameter} px: the bars sit off centre"
            );
        }
    }

    #[test]
    fn a_deck_sized_marker_keeps_its_three_two_three_bars() {
        let [first, second] = bars(24.0);
        assert_eq!(first, (8.0, 5.0, 3.0, 14.0));
        assert_eq!(second, (13.0, 5.0, 3.0, 14.0));
    }

    #[test]
    fn the_disc_is_opaque_in_the_shade_it_showed_over_black() {
        let draws = pause_marker(24.0, SYMBOL, BLACK);
        let Some(Draw::Circle { fill, .. }) = draws.first() else {
            panic!("BUG: the pause marker opens with its disc");
        };
        assert_eq!(*fill, Color::from_rgb(0xb2, 0xb2, 0xb2).into());
    }
}
