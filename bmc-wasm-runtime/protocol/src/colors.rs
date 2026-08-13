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

//! Color constants and perceptual color manipulation for the design system.
//!
//! **All color math in this module uses OkLCH** (via the `palette` crate).
//! This is deliberate — naive RGB operations produce perceptually uneven
//! results: mid-gray blends shift toward blue, brightness scaling distorts
//! saturation, and hue interpolation takes wrong-way arcs through muddy
//! tones. OkLCH avoids all of this by operating in a perceptually uniform
//! space where lightness, chroma, and hue behave as humans expect.
//!
//! If you add new color operations here, keep them in OkLCH. The only
//! exceptions are trivial channel extractions (`alpha`, `brightness` as a
//! raw RGB multiply) that are clearly labeled as non-perceptual.
//!
//! Use `palette` trait methods (`Mix`, `Lighten`, `ShiftHue`, etc.) instead
//! of hand-rolling math. Don't reimplement what upstream already provides.

// ── Color type ──────────────────────────────────────────────────────

/// Zero-cost newtype wrapper around a packed `0xRRGGBBAA` color.
///
/// Provides type-safe color construction, component extraction, and
/// perceptual color manipulation via method chaining.
///
/// # Constructors
///
/// ```ignore
/// Color::from_hex(0xF4_F4_F4)           // opaque, alpha = 0xFF
/// Color::from_rgb(244, 244, 244)        // opaque, alpha = 0xFF
/// Color::from_rgba(244, 244, 244, 128)  // explicit alpha
/// ```
///
/// # Chaining
///
/// ```ignore
/// BLUE_50.lightness(0.3).chroma(0.06)   // perceptual (OkLCH)
/// GRAY_10.brightness(0.5).with_alpha(0.8) // simple (RGB multiply + alpha)
/// RED_50.mix(GRAY_90, 0.3)              // perceptual blend
/// ```
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Color(u32);

impl core::fmt::Debug for Color {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Color(#{:08X})", self.0)
    }
}

impl Color {
    // ── Constructors ────────────────────────────────────────────────

    /// Create from a 24-bit hex value (`0xRRGGBB`), alpha defaults to 0xFF.
    ///
    /// Bits above the low 24 are masked off — passing a value outside
    /// `0x00_00_00..=0xFF_FF_FF` is silently truncated rather than panicking
    /// on the shift overflow.
    #[must_use]
    pub const fn from_hex(hex: u32) -> Self {
        Self(((hex & 0x00FF_FFFF) << 8) | 0xFF)
    }

    /// Create from RGBA components.
    #[must_use]
    pub const fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self((r as u32) << 24 | (g as u32) << 16 | (b as u32) << 8 | a as u32)
    }

    /// Create an opaque color from RGB components (alpha = 0xFF).
    #[must_use]
    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self::from_rgba(r, g, b, 0xFF)
    }

    /// Create an opaque color from HSV (hue 0–360, saturation 0–1, value 0–1).
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "RGB channels are clamped to 0–255"
    )]
    pub fn from_hsv(h: f32, s: f32, v: f32) -> Self {
        use palette::{FromColor, Hsv, Srgb};
        debug_assert!(
            h.is_finite() && s.is_finite() && v.is_finite(),
            "from_hsv: non-finite input h={h} s={s} v={v}"
        );
        let rgb = Srgb::from_color(Hsv::new(h, s, v));
        Self::from_rgb(
            (rgb.red * 255.0) as u8,
            (rgb.green * 255.0) as u8,
            (rgb.blue * 255.0) as u8,
        )
    }

    /// Create from a raw packed `0xRRGGBBAA` value.
    ///
    /// Prefer [`from_hex`](Self::from_hex) or [`from_rgba`](Self::from_rgba)
    /// for new code. This exists for deserialization and wire-format interop.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    // ── Getters ─────────────────────────────────────────────────────

    /// Red channel (0–255).
    #[must_use]
    pub const fn red(self) -> u8 {
        (self.0 >> 24) as u8
    }

    /// Green channel (0–255).
    #[must_use]
    #[cfg_attr(not(target_arch = "wasm32"), expect(clippy::cast_possible_truncation))]
    pub const fn green(self) -> u8 {
        (self.0 >> 16) as u8
    }

    /// Blue channel (0–255).
    #[must_use]
    #[cfg_attr(not(target_arch = "wasm32"), expect(clippy::cast_possible_truncation))]
    pub const fn blue(self) -> u8 {
        (self.0 >> 8) as u8
    }

    /// Alpha channel (0–255).
    #[must_use]
    #[cfg_attr(not(target_arch = "wasm32"), expect(clippy::cast_possible_truncation))]
    pub const fn alpha(self) -> u8 {
        self.0 as u8
    }

    /// Packed `0xRRGGBBAA` representation.
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        self.0
    }

    /// The colour as egui paints it, alpha preserved.
    /// `const`, unlike the `From` impl, so palette tables can be built at
    /// compile time; `From` delegates here.
    #[cfg(feature = "egui")]
    #[must_use]
    pub const fn to_egui(self) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied_const(
            self.red(),
            self.green(),
            self.blue(),
            self.alpha(),
        )
    }

    /// Return `self` if set (non-zero), otherwise `fallback`.
    #[must_use]
    pub const fn or(self, fallback: Self) -> Self {
        if self.0 != 0 { self } else { fallback }
    }

    // ── Simple manipulation (const-capable) ─────────────────────────

    /// Set the alpha channel (0.0 = transparent, 1.0 = opaque).
    ///
    /// Out-of-range inputs are clamped to `[0.0, 1.0]`; `NaN` is treated as
    /// `0.0` (transparent). Debug builds assert the input is in range so
    /// upstream bugs surface loudly instead of producing wrapped alpha.
    #[must_use]
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)
    )]
    pub const fn with_alpha(self, alpha: f32) -> Self {
        debug_assert!(alpha >= 0.0 && alpha <= 1.0, "alpha must be in [0, 1]");
        let clamped = if alpha > 1.0 {
            1.0
        } else if alpha >= 0.0 {
            alpha
        } else {
            0.0
        };
        let a = (clamped * 255.0) as u32 & 0xFF;
        Self((self.0 & 0xFFFF_FF00) | a)
    }

    /// Scale the current alpha by a factor (0.0 = fully transparent, 1.0 = unchanged).
    ///
    /// Unlike [`with_alpha`](Self::with_alpha) which sets alpha absolutely,
    /// this multiplies the existing alpha: `new_alpha = current_alpha * factor`.
    #[must_use]
    pub const fn scale_alpha(self, factor: f32) -> Self {
        let a = clamp_u8(self.alpha() as f32 * factor);
        Self::from_rgba(self.red(), self.green(), self.blue(), a)
    }

    /// Scale RGB channels (0.0 = black, 1.0 = unchanged). Alpha is preserved.
    ///
    /// This is a raw RGB multiply — fast but NOT perceptually uniform.
    /// For perceptual darkening, use [`lightness`](Self::lightness) instead.
    #[must_use]
    pub const fn brightness(self, scale: f32) -> Self {
        let r = clamp_u8(self.red() as f32 * scale);
        let g = clamp_u8(self.green() as f32 * scale);
        let b = clamp_u8(self.blue() as f32 * scale);
        Self::from_rgba(r, g, b, self.alpha())
    }

    // ── Perceptual manipulation (OkLCH, runtime only) ───────────────

    /// Set perceptual lightness (0.0–1.0) in OkLCH space.
    ///
    /// Hue and alpha are preserved. Gray detection prevents injecting
    /// a random hue into true achromatic colors.
    #[must_use]
    pub fn lightness(self, val: f32) -> Self {
        use palette::{FromColor, IntoColor, Oklch, Srgb};

        let srgb: Srgb<f32> = Srgb::new(self.red(), self.green(), self.blue()).into_format();
        let mut oklch: Oklch = srgb.into_color();

        let is_gray = oklch.chroma < 0.005;
        oklch.l = val;
        if is_gray {
            oklch.chroma = 0.0;
        }

        let out: Srgb<f32> = Srgb::from_color(oklch);
        let out = out.into_format::<u8>();
        Self::from_rgba(out.red, out.green, out.blue, self.alpha())
    }

    /// Set chroma in OkLCH space, preserving lightness, hue, and alpha.
    ///
    /// For non-gray colors, ensures chroma is at least `val` (raises but
    /// never reduces existing chroma). For achromatic (gray) sources,
    /// chroma is forced to 0 — you cannot inject a hue into a true gray
    /// via chroma alone.
    #[must_use]
    pub fn chroma(self, val: f32) -> Self {
        use palette::{FromColor, IntoColor, Oklch, Srgb};

        let srgb: Srgb<f32> = Srgb::new(self.red(), self.green(), self.blue()).into_format();
        let mut oklch: Oklch = srgb.into_color();

        let is_gray = oklch.chroma < 0.005;
        if is_gray {
            oklch.chroma = 0.0;
        } else {
            oklch.chroma = oklch.chroma.max(val);
        }

        let out: Srgb<f32> = Srgb::from_color(oklch);
        let out = out.into_format::<u8>();
        Self::from_rgba(out.red, out.green, out.blue, self.alpha())
    }

    /// Mix with another color in OkLCH perceptual space.
    ///
    /// `t` is the blend fraction: 0.0 = all `self`, 1.0 = all `other`.
    /// Interpolation happens in OkLCH (polar Oklab) — lightness and chroma
    /// interpolate linearly while hue follows the shortest arc. This keeps
    /// blends between different hues vivid (e.g., gray→red stays saturated
    /// through the middle instead of going muddy). Alpha is linearly interpolated.
    #[must_use]
    pub fn mix(self, other: Self, t: f32) -> Self {
        use palette::{FromColor, IntoColor, Mix, Oklch, Srgb};

        let a_lch: Oklch = Srgb::new(self.red(), self.green(), self.blue())
            .into_format::<f32>()
            .into_color();
        let b_lch: Oklch = Srgb::new(other.red(), other.green(), other.blue())
            .into_format::<f32>()
            .into_color();

        let out: Srgb<f32> = Srgb::from_color(a_lch.mix(b_lch, t));
        let out = out.into_format::<u8>();
        let out_a = clamp_u8(f32::from(self.alpha()) * (1.0 - t) + f32::from(other.alpha()) * t);
        Self::from_rgba(out.red, out.green, out.blue, out_a)
    }
}

#[cfg(feature = "egui")]
impl From<Color> for egui::Color32 {
    fn from(color: Color) -> Self {
        color.to_egui()
    }
}

/// Clamp a float to u8 range without `f32::min`/`f32::max` (which aren't const).
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
const fn clamp_u8(val: f32) -> u8 {
    if val > 255.0 {
        255
    } else if val < 0.0 {
        0
    } else {
        val as u8
    }
}

// ── Design system palette ───────────────────────────────────────────

pub const GRAY_10: Color = Color::from_hex(0xF4_F4_F4);
pub const GRAY_20: Color = Color::from_hex(0xE0_E0_E0);
pub const GRAY_30: Color = Color::from_hex(0xC6_C6_C6);
pub const GRAY_40: Color = Color::from_hex(0xA8_A8_A8);
pub const GRAY_50: Color = Color::from_hex(0x8D_8D_8D);
pub const GRAY_60: Color = Color::from_hex(0x6F_6F_6F);
pub const GRAY_70: Color = Color::from_hex(0x52_52_52);
pub const GRAY_80: Color = Color::from_hex(0x39_39_39);
pub const GRAY_90: Color = Color::from_hex(0x26_26_26);
pub const GRAY_100: Color = Color::from_hex(0x16_1616);

pub const LIME_10: Color = Color::from_hex(0xE0_FC_D6);
pub const LIME_20: Color = Color::from_hex(0xA6_F3_82);
pub const LIME_30: Color = Color::from_hex(0x89_DB_5D);
pub const LIME_40: Color = Color::from_hex(0x6D_BC_39);
pub const LIME_50: Color = Color::from_hex(0x59_9F_2A);
pub const LIME_60: Color = Color::from_hex(0x45_7D_1F);
pub const LIME_70: Color = Color::from_hex(0x35_5C_15);
pub const LIME_80: Color = Color::from_hex(0x26_40_0C);
pub const LIME_90: Color = Color::from_hex(0x18_2B_05);
pub const LIME_100: Color = Color::from_hex(0x08_19_05);

pub const GREEN_10: Color = Color::from_hex(0xDD_FB_E9);
pub const GREEN_20: Color = Color::from_hex(0xA3_F1_B9);
pub const GREEN_30: Color = Color::from_hex(0x5A_DF_88);
pub const GREEN_40: Color = Color::from_hex(0x34_C0_6A);
pub const GREEN_50: Color = Color::from_hex(0x13_A4_54);
pub const GREEN_60: Color = Color::from_hex(0x16_80_42);
pub const GREEN_70: Color = Color::from_hex(0x19_5E_33);
pub const GREEN_80: Color = Color::from_hex(0x12_42_23);
pub const GREEN_90: Color = Color::from_hex(0x10_2B_19);
pub const GREEN_100: Color = Color::from_hex(0x06_19_12);

pub const TEAL_10: Color = Color::from_hex(0xE0_FA_FB);
pub const TEAL_20: Color = Color::from_hex(0xA3_EC_F1);
pub const TEAL_30: Color = Color::from_hex(0x56_D8_E0);
pub const TEAL_40: Color = Color::from_hex(0x00_BA_C5);
pub const TEAL_50: Color = Color::from_hex(0x00_9D_A7);
pub const TEAL_60: Color = Color::from_hex(0x00_7C_83);
pub const TEAL_70: Color = Color::from_hex(0x00_5E_5E);
pub const TEAL_80: Color = Color::from_hex(0x00_40_42);
pub const TEAL_90: Color = Color::from_hex(0x00_2A_2D);
pub const TEAL_100: Color = Color::from_hex(0x03_1A_1C);

pub const BLUE_10: Color = Color::from_hex(0xEC_F5_FF);
pub const BLUE_20: Color = Color::from_hex(0xD0_E0_FF);
pub const BLUE_30: Color = Color::from_hex(0xA9_C7_FF);
pub const BLUE_40: Color = Color::from_hex(0x7C_A8_FF);
pub const BLUE_50: Color = Color::from_hex(0x4B_8A_FF);
pub const BLUE_60: Color = Color::from_hex(0x24_60_FF);
pub const BLUE_70: Color = Color::from_hex(0x10_43_CD);
pub const BLUE_80: Color = Color::from_hex(0x0A_2E_9B);
pub const BLUE_90: Color = Color::from_hex(0x07_1D_67);
pub const BLUE_100: Color = Color::from_hex(0x07_13_38);

pub const VIOLET_10: Color = Color::from_hex(0xF3_F2_FF);
pub const VIOLET_20: Color = Color::from_hex(0xDF_DC_FF);
pub const VIOLET_30: Color = Color::from_hex(0xC5_C0_FF);
pub const VIOLET_40: Color = Color::from_hex(0xA6_9D_FF);
pub const VIOLET_50: Color = Color::from_hex(0x8B_7C_FF);
pub const VIOLET_60: Color = Color::from_hex(0x6B_50_FF);
pub const VIOLET_70: Color = Color::from_hex(0x54_32_CD);
pub const VIOLET_80: Color = Color::from_hex(0x39_23_8F);
pub const VIOLET_90: Color = Color::from_hex(0x28_16_61);
pub const VIOLET_100: Color = Color::from_hex(0x17_0D_3A);

pub const PURPLE_10: Color = Color::from_hex(0xFB_F1_FB);
pub const PURPLE_20: Color = Color::from_hex(0xF2_D6_FD);
pub const PURPLE_30: Color = Color::from_hex(0xE3_B6_FA);
pub const PURPLE_40: Color = Color::from_hex(0xD2_8D_F7);
pub const PURPLE_50: Color = Color::from_hex(0xC0_63_F9);
pub const PURPLE_60: Color = Color::from_hex(0xA7_2D_EA);
pub const PURPLE_70: Color = Color::from_hex(0x7E_1C_B2);
pub const PURPLE_80: Color = Color::from_hex(0x59_13_7D);
pub const PURPLE_90: Color = Color::from_hex(0x3B_11_51);
pub const PURPLE_100: Color = Color::from_hex(0x20_0F_29);

pub const MAGENTA_10: Color = Color::from_hex(0xFF_F0_F6);
pub const MAGENTA_20: Color = Color::from_hex(0xFF_D5_E4);
pub const MAGENTA_30: Color = Color::from_hex(0xFF_B0_CA);
pub const MAGENTA_40: Color = Color::from_hex(0xFB_82_A8);
pub const MAGENTA_50: Color = Color::from_hex(0xEE_58_84);
pub const MAGENTA_60: Color = Color::from_hex(0xD3_26_5D);
pub const MAGENTA_70: Color = Color::from_hex(0xA0_17_43);
pub const MAGENTA_80: Color = Color::from_hex(0x72_0F_2D);
pub const MAGENTA_90: Color = Color::from_hex(0x4F_07_1D);
pub const MAGENTA_100: Color = Color::from_hex(0x29_0C_17);

pub const RED_10: Color = Color::from_hex(0xFF_F1_F2);
pub const RED_20: Color = Color::from_hex(0xFF_D6_D5);
pub const RED_30: Color = Color::from_hex(0xFF_B3_B2);
pub const RED_40: Color = Color::from_hex(0xFF_83_84);
pub const RED_50: Color = Color::from_hex(0xF9_53_55);
pub const RED_60: Color = Color::from_hex(0xD9_22_2C);
pub const RED_70: Color = Color::from_hex(0xA2_17_1F);
pub const RED_80: Color = Color::from_hex(0x74_0E_14);
pub const RED_90: Color = Color::from_hex(0x4F_09_0D);
pub const RED_100: Color = Color::from_hex(0x2B_0B_0B);

pub const ORANGE_10: Color = Color::from_hex(0xFF_F1_E9);
pub const ORANGE_20: Color = Color::from_hex(0xFF_D8_BF);
pub const ORANGE_30: Color = Color::from_hex(0xFF_B6_87);
pub const ORANGE_40: Color = Color::from_hex(0xFE_84_31);
pub const ORANGE_50: Color = Color::from_hex(0xEB_63_07);
pub const ORANGE_60: Color = Color::from_hex(0xC1_48_12);
pub const ORANGE_70: Color = Color::from_hex(0x93_32_00);
pub const ORANGE_80: Color = Color::from_hex(0x64_26_00);
pub const ORANGE_90: Color = Color::from_hex(0x42_1B_00);
pub const ORANGE_100: Color = Color::from_hex(0x25_12_00);

pub const GOLD_10: Color = Color::from_hex(0xFF_F2_DE);
pub const GOLD_20: Color = Color::from_hex(0xFD_DC_95);
pub const GOLD_30: Color = Color::from_hex(0xFE_BA_53);
pub const GOLD_40: Color = Color::from_hex(0xED_94_19);
pub const GOLD_50: Color = Color::from_hex(0xCF_79_0E);
pub const GOLD_60: Color = Color::from_hex(0xA4_5F_09);
pub const GOLD_70: Color = Color::from_hex(0x7B_45_05);
pub const GOLD_80: Color = Color::from_hex(0x57_30_02);
pub const GOLD_90: Color = Color::from_hex(0x3B_1F_01);
pub const GOLD_100: Color = Color::from_hex(0x24_11_00);

pub const YELLOW_10: Color = Color::from_hex(0xFC_F4_D6);
pub const YELLOW_20: Color = Color::from_hex(0xFE_DD_6F);
pub const YELLOW_30: Color = Color::from_hex(0xF4_C0_1A);
pub const YELLOW_40: Color = Color::from_hex(0xD3_A1_03);
pub const YELLOW_50: Color = Color::from_hex(0xB2_87_00);
pub const YELLOW_60: Color = Color::from_hex(0x8E_6B_00);
pub const YELLOW_70: Color = Color::from_hex(0x69_4F_04);
pub const YELLOW_80: Color = Color::from_hex(0x49_36_05);
pub const YELLOW_90: Color = Color::from_hex(0x31_24_02);
pub const YELLOW_100: Color = Color::from_hex(0x1D_14_01);

pub const BLACK: Color = Color::from_hex(0x00_00_00);
pub const WHITE: Color = Color::from_hex(0xFF_FF_FF);
pub const TRANSPARENT: Color = Color::from_rgba(0, 0, 0, 0);

// ── Palette collection ──────────────────────────────────────────────

/// A named row of 10 color swatches (steps 10–100).
#[derive(Debug, Clone, Copy)]
pub struct ColorSwatch {
    pub name: &'static str,
    pub colors: [Color; 10],
}

/// The full design system palette — all color families at steps 10–100.
pub const PALETTE: &[ColorSwatch] = &[
    ColorSwatch {
        name: "Gray",
        colors: [
            GRAY_10, GRAY_20, GRAY_30, GRAY_40, GRAY_50, GRAY_60, GRAY_70, GRAY_80, GRAY_90,
            GRAY_100,
        ],
    },
    ColorSwatch {
        name: "Blue",
        colors: [
            BLUE_10, BLUE_20, BLUE_30, BLUE_40, BLUE_50, BLUE_60, BLUE_70, BLUE_80, BLUE_90,
            BLUE_100,
        ],
    },
    ColorSwatch {
        name: "Green",
        colors: [
            GREEN_10, GREEN_20, GREEN_30, GREEN_40, GREEN_50, GREEN_60, GREEN_70, GREEN_80,
            GREEN_90, GREEN_100,
        ],
    },
    ColorSwatch {
        name: "Red",
        colors: [
            RED_10, RED_20, RED_30, RED_40, RED_50, RED_60, RED_70, RED_80, RED_90, RED_100,
        ],
    },
    ColorSwatch {
        name: "Violet",
        colors: [
            VIOLET_10, VIOLET_20, VIOLET_30, VIOLET_40, VIOLET_50, VIOLET_60, VIOLET_70, VIOLET_80,
            VIOLET_90, VIOLET_100,
        ],
    },
    ColorSwatch {
        name: "Gold",
        colors: [
            GOLD_10, GOLD_20, GOLD_30, GOLD_40, GOLD_50, GOLD_60, GOLD_70, GOLD_80, GOLD_90,
            GOLD_100,
        ],
    },
    ColorSwatch {
        name: "Yellow",
        colors: [
            YELLOW_10, YELLOW_20, YELLOW_30, YELLOW_40, YELLOW_50, YELLOW_60, YELLOW_70, YELLOW_80,
            YELLOW_90, YELLOW_100,
        ],
    },
    ColorSwatch {
        name: "Orange",
        colors: [
            ORANGE_10, ORANGE_20, ORANGE_30, ORANGE_40, ORANGE_50, ORANGE_60, ORANGE_70, ORANGE_80,
            ORANGE_90, ORANGE_100,
        ],
    },
    ColorSwatch {
        name: "Teal",
        colors: [
            TEAL_10, TEAL_20, TEAL_30, TEAL_40, TEAL_50, TEAL_60, TEAL_70, TEAL_80, TEAL_90,
            TEAL_100,
        ],
    },
    ColorSwatch {
        name: "Purple",
        colors: [
            PURPLE_10, PURPLE_20, PURPLE_30, PURPLE_40, PURPLE_50, PURPLE_60, PURPLE_70, PURPLE_80,
            PURPLE_90, PURPLE_100,
        ],
    },
    ColorSwatch {
        name: "Magenta",
        colors: [
            MAGENTA_10,
            MAGENTA_20,
            MAGENTA_30,
            MAGENTA_40,
            MAGENTA_50,
            MAGENTA_60,
            MAGENTA_70,
            MAGENTA_80,
            MAGENTA_90,
            MAGENTA_100,
        ],
    },
    ColorSwatch {
        name: "Lime",
        colors: [
            LIME_10, LIME_20, LIME_30, LIME_40, LIME_50, LIME_60, LIME_70, LIME_80, LIME_90,
            LIME_100,
        ],
    },
];
