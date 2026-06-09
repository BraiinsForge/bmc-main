// Copyright (C) 2026  Braiins Systems s.r.o.

//! Per-size layout bands and their `fit`-scaling.
//!
//! Each [`SizeVariant`] carries a canonical band (font sizes, chart height,
//! paddings) taken from the deckfeeder per-size CSS at that variant's canonical
//! viewport. The band is then multiplied by [`WidgetSize::fit`] so an
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
    pub chart_height: f32,
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
}

const FULL: SizeBand = SizeBand {
    price_font: 154,
    chart_height: 229.0,
    header_font: 24,
    icon_diameter: 24.0,
    glyph_font: 14,
    badge_padding: 5.0,
    top_padding: 16.0,
    header_left_gap: 12.0,
    header_right_gap: 8.0,
    edge_padding: 16.0,
    show_period: true,
};

const LARGE: SizeBand = SizeBand {
    price_font: 51,
    chart_height: 180.0,
    header_font: 20,
    icon_diameter: 20.0,
    ..FULL
};

const MEDIUM: SizeBand = SizeBand {
    price_font: 40,
    chart_height: 120.0,
    header_font: 16,
    icon_diameter: 16.0,
    top_padding: 12.0,
    edge_padding: 12.0,
    ..FULL
};

const SMALL: SizeBand = SizeBand {
    price_font: 24,
    chart_height: 80.0,
    header_font: 14,
    icon_diameter: 14.0,
    badge_padding: 4.0,
    top_padding: 8.0,
    header_left_gap: 8.0,
    header_right_gap: 4.0,
    edge_padding: 8.0,
    show_period: false,
    ..FULL
};

impl SizeBand {
    /// Multiply fonts and geometry by `fit`; visibility flags are layout
    /// structure and pass through unscaled.
    #[must_use]
    pub fn scaled(self, fit: f32) -> Self {
        Self {
            price_font: scale_font(self.price_font, fit),
            chart_height: self.chart_height * fit,
            header_font: scale_font(self.header_font, fit),
            icon_diameter: self.icon_diameter * fit,
            glyph_font: scale_font(self.glyph_font, fit),
            badge_padding: self.badge_padding * fit,
            top_padding: self.top_padding * fit,
            header_left_gap: self.header_left_gap * fit,
            header_right_gap: self.header_right_gap * fit,
            edge_padding: self.edge_padding * fit,
            show_period: self.show_period,
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
        assert!((scaled.chart_height - 114.5).abs() < 1e-3);
        assert_eq!(scaled.price_font, 77);
        assert_eq!(scaled.header_font, 12);
        assert!(scaled.show_period);
    }

    #[test]
    fn fit_one_is_identity_for_the_canonical_band() {
        let band = band_for(SizeVariant::Medium);
        let scaled = band.scaled(1.0);
        assert_eq!(scaled.price_font, band.price_font);
        assert!((scaled.chart_height - band.chart_height).abs() < 1e-6);
    }
}
