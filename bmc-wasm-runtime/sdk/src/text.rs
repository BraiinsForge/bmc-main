// Copyright (C) 2026  Braiins Systems s.r.o.

//! Text primitives: spans, styles, and text/paragraph builders.

use bmc_wasm_protocol::{AnimProperty, Color, Easing, GRAY_10, LoopMode, PropsData, TextStyle};

use crate::tree::Node;

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
#[derive(Clone, Debug)]
pub struct TransitionDef {
    pub duration_ms: u32,
    pub easing: Easing,
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

/// Named CSS font weights. The text primitive carries a raw `u16` weight
/// (see [`Span::weight`]); this enum is the typed convention for widgets
/// that prefer named weights, and converts directly via `weight as u16`.
///
/// Only the weights the deck's font set ships with are enumerated. Add
/// more here as fonts gain them — keep the discriminants matching the
/// CSS standard so the renderer's `weight >= 600` threshold stays valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum FontWeight {
    Regular = 400,
    SemiBold = 600,
    Bold = 700,
}

/// A text span with optional style overrides
#[derive(Clone, Debug)]
pub struct Span {
    pub text: String,
    pub weight: Option<u16>,
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
        let weight_bits = self.weight.unwrap_or(0) & 0xFFF;
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
        if ts.weight != 400 {
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
/// span("bold", style!(weight: 700))
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

/// Rich paragraph with multiple styled spans
pub fn paragraph(style: StyleResult, spans: impl IntoIterator<Item = Span>) -> Node {
    Node::Paragraph {
        props: style.1,
        base_style: style.0,
        spans: spans.into_iter().collect(),
    }
}
