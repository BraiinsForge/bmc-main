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

//! Loading-placeholder builders — the Carbon skeleton pattern.
//!
//! A value's slot keeps its size whether the value is loading or present,
//! so a screen filled by several independent sources never reflows as they
//! land one by one. The host owns the geometry (bar heights, glyph advance,
//! centering); these name the role and the metrics of the text the slot
//! would have held.

use bmc_wasm_protocol::{Color, GRAY_90, SkeletonKind};

use crate::tree::Node;

/// Carbon's `$skeleton-background` tone for the dark themes: the static
/// bar reads as a reserved slot, not as content. (Carbon's brighter
/// `$skeleton-element` is the colour of the sweep this omits.)
pub const ELEMENT_ON_DARK: Color = GRAY_90;

/// A bar standing in for `chars` glyphs of body text at `font_size`.
#[must_use]
pub fn text(chars: f32, font_size: u32, color: Color) -> Node {
    bar(SkeletonKind::Text, chars, font_size, 0.0, 0.0, color)
}

/// A bar standing in for `chars` glyphs of heading-sized text — Carbon's
/// taller bar, for a hero value's slot.
#[must_use]
pub fn heading(chars: f32, font_size: u32, color: Color) -> Node {
    bar(SkeletonKind::Heading, chars, font_size, 0.0, 0.0, color)
}

/// Carbon `.cds--skeleton__placeholder`: a box of an explicit pixel size,
/// for a slot whose shape is not a line of text — a chart area, an icon.
#[must_use]
pub fn placeholder(width: f32, height: f32, color: Color) -> Node {
    bar(SkeletonKind::Placeholder, 0.0, 0, width, height, color)
}

/// A bar as wide as its container, of an explicit height.
#[must_use]
pub fn fill(height: f32, color: Color) -> Node {
    bar(SkeletonKind::Fill, 0.0, 0, 0.0, height, color)
}

fn bar(
    kind: SkeletonKind,
    chars: f32,
    font_size: u32,
    width: f32,
    height: f32,
    color: Color,
) -> Node {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a font size is small and exact in f32"
    )]
    Node::Skeleton {
        kind,
        chars,
        font_size: font_size as f32,
        width,
        height,
        color,
    }
}
