// Copyright (C) 2026  Braiins Systems s.r.o.

//! Color constants for the design system.

// Gray scale
pub const GRAY_10: u32 = 0xF4F4_F4FF;
pub const GRAY_20: u32 = 0xE0E0_E0FF;
pub const GRAY_30: u32 = 0xC6C6_C6FF;
pub const GRAY_40: u32 = 0xA8A8_A8FF;
pub const GRAY_50: u32 = 0x8D8D_8DFF;
pub const GRAY_60: u32 = 0x6F6F_6FFF;
pub const GRAY_70: u32 = 0x5252_52FF;
pub const GRAY_80: u32 = 0x3939_39FF;
pub const GRAY_90: u32 = 0x2626_26FF;
pub const GRAY_100: u32 = 0x1616_16FF;

// Violet (primary brand color)
pub const VIOLET_10: u32 = 0xF3F2_FFFF;
pub const VIOLET_20: u32 = 0xDFDC_FFFF;
pub const VIOLET_30: u32 = 0xC5C0_FFFF;
pub const VIOLET_40: u32 = 0xA69D_FFFF;
pub const VIOLET_50: u32 = 0x8B7C_FFFF;
pub const VIOLET_60: u32 = 0x6B50_FFFF;
pub const VIOLET_70: u32 = 0x5432_CDFF;
pub const VIOLET_80: u32 = 0x3923_8FFF;
pub const VIOLET_90: u32 = 0x2816_61FF;
pub const VIOLET_100: u32 = 0x170D_3AFF;

// Green (success)
pub const GREEN_10: u32 = 0xDDFB_E9FF;
pub const GREEN_20: u32 = 0xA3F1_B9FF;
pub const GREEN_30: u32 = 0x5ADF_88FF;
pub const GREEN_40: u32 = 0x34C0_6AFF;
pub const GREEN_50: u32 = 0x13A4_54FF;
pub const GREEN_60: u32 = 0x1680_42FF;
pub const GREEN_70: u32 = 0x195E_33FF;
pub const GREEN_80: u32 = 0x1242_23FF;
pub const GREEN_90: u32 = 0x102B_19FF;
pub const GREEN_100: u32 = 0x0619_12FF;

// Red (error/danger)
pub const RED_10: u32 = 0xFFF1_F2FF;
pub const RED_20: u32 = 0xFFD6_D5FF;
pub const RED_30: u32 = 0xFFB3_B2FF;
pub const RED_40: u32 = 0xFF83_84FF;
pub const RED_50: u32 = 0xF953_55FF;
pub const RED_60: u32 = 0xD922_2CFF;
pub const RED_70: u32 = 0xA217_1FFF;
pub const RED_80: u32 = 0x740E_14FF;
pub const RED_90: u32 = 0x4F09_0DFF;
pub const RED_100: u32 = 0x2B0B_0BFF;

// Orange (warning)
pub const ORANGE_10: u32 = 0xFFF1_E9FF;
pub const ORANGE_20: u32 = 0xFFD8_BFFF;
pub const ORANGE_30: u32 = 0xFFB6_87FF;
pub const ORANGE_40: u32 = 0xFE84_31FF;
pub const ORANGE_50: u32 = 0xEB63_07FF;
pub const ORANGE_60: u32 = 0xC148_12FF;
pub const ORANGE_70: u32 = 0x9332_00FF;
pub const ORANGE_80: u32 = 0x6426_00FF;
pub const ORANGE_90: u32 = 0x421B_00FF;
pub const ORANGE_100: u32 = 0x2512_00FF;

// Absolute
pub const BLACK: u32 = 0x0000_00FF;
pub const WHITE: u32 = 0xFFFF_FFFF;

// Special
pub const TRANSPARENT: u32 = 0x0000_0000;

/// Color utility macro.
///
/// - `color!(GRAY_80, alpha: 0.5)` — adjust alpha (0.0–1.0)
/// - `color!(GRAY_80, lightness: 0.5)` — scale RGB brightness (0.0 = black, 1.0 = unchanged)
/// - `color!(GRAY_80, alpha: 0.5, lightness: 0.8)` — both
#[macro_export]
macro_rules! color {
    ($base:expr, alpha: $a:expr) => {
        ($base & 0xFFFF_FF00) | (($a * 255.0) as u32 & 0xFF)
    };
    ($base:expr, lightness: $l:expr) => {{
        let r = ((($base >> 24) & 0xFF) as f32 * $l).min(255.0) as u32;
        let g = ((($base >> 16) & 0xFF) as f32 * $l).min(255.0) as u32;
        let b = ((($base >> 8) & 0xFF) as f32 * $l).min(255.0) as u32;
        let a = $base & 0xFF;
        (r << 24) | (g << 16) | (b << 8) | a
    }};
    ($base:expr, alpha: $a:expr, lightness: $l:expr) => {{
        let r = ((($base >> 24) & 0xFF) as f32 * $l).min(255.0) as u32;
        let g = ((($base >> 16) & 0xFF) as f32 * $l).min(255.0) as u32;
        let b = ((($base >> 8) & 0xFF) as f32 * $l).min(255.0) as u32;
        let a = ($a * 255.0) as u32 & 0xFF;
        (r << 24) | (g << 16) | (b << 8) | a
    }};
}
