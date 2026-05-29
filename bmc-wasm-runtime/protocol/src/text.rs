// Copyright (C) 2026  Braiins Systems s.r.o.

//! Text styling types shared between SDK and host.

use core::fmt;

use crate::colors::{Color, GRAY_10, TRANSPARENT};
use crate::ids::BitmapId;

/// Horizontal text alignment.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TextAlign {
    #[default]
    Left = 0,
    Center = 1,
    Right = 2,
}

/// Vertical anchor for the `(x, y)` of a single-line canvas-mode
/// `Draw::text`. Maps directly onto the renderer's femtovg baselines.
///
/// `Top` (the default) keeps the historical behaviour: `y` is the top
/// edge of the glyph box. `Center` puts the visual centre on `y` —
/// matching the natural anchor for badges, date windows, callouts and
/// anything else that wants the text centred on a layout anchor
/// without offset-by-half-font-size fudges at the call site.
///
/// `Baseline` is the typographic baseline (alphabetic). Useful when
/// aligning text to a rule drawn at the same `y`.
///
/// Note: this affects `Draw::text` (canvas mode) only. Multi-line
/// paragraph text built via `text(...)` / `paragraph(...)` is
/// positioned by the parent layout's cross-align, which the wider
/// `Node` tree already handles.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum VerticalAlign {
    #[default]
    Top = 0,
    Center = 1,
    Bottom = 2,
    Baseline = 3,
}

/// Text anchor along an arc.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ArcAnchor {
    #[default]
    Start = 0,
    Center = 1,
    End = 2,
}

/// Invalid [`ArcAnchor`] wire discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidArcAnchor(pub u8);

impl fmt::Display for InvalidArcAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid arc anchor wire value {}", self.0)
    }
}

impl std::error::Error for InvalidArcAnchor {}

impl From<ArcAnchor> for u8 {
    fn from(anchor: ArcAnchor) -> Self {
        anchor as Self
    }
}

impl TryFrom<u8> for ArcAnchor {
    type Error = InvalidArcAnchor;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Start),
            1 => Ok(Self::Center),
            2 => Ok(Self::End),
            _ => Err(InvalidArcAnchor(value)),
        }
    }
}

/// Text facing direction along an arc.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ArcTextFacing {
    #[default]
    Outward = 0,
    Inward = 1,
}

/// Invalid [`ArcTextFacing`] wire discriminant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidArcTextFacing(pub u8);

impl fmt::Display for InvalidArcTextFacing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid arc text facing wire value {}", self.0)
    }
}

impl std::error::Error for InvalidArcTextFacing {}

impl From<ArcTextFacing> for u8 {
    fn from(facing: ArcTextFacing) -> Self {
        facing as Self
    }
}

impl TryFrom<u8> for ArcTextFacing {
    type Error = InvalidArcTextFacing;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Outward),
            1 => Ok(Self::Inward),
            _ => Err(InvalidArcTextFacing(value)),
        }
    }
}

/// Cross-axis alignment for row/column containers.
///
/// Controls how children are aligned perpendicular to the main flex direction:
/// - Row: controls vertical alignment of children
/// - Column: controls horizontal alignment of children
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CrossAlign {
    /// Children stretch to fill the container's cross-axis (default).
    #[default]
    Stretch = 0,
    /// Children are centered along the cross-axis.
    Center = 1,
    /// Children are packed to the start of the cross-axis.
    Start = 2,
    /// Children are packed to the end of the cross-axis.
    End = 3,
}

/// Packed layout flags occupying 4 bytes (offset 36–40) in `PropsData` wire
/// format. Bit allocation is documented here so future flags add a named
/// constant + accessor instead of reaching into the bits ad-hoc.
///
/// | bits      | meaning                              |
/// |-----------|--------------------------------------|
/// | `0..8`    | `CrossAlign` discriminant            |
/// | `8`       | `wrap` (bool)                        |
/// | `9..32`   | reserved — must remain zero          |
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutFlags(u32);

impl LayoutFlags {
    const CROSS_ALIGN_MASK: u32 = 0xFF;
    const FLAG_WRAP: u32 = 1 << 8;

    #[must_use]
    pub fn new(cross_align: CrossAlign, wrap: bool) -> Self {
        let mut bits = (cross_align as u32) & Self::CROSS_ALIGN_MASK;
        if wrap {
            bits |= Self::FLAG_WRAP;
        }
        Self(bits)
    }

    #[must_use]
    pub fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    #[must_use]
    pub fn bits(self) -> u32 {
        self.0
    }

    #[must_use]
    pub fn cross_align(self) -> CrossAlign {
        match self.0 & Self::CROSS_ALIGN_MASK {
            1 => CrossAlign::Center,
            2 => CrossAlign::Start,
            3 => CrossAlign::End,
            _ => CrossAlign::Stretch,
        }
    }

    #[must_use]
    pub fn wrap(self) -> bool {
        self.0 & Self::FLAG_WRAP != 0
    }
}

/// Text overflow behavior for single-line text
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TextOverflow {
    /// Normal line wrapping (default)
    #[default]
    Wrap = 0,
    /// Single line, hard clip at container edge
    Clip = 1,
    /// Single line, truncate with "…"
    Ellipsis = 2,
}

/// CSS-style font weight, stored as a raw `u16` so widget code can use
/// either the named constants ([`Self::REGULAR`], [`Self::SEMIBOLD`],
/// [`Self::BOLD`]) or an arbitrary intermediate value (`FontWeight(500)`).
/// Only the weights the deck's font set ships with have named constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct FontWeight(pub u16);

impl FontWeight {
    pub const REGULAR: Self = Self(400);
    pub const SEMIBOLD: Self = Self(600);
    pub const BOLD: Self = Self(700);
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::REGULAR
    }
}

impl From<u16> for FontWeight {
    fn from(w: u16) -> Self {
        Self(w)
    }
}

impl From<FontWeight> for u16 {
    fn from(w: FontWeight) -> Self {
        w.0
    }
}

/// Font family selector. `Sans = 0` keeps existing serialized widgets on
/// the historical Braiins Sans face; `DeckSans` switches to the display
/// face used by the legacy slint deck.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum FontFamily {
    #[default]
    Sans = 0,
    DeckSans = 1,
}

/// Text style for paragraphs (28 bytes serialized).
#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub size: u32,          // default: 16
    pub color: Color,       // default: GRAY_10
    pub max_width: u32,     // default: 0 (use container width)
    pub weight: FontWeight, // default: FontWeight::REGULAR
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub line_height: f32, // default: 1.4 (multiplier)
    pub align: TextAlign,
    pub text_overflow: TextOverflow,   // default: Wrap
    pub outline_color: Color,          // default: TRANSPARENT (no outline)
    pub outline_width: f32,            // default: 0.0 (no outline)
    pub vertical_align: VerticalAlign, // default: Top
    pub family: FontFamily,            // default: Sans
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 16,
            color: GRAY_10,
            max_width: 0,
            weight: FontWeight::REGULAR,
            italic: false,
            underline: false,
            strikethrough: false,
            line_height: 1.4,
            align: TextAlign::Left,
            text_overflow: TextOverflow::Wrap,
            outline_color: TRANSPARENT,
            outline_width: 0.0,
            vertical_align: VerticalAlign::Top,
            family: FontFamily::Sans,
        }
    }
}

impl TextStyle {
    pub const SIZE: usize = 28;

    /// Serialize to 28 bytes:
    /// [size: u32][color: u32][max_width: u32][flags: u32][outline_color: u32][outline_width: f32][flags2: u32]
    /// flags bits:
    ///   0-11:  weight (0-4095)
    ///   12-23: line_height × 100
    ///   24:    italic
    ///   25:    underline
    ///   26:    strikethrough
    ///   27-28: align
    ///   29-30: text_overflow (0=wrap, 1=clip, 2=ellipsis)
    /// flags2 bits:
    ///   0-1:   vertical_align (0=top, 1=center, 2=bottom, 3=baseline)
    ///   2-3:   family (0=sans, 1=deck-sans)
    ///   4-31:  reserved
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0_u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.size.to_le_bytes());
        buf[4..8].copy_from_slice(&self.color.to_u32().to_le_bytes());
        buf[8..12].copy_from_slice(&self.max_width.to_le_bytes());

        let weight_bits = u32::from(self.weight.0) & 0xFFF;
        // line_height scaled to 12-bit fixed point, clamped to valid range
        let lh_scaled = (self.line_height * 100.0).clamp(0.0, 4095.0);
        // SAFETY: clamped to [0, 4095], fits in u32 without truncation or sign loss
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lh_bits = (lh_scaled as u32) << 12;
        let italic_bit = if self.italic { 1 << 24 } else { 0 };
        let underline_bit = if self.underline { 1 << 25 } else { 0 };
        let strike_bit = if self.strikethrough { 1 << 26 } else { 0 };
        let align_bits = (self.align as u32 & 0x3) << 27;
        let overflow_bits = (self.text_overflow as u32 & 0x3) << 29;

        let flags = weight_bits
            | lh_bits
            | italic_bit
            | underline_bit
            | strike_bit
            | align_bits
            | overflow_bits;
        let flags2 = (self.vertical_align as u32 & 0x3) | ((self.family as u32 & 0x3) << 2);
        buf[12..16].copy_from_slice(&flags.to_le_bytes());
        buf[16..20].copy_from_slice(&self.outline_color.to_u32().to_le_bytes());
        buf[20..24].copy_from_slice(&self.outline_width.to_le_bytes());
        buf[24..28].copy_from_slice(&flags2.to_le_bytes());
        buf
    }

    /// Deserialize from 28 bytes
    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Self {
        let size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let color = Color::from_raw(u32::from_le_bytes([data[4], data[5], data[6], data[7]]));
        let max_width = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let flags = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        let outline_color =
            Color::from_raw(u32::from_le_bytes([data[16], data[17], data[18], data[19]]));
        let outline_width = f32::from_le_bytes([data[20], data[21], data[22], data[23]]);
        let flags2 = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);

        let weight = FontWeight((flags & 0xFFF) as u16);
        // 12-bit value (max 4095) fits exactly in f32 mantissa (23 bits), no precision loss
        let lh_fixed = (flags >> 12) & 0xFFF;
        #[expect(clippy::cast_precision_loss)]
        let line_height = (lh_fixed as f32) / 100.0;
        let italic = (flags >> 24) & 1 != 0;
        let underline = (flags >> 25) & 1 != 0;
        let strikethrough = (flags >> 26) & 1 != 0;
        let align = match (flags >> 27) & 0x3 {
            1 => TextAlign::Center,
            2 => TextAlign::Right,
            _ => TextAlign::Left,
        };
        let text_overflow = match (flags >> 29) & 0x3 {
            1 => TextOverflow::Clip,
            2 => TextOverflow::Ellipsis,
            _ => TextOverflow::Wrap,
        };
        let vertical_align = match flags2 & 0x3 {
            1 => VerticalAlign::Center,
            2 => VerticalAlign::Bottom,
            3 => VerticalAlign::Baseline,
            _ => VerticalAlign::Top,
        };
        let family = match (flags2 >> 2) & 0x3 {
            1 => FontFamily::DeckSans,
            _ => FontFamily::Sans,
        };

        Self {
            size,
            color,
            max_width,
            weight,
            italic,
            underline,
            strikethrough,
            line_height,
            align,
            text_overflow,
            outline_color,
            outline_width,
            vertical_align,
            family,
        }
    }
}

/// Fixed-size props structure (66 bytes)
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct PropsData {
    pub padding: f32,
    pub margin: f32,
    pub gap: f32,
    pub background: Color,
    pub width: f32,
    pub height: f32,
    pub flex: f32,
    pub max_width: f32,
    pub max_height: f32,
    pub cross_align: CrossAlign,
    /// Enable flex wrapping: children wrap to the next line when they exceed
    /// the container's main-axis size. Equivalent to CSS `flex-wrap: wrap`.
    pub wrap: bool,
    /// Nine-patch background image. `None` means no background image.
    pub bg_np_id: Option<BitmapId>,
    pub bg_np_left: u16,
    pub bg_np_top: u16,
    pub bg_np_right: u16,
    pub bg_np_bottom: u16,
    /// Absolute positioning insets. `NAN` means "auto" (unset).
    /// Setting any inset to a finite value makes the node absolutely positioned.
    pub inset_top: f32,
    pub inset_right: f32,
    pub inset_bottom: f32,
    pub inset_left: f32,
}

impl Default for PropsData {
    fn default() -> Self {
        Self {
            padding: 0.0,
            margin: 0.0,
            gap: 0.0,
            background: TRANSPARENT,
            width: 0.0,
            height: 0.0,
            flex: 0.0,
            max_width: 0.0,
            max_height: 0.0,
            cross_align: CrossAlign::Stretch,
            wrap: false,
            bg_np_id: None,
            bg_np_left: 0,
            bg_np_top: 0,
            bg_np_right: 0,
            bg_np_bottom: 0,
            inset_top: f32::NAN,
            inset_right: f32::NAN,
            inset_bottom: f32::NAN,
            inset_left: f32::NAN,
        }
    }
}

impl PropsData {
    pub const SIZE: usize = 66;

    /// Returns `true` if any inset is set (finite), meaning this node is absolutely positioned.
    #[must_use]
    pub fn is_absolute(&self) -> bool {
        self.inset_top.is_finite()
            || self.inset_right.is_finite()
            || self.inset_bottom.is_finite()
            || self.inset_left.is_finite()
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0_u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.padding.to_le_bytes());
        buf[4..8].copy_from_slice(&self.margin.to_le_bytes());
        buf[8..12].copy_from_slice(&self.gap.to_le_bytes());
        buf[12..16].copy_from_slice(&self.background.to_u32().to_le_bytes());
        buf[16..20].copy_from_slice(&self.width.to_le_bytes());
        buf[20..24].copy_from_slice(&self.height.to_le_bytes());
        buf[24..28].copy_from_slice(&self.flex.to_le_bytes());
        buf[28..32].copy_from_slice(&self.max_width.to_le_bytes());
        buf[32..36].copy_from_slice(&self.max_height.to_le_bytes());
        let layout_flags = LayoutFlags::new(self.cross_align, self.wrap);
        buf[36..40].copy_from_slice(&layout_flags.bits().to_le_bytes());
        buf[40..42].copy_from_slice(&self.bg_np_id.map_or(0, BitmapId::to_wire).to_le_bytes());
        buf[42..44].copy_from_slice(&self.bg_np_left.to_le_bytes());
        buf[44..46].copy_from_slice(&self.bg_np_top.to_le_bytes());
        buf[46..48].copy_from_slice(&self.bg_np_right.to_le_bytes());
        buf[48..50].copy_from_slice(&self.bg_np_bottom.to_le_bytes());
        buf[50..54].copy_from_slice(&self.inset_top.to_le_bytes());
        buf[54..58].copy_from_slice(&self.inset_right.to_le_bytes());
        buf[58..62].copy_from_slice(&self.inset_bottom.to_le_bytes());
        buf[62..66].copy_from_slice(&self.inset_left.to_le_bytes());
        buf
    }

    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Self {
        Self {
            padding: f32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            margin: f32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            gap: f32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            background: Color::from_raw(u32::from_le_bytes([
                data[12], data[13], data[14], data[15],
            ])),
            width: f32::from_le_bytes([data[16], data[17], data[18], data[19]]),
            height: f32::from_le_bytes([data[20], data[21], data[22], data[23]]),
            flex: f32::from_le_bytes([data[24], data[25], data[26], data[27]]),
            max_width: f32::from_le_bytes([data[28], data[29], data[30], data[31]]),
            max_height: f32::from_le_bytes([data[32], data[33], data[34], data[35]]),
            cross_align: LayoutFlags::from_bits(u32::from_le_bytes([
                data[36], data[37], data[38], data[39],
            ]))
            .cross_align(),
            wrap: LayoutFlags::from_bits(u32::from_le_bytes([
                data[36], data[37], data[38], data[39],
            ]))
            .wrap(),
            bg_np_id: BitmapId::from_wire(u16::from_le_bytes([data[40], data[41]])),
            bg_np_left: u16::from_le_bytes([data[42], data[43]]),
            bg_np_top: u16::from_le_bytes([data[44], data[45]]),
            bg_np_right: u16::from_le_bytes([data[46], data[47]]),
            bg_np_bottom: u16::from_le_bytes([data[48], data[49]]),
            inset_top: f32::from_le_bytes([data[50], data[51], data[52], data[53]]),
            inset_right: f32::from_le_bytes([data[54], data[55], data[56], data[57]]),
            inset_bottom: f32::from_le_bytes([data[58], data[59], data[60], data[61]]),
            inset_left: f32::from_le_bytes([data[62], data[63], data[64], data[65]]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_anchor_decodes_known_wire_values() {
        assert_eq!(ArcAnchor::try_from(0), Ok(ArcAnchor::Start));
        assert_eq!(ArcAnchor::try_from(1), Ok(ArcAnchor::Center));
        assert_eq!(ArcAnchor::try_from(2), Ok(ArcAnchor::End));
        assert_eq!(ArcAnchor::try_from(3), Err(InvalidArcAnchor(3)));
    }

    #[test]
    fn arc_anchor_encodes_known_wire_values() {
        assert_eq!(u8::from(ArcAnchor::Start), 0);
        assert_eq!(u8::from(ArcAnchor::Center), 1);
        assert_eq!(u8::from(ArcAnchor::End), 2);
    }

    #[test]
    fn arc_text_facing_decodes_known_wire_values() {
        assert_eq!(ArcTextFacing::try_from(0), Ok(ArcTextFacing::Outward));
        assert_eq!(ArcTextFacing::try_from(1), Ok(ArcTextFacing::Inward));
        assert_eq!(ArcTextFacing::try_from(2), Err(InvalidArcTextFacing(2)));
    }

    #[test]
    fn arc_text_facing_encodes_known_wire_values() {
        assert_eq!(u8::from(ArcTextFacing::Outward), 0);
        assert_eq!(u8::from(ArcTextFacing::Inward), 1);
    }

    #[test]
    fn text_style_round_trips_default() {
        let s = TextStyle::default();
        let bytes = s.to_bytes();
        let back = TextStyle::from_bytes(&bytes);
        assert_eq!(back.vertical_align, VerticalAlign::Top);
        assert_eq!(back.align, TextAlign::Left);
        assert_eq!(back.family, FontFamily::Sans);
    }

    #[test]
    fn text_style_round_trips_every_family() {
        for variant in [FontFamily::Sans, FontFamily::DeckSans] {
            let s = TextStyle {
                family: variant,
                ..TextStyle::default()
            };
            let bytes = s.to_bytes();
            let back = TextStyle::from_bytes(&bytes);
            assert_eq!(back.family, variant);
        }
    }

    #[test]
    fn text_style_family_independent_of_vertical_align() {
        let s = TextStyle {
            family: FontFamily::DeckSans,
            vertical_align: VerticalAlign::Baseline,
            ..TextStyle::default()
        };
        let back = TextStyle::from_bytes(&s.to_bytes());
        assert_eq!(back.family, FontFamily::DeckSans);
        assert_eq!(back.vertical_align, VerticalAlign::Baseline);
    }

    #[test]
    fn text_style_round_trips_every_vertical_align() {
        for variant in [
            VerticalAlign::Top,
            VerticalAlign::Center,
            VerticalAlign::Bottom,
            VerticalAlign::Baseline,
        ] {
            let s = TextStyle {
                vertical_align: variant,
                ..TextStyle::default()
            };
            let bytes = s.to_bytes();
            let back = TextStyle::from_bytes(&bytes);
            assert_eq!(back.vertical_align, variant);
        }
    }

    #[test]
    fn text_style_vertical_align_independent_of_other_fields() {
        let s = TextStyle {
            size: 32,
            weight: FontWeight::BOLD,
            italic: true,
            underline: true,
            strikethrough: true,
            line_height: 1.6,
            align: TextAlign::Right,
            text_overflow: TextOverflow::Ellipsis,
            vertical_align: VerticalAlign::Baseline,
            ..TextStyle::default()
        };
        let back = TextStyle::from_bytes(&s.to_bytes());
        assert_eq!(back.size, 32);
        assert_eq!(back.weight, FontWeight::BOLD);
        assert!(back.italic);
        assert!(back.underline);
        assert!(back.strikethrough);
        assert!((back.line_height - 1.6).abs() < 0.01);
        assert_eq!(back.align, TextAlign::Right);
        assert_eq!(back.text_overflow, TextOverflow::Ellipsis);
        assert_eq!(back.vertical_align, VerticalAlign::Baseline);
    }
}
