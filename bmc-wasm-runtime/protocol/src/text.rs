// Copyright (C) 2026  Braiins Systems s.r.o.

//! Text styling types shared between SDK and host.

use crate::GRAY_10;

/// Text alignment
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TextAlign {
    #[default]
    Left = 0,
    Center = 1,
    Right = 2,
}

/// Text style for paragraphs (16 bytes serialized)
#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub size: u32,      // default: 16
    pub color: u32,     // default: GRAY_10
    pub max_width: u32, // default: 0 (use container width)
    pub weight: u16,    // default: 400, 700 = bold
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub line_height: f32, // default: 1.4 (multiplier)
    pub align: TextAlign,
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
        }
    }
}

impl TextStyle {
    pub const SIZE: usize = 16;

    /// Serialize to 16 bytes:
    /// [size: u32][color: u32][max_width: u32][flags: u32]
    /// flags bits:
    ///   0-11:  weight (0-4095)
    ///   12-23: line_height × 100
    ///   24:    italic
    ///   25:    underline
    ///   26:    strikethrough
    ///   27-28: align
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0_u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.size.to_le_bytes());
        buf[4..8].copy_from_slice(&self.color.to_le_bytes());
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

        let flags = weight_bits | lh_bits | italic_bit | underline_bit | strike_bit | align_bits;
        buf[12..16].copy_from_slice(&flags.to_le_bytes());
        buf
    }

    /// Deserialize from 16 bytes
    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Self {
        let size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let color = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let max_width = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let flags = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);

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
        }
    }
}

/// Fixed-size props structure (32 bytes)
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct PropsData {
    pub padding: f32,
    pub margin: f32,
    pub gap: f32,
    pub background: u32,
    pub width: f32,
    pub height: f32,
    pub flex: f32,
    pub color: u32,
}

impl PropsData {
    pub const SIZE: usize = 32;

    #[must_use]
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0_u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.padding.to_le_bytes());
        buf[4..8].copy_from_slice(&self.margin.to_le_bytes());
        buf[8..12].copy_from_slice(&self.gap.to_le_bytes());
        buf[12..16].copy_from_slice(&self.background.to_le_bytes());
        buf[16..20].copy_from_slice(&self.width.to_le_bytes());
        buf[20..24].copy_from_slice(&self.height.to_le_bytes());
        buf[24..28].copy_from_slice(&self.flex.to_le_bytes());
        buf[28..32].copy_from_slice(&self.color.to_le_bytes());
        buf
    }

    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Self {
        Self {
            padding: f32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            margin: f32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            gap: f32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            background: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
            width: f32::from_le_bytes([data[16], data[17], data[18], data[19]]),
            height: f32::from_le_bytes([data[20], data[21], data[22], data[23]]),
            flex: f32::from_le_bytes([data[24], data[25], data[26], data[27]]),
            color: u32::from_le_bytes([data[28], data[29], data[30], data[31]]),
        }
    }
}
