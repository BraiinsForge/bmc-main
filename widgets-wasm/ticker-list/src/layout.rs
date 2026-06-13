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
    /// 2 at Full, 1 otherwise.
    pub columns: usize,
    pub show_sparkline: bool,
    /// Whether the stale badge carries the last-refresh age; icon-only on
    /// bands too narrow to fit it between symbol and price.
    pub stale_label: bool,
    pub stale_font: u32,
    pub stale_icon: f32,
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
    stale_font: 14,
    stale_icon: 16.0,
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
            stale_font: scale_font(self.stale_font, fit),
            stale_icon: self.stale_icon * fit,
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

/// How many symbols a size renders and fetches: Full 8, Large 4,
/// Medium/Small 2.
#[must_use]
pub fn size_capacity(variant: SizeVariant) -> usize {
    band_for(variant).rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacities_match_size_bands() {
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
