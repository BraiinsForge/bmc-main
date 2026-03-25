// Copyright (C) 2026  Braiins Systems s.r.o.

//! Text styling types shared between SDK and host.

use crate::colors::{Color, GRAY_10, TRANSPARENT};
use crate::ids::BitmapId;

/// Text alignment
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TextAlign {
    #[default]
    Left = 0,
    Center = 1,
    Right = 2,
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

/// Text style for paragraphs (24 bytes serialized)
#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub size: u32,      // default: 16
    pub color: Color,   // default: GRAY_10
    pub max_width: u32, // default: 0 (use container width)
    pub weight: u16,    // default: 400, 700 = bold
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub line_height: f32, // default: 1.4 (multiplier)
    pub align: TextAlign,
    pub text_overflow: TextOverflow, // default: Wrap
    pub outline_color: Color,        // default: TRANSPARENT (no outline)
    pub outline_width: f32,          // default: 0.0 (no outline)
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 16,
            color: GRAY_10,
            max_width: 0,
            weight: 400,
            italic: false,
            underline: false,
            strikethrough: false,
            line_height: 1.4,
            align: TextAlign::Left,
            text_overflow: TextOverflow::Wrap,
            outline_color: TRANSPARENT,
            outline_width: 0.0,
        }
    }
}

impl TextStyle {
    pub const SIZE: usize = 24;

    /// Serialize to 24 bytes:
    /// [size: u32][color: u32][max_width: u32][flags: u32][outline_color: u32][outline_width: f32]
    /// flags bits:
    ///   0-11:  weight (0-4095)
    ///   12-23: line_height × 100
    ///   24:    italic
    ///   25:    underline
    ///   26:    strikethrough
    ///   27-28: align
    ///   29-30: text_overflow (0=wrap, 1=clip, 2=ellipsis)
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0_u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.size.to_le_bytes());
        buf[4..8].copy_from_slice(&self.color.to_u32().to_le_bytes());
        buf[8..12].copy_from_slice(&self.max_width.to_le_bytes());

        let weight_bits = u32::from(self.weight) & 0xFFF;
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
        buf[12..16].copy_from_slice(&flags.to_le_bytes());
        buf[16..20].copy_from_slice(&self.outline_color.to_u32().to_le_bytes());
        buf[20..24].copy_from_slice(&self.outline_width.to_le_bytes());
        buf
    }

    /// Deserialize from 24 bytes
    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Self {
        let size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let color = Color::from_raw(u32::from_le_bytes([data[4], data[5], data[6], data[7]]));
        let max_width = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let flags = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        let outline_color =
            Color::from_raw(u32::from_le_bytes([data[16], data[17], data[18], data[19]]));
        let outline_width = f32::from_le_bytes([data[20], data[21], data[22], data[23]]);

        let weight = (flags & 0xFFF) as u16;
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
    /// Nine-patch background image. `bitmap_id == NONE` means none.
    pub bg_np_id: BitmapId,
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
            bg_np_id: BitmapId::NONE,
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
        buf[40..42].copy_from_slice(&self.bg_np_id.raw().to_le_bytes());
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
            bg_np_id: BitmapId::from_raw(u16::from_le_bytes([data[40], data[41]])),
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
