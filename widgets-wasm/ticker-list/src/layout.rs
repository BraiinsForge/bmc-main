// Copyright (C) 2026  Braiins Systems s.r.o.

//! Per-size layout bands (row capacity, columns, sparkline box, fonts) and their
//! `fit`-scaling. Fonts and geometry scale by [`WidgetSize::fit`] so BMM101
//! (480×320) shrinks instead of overflowing — the same mechanism the clock and
//! the sparkline use. Row/column counts and `show_sparkline` are layout
//! structure and are picked from the variant, not scaled.

use bmc_wasm_sdk::{SizeVariant, scale_font};

#[derive(Clone, Copy)]
pub struct Band {
    pub symbol_font: u32,
    pub company_font: u32,
    pub price_font: u32,
    pub change_font: u32,
    pub chart_width: f32,
    pub chart_height: f32,
    pub badge_padding: f32,
    pub row_padding: f32,
    pub row_gap: f32,
    /// Character budget for the (ellipsized) company name.
    pub company_chars: usize,
    /// Maximum rows rendered (and the fetch capacity).
    pub rows: usize,
    /// 2 at Full (two interleaved columns), 1 otherwise.
    pub columns: usize,
    pub show_sparkline: bool,
    /// Whether the stale badge carries the "Stale data" label; icon-only on
    /// bands too narrow to fit the label between symbol and price.
    pub stale_label: bool,
}

const FULL: Band = Band {
    symbol_font: 32,
    company_font: 24,
    price_font: 32,
    change_font: 24,
    chart_width: 140.0,
    chart_height: 56.0,
    badge_padding: 4.0,
    row_padding: 12.0,
    row_gap: 4.0,
    company_chars: 24,
    rows: 8,
    columns: 2,
    show_sparkline: true,
    stale_label: true,
};

const LARGE: Band = Band {
    chart_height: 50.0,
    rows: 4,
    columns: 1,
    ..FULL
};

const MEDIUM: Band = Band {
    chart_height: 45.0,
    rows: 2,
    columns: 1,
    ..FULL
};

const SMALL: Band = Band {
    chart_height: 0.0,
    company_chars: 12,
    rows: 2,
    columns: 1,
    show_sparkline: false,
    stale_label: false,
    ..FULL
};

impl Band {
    /// Multiply fonts and geometry by `fit`; counts, columns, the sparkline
    /// flag, and the char budget are structure and pass through unscaled.
    #[must_use]
    pub fn scaled(self, fit: f32) -> Self {
        Self {
            symbol_font: scale_font(self.symbol_font, fit),
            company_font: scale_font(self.company_font, fit),
            price_font: scale_font(self.price_font, fit),
            change_font: scale_font(self.change_font, fit),
            chart_width: self.chart_width * fit,
            chart_height: self.chart_height * fit,
            badge_padding: self.badge_padding * fit,
            row_padding: self.row_padding * fit,
            row_gap: self.row_gap * fit,
            ..self
        }
    }
}

#[must_use]
pub fn band_for(variant: SizeVariant) -> Band {
    match variant {
        SizeVariant::Full => FULL,
        SizeVariant::Large => LARGE,
        SizeVariant::Medium => MEDIUM,
        SizeVariant::Small => SMALL,
    }
}

/// How many symbols a size renders (and fetches), matching deckfeeder's
/// `getMaxSymbolsForViewport`: Full 8, Large 4, Medium/Small 2.
#[must_use]
pub fn size_capacity(variant: SizeVariant) -> usize {
    band_for(variant).rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacities_match_deckfeeder_per_size() {
        assert_eq!(size_capacity(SizeVariant::Full), 8);
        assert_eq!(size_capacity(SizeVariant::Large), 4);
        assert_eq!(size_capacity(SizeVariant::Medium), 2);
        assert_eq!(size_capacity(SizeVariant::Small), 2);
    }

    #[test]
    fn full_is_two_columns_with_sparkline_small_is_one_without() {
        assert_eq!(band_for(SizeVariant::Full).columns, 2);
        assert!(band_for(SizeVariant::Full).show_sparkline);
        assert_eq!(band_for(SizeVariant::Small).columns, 1);
        assert!(!band_for(SizeVariant::Small).show_sparkline);
    }

    #[test]
    fn fit_scales_fonts_and_geometry_not_counts() {
        let scaled = band_for(SizeVariant::Large).scaled(0.5);
        assert_eq!(scaled.symbol_font, 16);
        assert!((scaled.chart_height - 25.0).abs() < 1e-3);
        assert_eq!(scaled.rows, 4);
        assert_eq!(scaled.company_chars, 24);
    }
}
