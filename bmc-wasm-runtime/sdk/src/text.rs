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

//! Text primitives: spans, styles, and text/paragraph builders.

use bmc_wasm_protocol::{
    AnimProperty, Color, CrossAlign, Easing, FontWeight, GRAY_10, LoopMode, PropsData, TextStyle,
};

use crate::props;
use crate::tree::{Draw, Node, row, touchable};

/// Definition of a single animation (serialized to host).
#[derive(Clone, Debug)]
pub struct AnimationDef {
    pub property: AnimProperty,
    pub from: f32,
    pub to: f32,
    pub duration_ms: u32,
    pub delay_ms: u16,
    pub easing: Easing,
    pub loop_mode: LoopMode,
}

/// Definition of a transition (serialized to host).
///
/// The host keys transition state on `(canvas_index, id_hash)`
/// so the interpolation follows the *logical* draw across tree-shape
/// changes (e.g. an optional sibling appearing or disappearing).
///
/// Without an id the host would key on draw position, which silently
/// swaps state between draws when their relative order shifts.
#[derive(Clone, Debug)]
pub struct TransitionDef {
    pub id_hash: u32,
    pub duration_ms: u32,
    pub easing: Easing,
}

/// FNV1a-32 hash of `s`. Used by to derive a stable per-draw key
/// from the widget-supplied id string at call time.
///
/// Const so `Draw::transition("hour-hand", …)` const-folds
/// the hash at compile time for `&'static str` ids.
#[must_use]
pub const fn fnv1a_32(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut hash: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(0x0100_0193);
        i += 1;
    }
    hash
}

/// Path interpolation mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Interpolation {
    /// Straight line segments between points.
    #[default]
    Linear = 0,
    /// Smooth Catmull-Rom spline through all points (host converts to cubic Bézier).
    CatmullRom = 1,
}

/// A text span with optional style overrides
#[derive(Clone, Debug)]
pub struct Span {
    pub text: String,
    pub weight: Option<FontWeight>,
    pub color: Option<Color>,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Span {
    /// Serialize span style flags (u16) and optional color
    /// flags bits:
    ///   0-11:  weight (if has_weight)
    ///   12:    has_weight
    ///   13:    has_color (color u32 follows after text)
    ///   14:    italic
    ///   15:    underline
    /// Note: strikethrough is in the extra byte if needed
    #[must_use]
    pub fn flags(&self) -> u16 {
        let weight_bits = self.weight.map_or(0, |w| w.0) & 0xFFF;
        let has_weight = if self.weight.is_some() { 1 << 12 } else { 0 };
        let has_color = if self.color.is_some() { 1 << 13 } else { 0 };
        let italic_bit = if self.italic { 1 << 14 } else { 0 };
        let underline_bit = if self.underline { 1 << 15 } else { 0 };
        weight_bits | has_weight | has_color | italic_bit | underline_bit
    }

    /// Extra flags byte for strikethrough (separate to fit in u16)
    #[must_use]
    pub fn extra_flags(&self) -> u8 {
        u8::from(self.strikethrough)
    }
}

/// Trait for optional style argument in span()
pub trait IntoSpanStyle {
    fn apply(self, span: &mut Span);
}

impl IntoSpanStyle for () {
    fn apply(self, _span: &mut Span) {}
}

impl IntoSpanStyle for StyleResult {
    fn apply(self, span: &mut Span) {
        let ts = self.0;
        if ts.weight != FontWeight::REGULAR {
            span.weight = Some(ts.weight);
        }
        if ts.color != GRAY_10 {
            span.color = Some(ts.color);
        }
        span.italic = ts.italic;
        span.underline = ts.underline;
        span.strikethrough = ts.strikethrough;
    }
}

/// Create a text span, optionally with style overrides
///
/// # Examples
/// ```ignore
/// span("plain text", ())
/// span("bold", style!(weight: FontWeight::BOLD))
/// span("colored", style!(color: RED_50))
/// ```
pub fn span(text: impl Into<String>, style: impl IntoSpanStyle) -> Span {
    let mut s = Span {
        text: text.into(),
        weight: None,
        color: None,
        italic: false,
        underline: false,
        strikethrough: false,
    };
    style.apply(&mut s);
    s
}

/// Combined text style and layout props for the style!() macro
#[derive(Clone, Copy, Debug)]
pub struct StyleResult(pub TextStyle, pub PropsData);

impl From<StyleResult> for TextStyle {
    fn from(sr: StyleResult) -> Self {
        sr.0
    }
}

impl From<StyleResult> for PropsData {
    fn from(sr: StyleResult) -> Self {
        sr.1
    }
}

/// Simple text node with unified styling
pub fn text(content: impl Into<String>, style: StyleResult) -> Node {
    Node::Paragraph {
        props: style.1,
        base_style: style.0,
        spans: vec![span(content, ())],
    }
}

/// Tappable text that hugs its label: the label sizes the box and a transparent,
/// touch-keyed canvas absolutely fills it, so the hit region matches the glyphs.
/// The SDK can't measure text, so the overlay avoids guessing a width. `id` is
/// the click key surfaced in the render readback.
#[must_use]
pub fn link(id: &str, label: impl Into<String>, style: StyleResult) -> Node {
    row(
        props!(cross_align: CrossAlign::Center),
        [
            text(label, style),
            touchable(
                id,
                props!(inset_top: 0.0, inset_right: 0.0, inset_bottom: 0.0, inset_left: 0.0),
                Vec::<Draw>::new(),
            ),
        ],
    )
}

/// Rich paragraph with multiple styled spans
pub fn paragraph(style: StyleResult, spans: impl IntoIterator<Item = Span>) -> Node {
    Node::Paragraph {
        props: style.1,
        base_style: style.0,
        spans: spans.into_iter().collect(),
    }
}
