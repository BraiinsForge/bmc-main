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

//! Per-size layout bands and their `fit`-scaling.
//!
//! Each [`SizeVariant`] carries a canonical band (font sizes, chart height,
//! paddings) at that variant's canonical viewport. The band is then multiplied
//! by [`WidgetSize::fit`] so an
//! off-canonical viewport (e.g. BMM101's 480×320) shrinks instead of
//! overflowing — the same scaling the digital/analog clock faces use.
//!
//! This deliberately scales fonts with the viewport, diverging from the
//! best-practices "keep typography stable" rule, because BMM101 support needs
//! it and `fit` is band-bounded (never inflates), exactly like the clock.

use bmc_wasm_sdk::{SizeVariant, scale_font};

#[derive(Clone, Copy)]
pub struct SizeBand {
    pub price_font: u32,
    /// Symbol / period / change-badge font.
    pub header_font: u32,
    pub icon_diameter: f32,
    pub glyph_font: u32,
    pub badge_padding: f32,
    pub top_padding: f32,
    pub header_left_gap: f32,
    pub header_right_gap: f32,
    /// Horizontal inset of the header content from the tile edges.
    pub edge_padding: f32,
    pub show_period: bool,
    /// Width of the candlestick price-axis gutter.
    pub axis_width: f32,
    /// Gap between the chart area and the price-axis gutter.
    pub axis_gap: f32,
    pub axis_font: u32,
    /// Height of the volume strip under the candles.
    pub volume_height: f32,
    /// Height of the time-label strip under the volume bars.
    pub x_label_height: f32,
    pub show_x_labels: bool,
}

const FULL: SizeBand = SizeBand {
    price_font: 154,
    header_font: 24,
    icon_diameter: 24.0,
    glyph_font: 14,
    badge_padding: 5.0,
    top_padding: 16.0,
    header_left_gap: 12.0,
    header_right_gap: 8.0,
    edge_padding: 16.0,
    show_period: true,
    axis_width: 120.0,
    axis_gap: 12.0,
    axis_font: 20,
    volume_height: 70.0,
    x_label_height: 35.0,
    show_x_labels: true,
};

const LARGE: SizeBand = SizeBand {
    price_font: 51,
    header_font: 20,
    icon_diameter: 20.0,
    ..FULL
};

const MEDIUM: SizeBand = SizeBand {
    price_font: 40,
    header_font: 16,
    icon_diameter: 16.0,
    top_padding: 12.0,
    edge_padding: 12.0,
    show_x_labels: false,
    ..FULL
};

const SMALL: SizeBand = SizeBand {
    price_font: 24,
    header_font: 14,
    icon_diameter: 14.0,
    badge_padding: 4.0,
    top_padding: 8.0,
    header_left_gap: 8.0,
    header_right_gap: 4.0,
    edge_padding: 8.0,
    show_period: false,
    show_x_labels: false,
    ..FULL
};

impl SizeBand {
    /// Multiply fonts and geometry by `fit`; visibility flags are layout
    /// structure and pass through unscaled.
    #[must_use]
    pub fn scaled(self, fit: f32) -> Self {
        Self {
            price_font: scale_font(self.price_font, fit),
            header_font: scale_font(self.header_font, fit),
            icon_diameter: self.icon_diameter * fit,
            glyph_font: scale_font(self.glyph_font, fit),
            badge_padding: self.badge_padding * fit,
            top_padding: self.top_padding * fit,
            header_left_gap: self.header_left_gap * fit,
            header_right_gap: self.header_right_gap * fit,
            edge_padding: self.edge_padding * fit,
            axis_width: self.axis_width * fit,
            axis_gap: self.axis_gap * fit,
            axis_font: scale_font(self.axis_font, fit),
            volume_height: self.volume_height * fit,
            x_label_height: self.x_label_height * fit,
            ..self
        }
    }
}

/// The canonical band for a size variant, before `fit`-scaling.
#[must_use]
pub fn band_for(variant: SizeVariant) -> SizeBand {
    match variant {
        SizeVariant::Full => FULL,
        SizeVariant::Large => LARGE,
        SizeVariant::Medium => MEDIUM,
        SizeVariant::Small => SMALL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_hides_the_period_label() {
        assert!(!band_for(SizeVariant::Small).show_period);
        assert!(band_for(SizeVariant::Medium).show_period);
        assert!(band_for(SizeVariant::Full).show_period);
    }

    #[test]
    fn fit_scales_geometry_and_fonts_but_not_visibility() {
        let scaled = band_for(SizeVariant::Full).scaled(0.5);
        assert_eq!(scaled.price_font, 77);
        assert_eq!(scaled.header_font, 12);
        assert!(scaled.show_period);
    }

    #[test]
    fn candlestick_chrome_is_fixed_and_fit_scaled() {
        // The axis gutter, volume strip, and time-label strip keep the same
        // canonical dimensions at every size; only fit-scaling shrinks them.
        // Time labels show at full/large only.
        let full = band_for(SizeVariant::Full);
        assert!((full.axis_width - 120.0).abs() < f32::EPSILON);
        assert!((full.axis_gap - 12.0).abs() < f32::EPSILON);
        assert_eq!(full.axis_font, 20);
        assert!((full.volume_height - 70.0).abs() < f32::EPSILON);
        assert!((full.x_label_height - 35.0).abs() < f32::EPSILON);
        assert!(full.show_x_labels);
        assert!(band_for(SizeVariant::Large).show_x_labels);
        assert!(!band_for(SizeVariant::Medium).show_x_labels);
        assert!(!band_for(SizeVariant::Small).show_x_labels);
        let scaled = full.scaled(0.5);
        assert!((scaled.axis_width - 60.0).abs() < f32::EPSILON);
        assert_eq!(scaled.axis_font, 10);
    }

    #[test]
    fn fit_one_is_identity_for_the_canonical_band() {
        let band = band_for(SizeVariant::Medium);
        let scaled = band.scaled(1.0);
        assert_eq!(scaled.price_font, band.price_font);
    }
}
