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

//! The entity's icon in its tinted circle, sinking in while the card is pressed,
//! with a hold's progress as a ring around it.

use std::f32::consts::{FRAC_1_SQRT_2, TAU};

use bmc_wasm_sdk::*;

use crate::view::style::{Accent, Icon, dimmed_unless, fill_insets};

/// Tile fills the icon background at 20 % of the state colour.
const BACKGROUND: f32 = 0.2;
/// A pressed icon sinks in like a pushed button, where Tile's grows by 1.2:
/// the ring then wraps the smaller circle, and reaches little past the resting one.
const PRESSED_SCALE: f32 = 0.9;
const RING_GAP: f32 = 6.0;
/// The ring keeps one weight against its circle: 6 px around an 80 px icon.
const RING_SHARE: f32 = 6.0 / 80.0;
/// Around a smaller icon a thinner ring would read as a hairline.
const MIN_RING_WIDTH: f32 = 6.0;
/// Tile draws a 24 px icon in a 36 px circle.
const ICON_SHARE: f32 = 24.0 / 36.0;
/// The mark's radius, as a share of the circle's.
const MARK_RADIUS_SHARE: f32 = 0.375;
/// The side of the glyph in the mark, as a share of the mark's diameter.
const MARK_GLYPH_SHARE: f32 = 0.7;
/// The black rim that parts the mark from the circle it overlaps, as a share of the circle's diameter.
const MARK_RIM_SHARE: f32 = 0.03;

const EXCLAMATION_THICK: Svg = include_svg!("assets/mdi/exclamation-thick.svg");

/// How the badge, and the card it heads, answer a finger on the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feedback {
    /// A press of the card fires nothing now: the icon is dimmed.
    Inert,
    Resting,
    /// A finger is on a card whose press does something: the card is tinted, and the icon sinks in.
    Pressed,
}

/// What a small disc on the circle's top right corner tells of the state, where Tile has its badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Home Assistant cannot reach the entity: Tile's white "!" on orange.
    Unavailable,
}

impl Mark {
    /// On the corner of a circle `radius` around `center`.
    fn draws(self, center: f32, radius: f32, rim: f32) -> [Draw; 3] {
        let (fill, Icon(glyph), ink) = match self {
            Self::Unavailable => (Accent::Warning.color(), Icon(&EXCLAMATION_THICK), WHITE),
        };
        let corner = radius * FRAC_1_SQRT_2;
        let (x, y) = (center + corner, center - corner);
        let mark = radius * MARK_RADIUS_SHARE;
        let side = 2.0 * mark * MARK_GLYPH_SHARE;
        [
            Draw::circle(x, y, mark + rim, BLACK),
            Draw::circle(x, y, mark, fill),
            Draw::svg(x - side / 2.0, y - side / 2.0, side, side, glyph, ink).with_anti_alias(),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Badge {
    pub icon: Icon,
    pub diameter: f32,
    pub color: Color,
    pub feedback: Feedback,
    /// How far a hold has got, from 0 to 1.
    pub progress: Option<f32>,
    pub mark: Option<Mark>,
}

impl Badge {
    /// The side of the square the badge takes, with room for its ring around the pressed icon.
    #[must_use]
    pub const fn side(diameter: f32) -> f32 {
        f32::max(
            diameter,
            diameter * PRESSED_SCALE + 2.0 * (RING_GAP + ring_width(diameter)),
        )
    }

    #[must_use]
    pub fn view(&self) -> Node {
        let side = Self::side(self.diameter);
        let center = side / 2.0;
        let scale = match self.feedback {
            Feedback::Inert | Feedback::Resting => 1.0,
            Feedback::Pressed => PRESSED_SCALE,
        };
        let radius = self.diameter / 2.0 * scale;
        let icon = 2.0 * radius * ICON_SHARE;
        let mut draws = vec![
            Draw::circle(center, center, radius, self.color.with_alpha(BACKGROUND)),
            Draw::svg(
                center - icon / 2.0,
                center - icon / 2.0,
                icon,
                icon,
                self.icon.0,
                self.color,
            )
            .with_anti_alias(),
        ];
        if let Some(progress) = self.progress {
            let width = ring_width(self.diameter);
            draws.push(Draw::arc(
                center,
                center,
                self.diameter / 2.0 * PRESSED_SCALE + RING_GAP + width / 2.0,
                0.0,
                TAU * progress,
                width,
                self.color,
                ArcSegments::Continuous,
                ArcCap::Round,
            ));
        }
        let badge = dimmed_unless(
            self.feedback != Feedback::Inert,
            canvas(props!(width: side, height: side), draws),
        );
        let Some(mark) = self.mark else {
            return badge;
        };
        // Over the ring, and at full brightness on a dimmed icon, as Tile's badge.
        let rim = self.diameter * MARK_RIM_SHARE;
        col(
            props!(width: side, height: side),
            [
                badge,
                canvas(
                    PropsData {
                        width: side,
                        height: side,
                        ..fill_insets()
                    },
                    mark.draws(center, radius, rim),
                ),
            ],
        )
    }
}

const fn ring_width(diameter: f32) -> f32 {
    f32::max(MIN_RING_WIDTH, diameter * RING_SHARE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn the_badge_holds_its_resting_circle_and_its_ring_around_the_pressed_one() {
        for diameter in [48.0, 80.0, 128.0, 400.0] {
            let half = Badge::side(diameter) / 2.0;
            let ring_outer = diameter / 2.0 * PRESSED_SCALE + RING_GAP + ring_width(diameter);
            assert!(diameter / 2.0 <= half, "{diameter}: resting circle");
            assert!(ring_outer <= half, "{diameter}: ring");
        }
    }

    #[wasm_test]
    fn the_ring_thickens_with_a_larger_icon_but_never_below_its_minimum() {
        assert_eq!(ring_width(48.0), MIN_RING_WIDTH);
        assert_eq!(ring_width(80.0), MIN_RING_WIDTH);
        assert!(ring_width(128.0) > MIN_RING_WIDTH);
    }
}
