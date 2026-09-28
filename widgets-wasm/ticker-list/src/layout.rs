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

//! Which frame a viewport gets, the Deck's per-size layout bands (row capacity,
//! columns, sparkline box, fonts) and their `fit`-scaling. Fonts and geometry
//! scale by [`WidgetSize::fit`] so a viewport short of its variant's box shrinks
//! instead of overflowing. Row/column counts and `show_sparkline` are layout
//! structure and are picked from the variant, not scaled.
//! BMM101 draws a frame of its own instead.

use bmc_wasm_sdk::{SizeVariant, WidgetSize, scale_font};

/// The frames a layout is picked for: the four BMC100 slots and BMM101's 480×320.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeBucket {
    Full,
    Large,
    Medium,
    Small,
    Bmm101,
}

impl SizeBucket {
    #[must_use]
    pub const fn design_size(self) -> (u32, u32) {
        match self {
            Self::Full => (1_280, 480),
            Self::Large => (638, 480),
            Self::Medium => (638, 238),
            Self::Small => (317, 238),
            Self::Bmm101 => (480, 320),
        }
    }
}

/// The two BMM frames the narrow bucket has to tell apart.
const BMM100_HEIGHT: u32 = 240;
const BMM101_HEIGHT: u32 = SizeBucket::Bmm101.design_size().1;
/// Split at their midpoint, so either frame keeps its bucket a few pixels either way.
const BMM101_MIN_HEIGHT: u32 = u32::midpoint(BMM100_HEIGHT, BMM101_HEIGHT);
const BMM101_MAX_WIDTH: u32 = SizeBucket::Bmm101.design_size().0;

/// A landscape frame no wider than BMM101 and at least as tall as the BMM split
/// is BMM101; everything else takes the closest BMC100 variant, as the SDK does.
#[must_use]
pub fn size_bucket(width: u32, height: u32) -> SizeBucket {
    if width <= BMM101_MAX_WIDTH && height < width && height >= BMM101_MIN_HEIGHT {
        return SizeBucket::Bmm101;
    }
    match SizeVariant::closest(width, height) {
        SizeVariant::Full => SizeBucket::Full,
        SizeVariant::Large => SizeBucket::Large,
        SizeVariant::Medium => SizeBucket::Medium,
        SizeVariant::Small => SizeBucket::Small,
    }
}

/// BMM101 seats as many rows as the Deck's Large.
pub const BMM101_ROWS: usize = 4;

#[derive(Clone, Copy, Debug)]
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
    /// Maximum rows rendered (and the fetch capacity).
    pub rows: usize,
    /// 2 at Full, 1 otherwise.
    pub columns: usize,
    pub show_sparkline: bool,
    /// The pause marker's diameter, and the stale warning's icon.
    pub marker_size: f32,
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
    rows: 8,
    columns: 2,
    show_sparkline: true,
    marker_size: 16.0,
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
    rows: 2,
    columns: 1,
    show_sparkline: false,
    ..FULL
};

impl Band {
    /// Multiply fonts and geometry by `fit`; counts, columns and the sparkline
    /// flag are structure and pass through unscaled.
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
            // Whole pixels, as `pause_marker` centres whole-pixel bars on the disc.
            marker_size: (self.marker_size * fit).round(),
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

/// How many symbols a viewport renders and fetches:
/// Full 8, Large and BMM101 4, Medium and Small 2.
#[must_use]
pub fn capacity(ws: WidgetSize) -> usize {
    match size_bucket(ws.width, ws.height) {
        SizeBucket::Bmm101 => BMM101_ROWS,
        SizeBucket::Full | SizeBucket::Large | SizeBucket::Medium | SizeBucket::Small => {
            band_for(ws.variant).rows
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(bucket: SizeBucket) -> WidgetSize {
        let (width, height) = bucket.design_size();
        WidgetSize::from_dimensions(width, height)
    }

    #[test]
    fn capacities_match_size_bands() {
        assert_eq!(capacity(at(SizeBucket::Full)), 8);
        assert_eq!(capacity(at(SizeBucket::Large)), 4);
        assert_eq!(capacity(at(SizeBucket::Medium)), 2);
        assert_eq!(capacity(at(SizeBucket::Small)), 2);
        assert_eq!(capacity(at(SizeBucket::Bmm101)), 4);
    }

    #[test]
    fn every_design_size_lands_in_its_own_bucket() {
        for bucket in [
            SizeBucket::Full,
            SizeBucket::Large,
            SizeBucket::Medium,
            SizeBucket::Small,
            SizeBucket::Bmm101,
        ] {
            let (width, height) = bucket.design_size();
            assert_eq!(size_bucket(width, height), bucket, "{width}x{height}");
        }
    }

    /// A frame that misses the BMM101 rule by a pixel falls to the SDK's closest variant.
    #[test]
    fn the_bmm101_bucket_ends_at_the_midpoint_of_the_bmm_heights_and_at_its_width() {
        assert_eq!(size_bucket(320, 240), SizeBucket::Small, "BMM100");
        assert_eq!(size_bucket(480, 280), SizeBucket::Bmm101);
        assert_eq!(size_bucket(480, 279), SizeBucket::Medium);
        assert_eq!(size_bucket(481, 320), SizeBucket::Large);
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
    }
}
