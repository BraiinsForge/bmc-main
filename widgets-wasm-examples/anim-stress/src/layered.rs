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

//! The layered scene: static content over, and beside, an animated strip.
//!
//! The cached static layer composites in one go before the dynamic half,
//! so a static node that paints *after* something dynamic
//! can only go in the layer while the two do not overlap.
//! Both cases have to be got right, and a widget only exercises them
//! if it holds both, so this holds both:
//!
//! - a text wall the animation never reaches, which belongs in the layer;
//! - an animated band strip;
//! - two identical badges, one over the strip and one over the wall, each
//!   static and each painted after the strip.
//!
//! The badge over the strip must repaint with the dynamic half to stay on top of it;
//! the badge over the wall may stay in the layer.
//! Getting either wrong is silent — the badge is drawn and then covered,
//! or never repainted — so both are lettered and high-contrast,
//! and a frame missing one says which case broke.
//!
//! The wall is deliberately expensive to rasterise.
//! A build that gives up the layer for this tree re-rasterises it every frame,
//! which is the cost banding exists to avoid and the difference this scene shows.

use bmc_wasm_sdk::{
    Color, Draw, FontWeight, GRAY_20, GRAY_80, Node, TextAlign, canvas, props, row, style, ufmt,
};

/// Bands tiling the animated half. Enough to keep the strip visibly alive,
/// few enough that recording them stays negligible against the wall.
const BAND_COUNT: usize = 12;

/// Rows of text in the static wall. Text is the most expensive static content
/// a widget can hold — shaping and rasterisation both — so the wall is what
/// makes a lost layer measurable rather than merely wrong.
const WALL_ROWS: usize = 18;

const BADGE_W: f32 = 180.0;
/// Share of its half a badge may take, so on a narrow slot
/// the badge over the wall still stops short of the strip.
const BADGE_MAX_SHARE: f32 = 0.8;
const BADGE_H: f32 = 64.0;
const BADGE_TEXT: u16 = 20;

/// Badge fill, against both the wall's grey and the strip's animated colours.
const BADGE_BG: Color = Color::from_rgb(250, 240, 60);
const BADGE_FG: Color = Color::from_rgb(10, 10, 10);

/// The scene for a `w` by `h` surface.
pub(crate) fn tree(w: f32, h: f32) -> Node {
    let half = w / 2.0;
    let badge_w = BADGE_W.min(half * BADGE_MAX_SHARE);
    let badge_y = h / 2.0 - BADGE_H / 2.0;
    row(
        props!(width: w, height: h),
        [
            canvas(props!(width: half, height: h), wall(half, h)),
            canvas(
                props!(width: half, height: h),
                crate::bands(half, h, BAND_COUNT),
            ),
            // Over the animation: painted after it and overlapping it, so it
            // has to repaint with the dynamic half to stay on top.
            badge("OVER ANIM", badge_w, half + (half - badge_w) / 2.0, badge_y),
            // Over the wall: painted just as late, but overlapping nothing dynamic,
            // so paint order is unobservable and it belongs in the layer.
            badge("OVER WALL", badge_w, (half - badge_w) / 2.0, badge_y),
        ],
    )
}

/// A static badge `w` wide, pinned at `x`, `y` in the root's coordinates.
fn badge(label: &str, w: f32, x: f32, y: f32) -> Node {
    canvas(
        props!(
            width: w,
            height: BADGE_H,
            inset_top: y,
            inset_left: x,
        ),
        [
            Draw::rect(0.0, 0.0, w, BADGE_H, BADGE_BG),
            Draw::text(
                w / 2.0,
                (BADGE_H - f32::from(BADGE_TEXT)) / 2.0,
                label,
                style!(size: u32::from(BADGE_TEXT), color: BADGE_FG, align: TextAlign::Center, weight: FontWeight::BOLD),
            ),
        ],
    )
}

/// The static half: a filled panel and rows of text, none of it animated.
fn wall(w: f32, h: f32) -> Vec<Draw> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "row count is small and exact in f32"
    )]
    let row_h = h / WALL_ROWS as f32;
    let mut draws = Vec::with_capacity(WALL_ROWS + 1);
    draws.push(Draw::rect(0.0, 0.0, w, h, GRAY_80));
    for i in 0..WALL_ROWS {
        #[expect(
            clippy::cast_precision_loss,
            reason = "row index is bounded by WALL_ROWS"
        )]
        let y = i as f32 * row_h;
        // Numbered so no two rows shape identically: a wall of one repeated
        // line would hit the paragraph cache and understate what a lost layer
        // costs.
        let mut line = String::with_capacity(48);
        ufmt::uwrite!(
            &mut line,
            "{} the quick brown fox jumps over the lazy dog",
            i
        )
        .expect("BUG: writing into a String cannot fail");
        draws.push(Draw::text(
            8.0,
            y + 2.0,
            line,
            style!(size: 14, color: GRAY_20),
        ));
    }
    draws
}
