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

//! Text styling types shared between SDK and host.

use core::fmt;

use crate::colors::{Color, GRAY_10, TRANSPARENT};
use crate::ids::BitmapId;
use crate::wire;

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

/// Main-axis distribution of a container's children — CSS `justify-content`.
///
/// - Row: controls horizontal placement of children
/// - Column: controls vertical placement of children
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Justify {
    /// Children pack to the main-axis start (default; CSS `flex-start`).
    #[default]
    Start = 0,
    /// Children are centered along the main axis.
    Center = 1,
    /// Children are packed to the end of the main axis.
    End = 2,
    /// First and last children pin to the edges, the rest spread evenly.
    SpaceBetween = 3,
}

/// Packed layout flags occupying 4 bytes (offset 36–40) in `PropsData` wire
/// format. Bit allocation is documented here so future flags add a named
/// constant + accessor instead of reaching into the bits ad-hoc.
///
/// | bits      | meaning                              |
/// |-----------|--------------------------------------|
/// | `0..8`    | `CrossAlign` discriminant            |
/// | `8`       | `wrap` (bool)                        |
/// | `9..17`   | `Justify` discriminant               |
/// | `17..32`  | reserved — must remain zero          |
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutFlags(u32);

impl LayoutFlags {
    const CROSS_ALIGN_MASK: u32 = 0xFF;
    const FLAG_WRAP: u32 = 1 << 8;
    const JUSTIFY_SHIFT: u32 = 9;
    const JUSTIFY_MASK: u32 = 0xFF << Self::JUSTIFY_SHIFT;

    #[must_use]
    pub fn new(cross_align: CrossAlign, wrap: bool, justify_content: Justify) -> Self {
        let mut bits = (cross_align as u32) & Self::CROSS_ALIGN_MASK;
        if wrap {
            bits |= Self::FLAG_WRAP;
        }
        bits |= (justify_content as u32) << Self::JUSTIFY_SHIFT;
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

    #[must_use]
    pub fn justify_content(self) -> Justify {
        match (self.0 & Self::JUSTIFY_MASK) >> Self::JUSTIFY_SHIFT {
            1 => Justify::Center,
            2 => Justify::End,
            3 => Justify::SpaceBetween,
            _ => Justify::Start,
        }
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

/// How an autofit text draw command scales its font size within its box.
///
/// This is a draw-command parameter (see `DRAW_AUTOFIT_TEXT`), not a
/// `TextStyle` field. `Shrink` searches `[min_size, size]`, `Grow` searches
/// `[size, max_size]`, `ShrinkAndGrow` searches `[min_size, max_size]`
/// (configured `size` is then only a starting hint).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AutoFit {
    /// Only shrink to fit (default).
    #[default]
    Shrink = 0,
    /// Only grow to fill.
    Grow = 1,
    /// Shrink or grow to fit.
    ShrinkAndGrow = 2,
}

impl AutoFit {
    /// Decode a wire byte, defaulting unknown values to `Shrink`.
    #[must_use]
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Grow,
            2 => Self::ShrinkAndGrow,
            _ => Self::Shrink,
        }
    }
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

        let mut buf = [0_u8; Self::SIZE];
        let mut p = 0;
        wire::write_u32(&mut buf, &mut p, self.size);
        wire::write_color(&mut buf, &mut p, self.color);
        wire::write_u32(&mut buf, &mut p, self.max_width);
        wire::write_u32(&mut buf, &mut p, flags);
        wire::write_color(&mut buf, &mut p, self.outline_color);
        wire::write_f32(&mut buf, &mut p, self.outline_width);
        wire::write_u32(&mut buf, &mut p, flags2);
        buf
    }

    /// Deserialize from 28 bytes
    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let mut p = 0;
        let size = wire::read_u32(data, &mut p)?;
        let color = wire::read_color(data, &mut p)?;
        let max_width = wire::read_u32(data, &mut p)?;
        let flags = wire::read_u32(data, &mut p)?;
        let outline_color = wire::read_color(data, &mut p)?;
        let outline_width = wire::read_f32(data, &mut p)?;
        let flags2 = wire::read_u32(data, &mut p)?;

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

        Some(Self {
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
        })
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
    /// CSS `justify-content`: main-axis distribution of children.
    pub justify_content: Justify,
    /// Enable flex wrapping: children wrap to the next line when they exceed
    /// the container's main-axis size. Equivalent to CSS `flex-wrap: wrap`.
    pub wrap: bool,
    /// Nine-patch background image. `None` means no background image.
    pub bg_np_id: Option<BitmapId>,
    pub bg_np_left: u16,
    pub bg_np_top: u16,
    pub bg_np_right: u16,
    pub bg_np_bottom: u16,
    /// Absolute positioning insets.
    /// `NAN` means "auto" (unset).
    /// Setting any inset to a finite value makes the node absolutely positioned.
    pub inset_top: f32,
    pub inset_right: f32,
    pub inset_bottom: f32,
    pub inset_left: f32,
    /// CSS `border-radius`: rounds the node's box — the `background` fill and the border alike.
    /// `0.0` paints square corners.
    /// Nine-patch backgrounds carry their rounding in the bitmap and ignore it.
    pub border_radius: f32,
    /// CSS `border-width`: `0.0` paints no border.
    pub border_width: f32,
    /// CSS `border-color`.
    pub border_color: Color,
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
            justify_content: Justify::Start,
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
            border_radius: 0.0,
            border_width: 0.0,
            border_color: TRANSPARENT,
        }
    }
}

impl PropsData {
    pub const SIZE: usize = 78;

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
        let mut p = 0;
        wire::write_f32(&mut buf, &mut p, self.padding);
        wire::write_f32(&mut buf, &mut p, self.margin);
        wire::write_f32(&mut buf, &mut p, self.gap);
        wire::write_color(&mut buf, &mut p, self.background);
        wire::write_f32(&mut buf, &mut p, self.width);
        wire::write_f32(&mut buf, &mut p, self.height);
        wire::write_f32(&mut buf, &mut p, self.flex);
        wire::write_f32(&mut buf, &mut p, self.max_width);
        wire::write_f32(&mut buf, &mut p, self.max_height);
        wire::write_u32(
            &mut buf,
            &mut p,
            LayoutFlags::new(self.cross_align, self.wrap, self.justify_content).bits(),
        );
        wire::write_u16(&mut buf, &mut p, self.bg_np_id.map_or(0, BitmapId::to_wire));
        wire::write_u16(&mut buf, &mut p, self.bg_np_left);
        wire::write_u16(&mut buf, &mut p, self.bg_np_top);
        wire::write_u16(&mut buf, &mut p, self.bg_np_right);
        wire::write_u16(&mut buf, &mut p, self.bg_np_bottom);
        wire::write_f32(&mut buf, &mut p, self.inset_top);
        wire::write_f32(&mut buf, &mut p, self.inset_right);
        wire::write_f32(&mut buf, &mut p, self.inset_bottom);
        wire::write_f32(&mut buf, &mut p, self.inset_left);
        wire::write_f32(&mut buf, &mut p, self.border_radius);
        wire::write_f32(&mut buf, &mut p, self.border_width);
        wire::write_color(&mut buf, &mut p, self.border_color);
        buf
    }

    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let mut p = 0;
        let padding = wire::read_f32(data, &mut p)?;
        let margin = wire::read_f32(data, &mut p)?;
        let gap = wire::read_f32(data, &mut p)?;
        let background = wire::read_color(data, &mut p)?;
        let width = wire::read_f32(data, &mut p)?;
        let height = wire::read_f32(data, &mut p)?;
        let flex = wire::read_f32(data, &mut p)?;
        let max_width = wire::read_f32(data, &mut p)?;
        let max_height = wire::read_f32(data, &mut p)?;
        let layout = LayoutFlags::from_bits(wire::read_u32(data, &mut p)?);
        let bg_np_id = BitmapId::from_wire(wire::read_u16(data, &mut p)?);
        let bg_np_left = wire::read_u16(data, &mut p)?;
        let bg_np_top = wire::read_u16(data, &mut p)?;
        let bg_np_right = wire::read_u16(data, &mut p)?;
        let bg_np_bottom = wire::read_u16(data, &mut p)?;
        let inset_top = wire::read_f32(data, &mut p)?;
        let inset_right = wire::read_f32(data, &mut p)?;
        let inset_bottom = wire::read_f32(data, &mut p)?;
        let inset_left = wire::read_f32(data, &mut p)?;
        let border_radius = wire::read_f32(data, &mut p)?;
        let border_width = wire::read_f32(data, &mut p)?;
        let border_color = wire::read_color(data, &mut p)?;
        Some(Self {
            padding,
            margin,
            gap,
            background,
            width,
            height,
            flex,
            max_width,
            max_height,
            cross_align: layout.cross_align(),
            justify_content: layout.justify_content(),
            wrap: layout.wrap(),
            bg_np_id,
            bg_np_left,
            bg_np_top,
            bg_np_right,
            bg_np_bottom,
            inset_top,
            inset_right,
            inset_bottom,
            inset_left,
            border_radius,
            border_width,
            border_color,
        })
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
    fn props_data_round_trips() {
        let props = PropsData {
            padding: 1.0,
            margin: 2.0,
            gap: 3.0,
            background: GRAY_10,
            width: 4.0,
            height: 5.0,
            flex: 6.0,
            max_width: 7.0,
            max_height: 8.0,
            cross_align: CrossAlign::Center,
            justify_content: Justify::SpaceBetween,
            wrap: true,
            bg_np_id: BitmapId::from_wire(42),
            bg_np_left: 10,
            bg_np_top: 11,
            bg_np_right: 12,
            bg_np_bottom: 13,
            inset_top: 14.0,
            inset_right: 15.0,
            inset_bottom: 16.0,
            inset_left: 17.0,
            border_radius: 18.0,
            border_width: 19.0,
            border_color: GRAY_10,
        };
        let bytes = props.to_bytes();
        let back = PropsData::from_bytes(&bytes).expect("full-size buffer decodes");
        // Re-encoding the decoded props reproduces the bytes exactly.
        assert_eq!(back.to_bytes(), bytes);
        assert_eq!(back.bg_np_id, BitmapId::from_wire(42));
        assert_eq!(back.cross_align, CrossAlign::Center);
        assert!(back.wrap);
        assert_eq!(back.bg_np_bottom, 13);
        // Border floats round-trip exactly via the byte equality above;
        // spot-check only the color to avoid strict float comparison.
        assert_eq!(back.border_color, GRAY_10);
    }

    #[test]
    fn props_data_from_bytes_rejects_truncated() {
        assert!(PropsData::from_bytes(&[0_u8; PropsData::SIZE - 1]).is_none());
    }

    #[test]
    fn text_style_round_trips_default() {
        let s = TextStyle::default();
        let bytes = s.to_bytes();
        let back = TextStyle::from_bytes(&bytes).expect("full-size buffer decodes");
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
            let back = TextStyle::from_bytes(&bytes).expect("full-size buffer decodes");
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
        let back = TextStyle::from_bytes(&s.to_bytes()).expect("full-size buffer decodes");
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
            let back = TextStyle::from_bytes(&bytes).expect("full-size buffer decodes");
            assert_eq!(back.vertical_align, variant);
        }
    }

    #[test]
    fn auto_fit_round_trips_every_variant() {
        for variant in [AutoFit::Shrink, AutoFit::Grow, AutoFit::ShrinkAndGrow] {
            assert_eq!(AutoFit::from_u8(variant as u8), variant);
        }
    }

    #[test]
    fn auto_fit_unknown_byte_falls_back_to_shrink() {
        assert_eq!(AutoFit::from_u8(255), AutoFit::Shrink);
        assert_eq!(AutoFit::default(), AutoFit::Shrink);
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
        let back = TextStyle::from_bytes(&s.to_bytes()).expect("full-size buffer decodes");
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
