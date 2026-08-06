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

use bmc_wasm_protocol::{Color, SkeletonKind};

use crate::renderer::Renderer;

#[derive(Clone, Copy, Default, Debug)]
pub struct SkeletonData {
    pub kind: SkeletonKind,
    /// Glyph count the slot would have held (`Text` / `Heading`).
    pub chars: f32,
    /// Font size of the text the slot stands in for (`Text` / `Heading`).
    pub font_size: f32,
    /// Explicit box for `Placeholder`, and the bar height for `Fill`.
    pub width: f32,
    pub height: f32,
    pub color: Color,
}

/// Carbon `.cds--skeleton__text` bar height.
const TEXT_H: f32 = 16.0;

/// Carbon `.cds--skeleton__heading` bar height.
const HEADING_H: f32 = 24.0;

/// Average glyph advance as a fraction of the font size, for sizing a bar
/// to the text it stands in for — measured against the deck faces' digits,
/// which run a little wider than their prose.
const GLYPH_ADVANCE: f32 = 0.55;

impl SkeletonData {
    /// The node's layout size. `Fill`'s width comes from its container,
    /// so it reports none and stretches.
    pub(crate) fn layout_size(&self) -> (Option<f32>, f32) {
        match self.kind {
            // Both text roles size to the line their text would have held.
            SkeletonKind::Text | SkeletonKind::Heading => (Some(self.bar_width()), self.font_size),
            SkeletonKind::Placeholder => (Some(self.width), self.height),
            SkeletonKind::Fill => (None, self.height),
        }
    }

    fn bar_width(&self) -> f32 {
        self.chars * self.font_size * GLYPH_ADVANCE
    }

    /// The bar's height inside the node's box; text roles center theirs
    /// in the line box, the others fill it.
    fn bar_height(&self) -> f32 {
        match self.kind {
            SkeletonKind::Text => TEXT_H,
            SkeletonKind::Heading => HEADING_H,
            SkeletonKind::Placeholder | SkeletonKind::Fill => self.height,
        }
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
    let bar_h = skeleton.bar_height().min(h);
    let bar_y = y + (h - bar_h) / 2.0;
    renderer.fill_rect(x, bar_y, w, bar_h, skeleton.color);
}
