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

//! Fragments every screen shares, and the design tokens behind them.
//! Each takes its geometry from parameters; the screen decides which.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::screens::icons;

// The legacy widget's palette, which the Figma variables agree with:
// white, Gray/40 muted, Gray/90 rules.
pub mod color {
    use bmc_wasm_sdk::{Color, GRAY_40, GRAY_90, WHITE};

    pub const BG: Color = Color::from_hex(0x00_00_00);
    pub const TEXT: Color = WHITE;
    pub const TEXT_MUTED: Color = GRAY_40;
    pub const DIVIDER: Color = GRAY_90;
    pub const BRAND: Color = Color::from_hex(0xE1_06_00);
    pub const PLACEHOLDER: Color = GRAY_90;
}

pub mod font {
    pub const TITLE: u32 = 24;
    /// Every table row, at every frame.
    pub const ROW: u32 = 20;
}

pub mod space {
    pub const PADDING: f32 = 16.0;
    pub const GAP: f32 = 8.0;
}

/// The swept bars, at the artwork's own 10:1 aspect — fitting them to a
/// square collapses them to a dash.
const STRIPE_HEIGHT: f32 = 24.0;
const STRIPE_WIDTH: f32 = STRIPE_HEIGHT * 10.0;

/// The frame every screen sits in: the design's black field, padded.
#[must_use]
pub fn frame(children: Vec<Node>) -> Node {
    col(
        props!(
            background: color::BG,
            padding: space::PADDING,
            gap: space::GAP,
            flex: 1.0
        ),
        children,
    )
}

/// The `F1` mark, the screen's title, and optionally the swept bars.
/// Which frames carry the bars is the screen's call.
#[must_use]
pub fn header(title: &str, stripe: bool) -> Node {
    let mut children = vec![
        text(
            "F1",
            style!(size: font::TITLE, weight: FontWeight::BOLD, color: color::TEXT),
        ),
        text(title, style!(size: font::TITLE, color: color::TEXT_MUTED)),
    ];
    if stripe {
        children.push(spacer(1.0));
        children.push(canvas(
            props!(width: STRIPE_WIDTH, height: STRIPE_HEIGHT),
            [Draw::svg(
                0.0,
                0.0,
                STRIPE_WIDTH,
                STRIPE_HEIGHT,
                &icons::BRAND_STRIPE,
                color::BRAND,
            )
            .with_anti_alias()],
        ));
    }
    row(
        props!(gap: space::GAP * 2.0, cross_align: CrossAlign::Center),
        children,
    )
}

#[must_use]
pub fn divider() -> Node {
    col(props!(height: 1.0, background: color::DIVIDER), [])
}

/// A team's mark, or its livery colour where this build carries no
/// artwork for the team — a new constructor, or one renamed since.
#[must_use]
pub fn team_mark(size: f32, team_name: &str, livery: Color) -> Node {
    let Some(mark) = icons::team_mark(team_name) else {
        return image_placeholder(size, Some(livery));
    };
    canvas(
        props!(width: size, height: size),
        [Draw::bitmap(0.0, 0.0, size, size, mark)],
    )
}

/// Holds an image's box before it arrives, so the row does not reflow
/// when it lands. A livery fills a team's square; anything else is neutral.
#[must_use]
pub fn image_placeholder(size: f32, livery: Option<Color>) -> Node {
    col(
        props!(
            width: size,
            height: size,
            background: livery.unwrap_or(color::PLACEHOLDER)
        ),
        [],
    )
}
