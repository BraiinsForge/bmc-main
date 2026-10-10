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

//! Loading-placeholder component — the Carbon skeleton pattern.
//!
//! A value's slot keeps its size whether the value is loading or present,
//! so a screen filled by several independent sources never reflows as they
//! land one by one.
//!
//! Geometry from `packages/styles/scss/components/skeleton-styles`:
//! square corners (only Carbon's circular variant rounds), and a bar of
//! a fixed height per role — 16 px for a text line, 24 px for a heading —
//! rather than a fraction of the font size. A text bar centers in the
//! line box its text would have occupied, so the gaps around the slot
//! are unchanged.
//!
//! Carbon animates the bar with a 3 s eased sweep; the Deck renders it
//! static, matching Carbon's own reduced-motion path, to leave the GPU
//! budget to the widget's content.

use bmc_ui::{SkeletonData, SkeletonKind};

use crate::renderer::Renderer;

/// Carbon `.cds--skeleton__text` bar height.
const TEXT_H: f32 = 16.0;

/// Carbon `.cds--skeleton__heading` bar height.
const HEADING_H: f32 = 24.0;

/// Average glyph advance as a fraction of the font size, for sizing a bar
/// to the text it stands in for — measured against the deck faces' digits,
/// which run a little wider than their prose.
const GLYPH_ADVANCE: f32 = 0.55;

/// The node's layout size. `Fill`'s width comes from its container,
/// so it reports none and stretches.
pub(crate) fn layout_size(skeleton: &SkeletonData) -> (Option<f32>, f32) {
    match skeleton.kind {
        // Both text roles size to the line their text would have held.
        SkeletonKind::Text | SkeletonKind::Heading => {
            (Some(bar_width(skeleton)), skeleton.font_size)
        }
        SkeletonKind::Placeholder => (Some(skeleton.width), skeleton.height),
        SkeletonKind::Fill => (None, skeleton.height),
    }
}

fn bar_width(skeleton: &SkeletonData) -> f32 {
    skeleton.chars * skeleton.font_size * GLYPH_ADVANCE
}

/// The bar's height inside the node's box; text roles center theirs
/// in the line box, the others fill it.
fn bar_height(skeleton: &SkeletonData) -> f32 {
    match skeleton.kind {
        SkeletonKind::Text => TEXT_H,
        SkeletonKind::Heading => HEADING_H,
        SkeletonKind::Placeholder | SkeletonKind::Fill => skeleton.height,
    }
}

/// Paint a skeleton into its laid-out box: a square-cornered bar, centered
/// vertically when it stands in for a line of text.
pub(crate) fn render_skeleton(
    renderer: &mut dyn Renderer,
    skeleton: &SkeletonData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    let bar_h = bar_height(skeleton).min(h);
    let bar_y = y + (h - bar_h) / 2.0;
    renderer.fill_rect(x, bar_y, w, bar_h, skeleton.color);
}
