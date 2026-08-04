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

//! Chart geometry, free of SDK draw types: the render path turns these
//! points into path draw commands.

use crate::model::Series;

/// Top of a chart's y axis: the sample maximum rounded up to the next
/// multiple of `3 × 10^k`, so the axis divides into thirds on round values.
/// `integer_round` keeps the multiple coarse for counted series (workers);
/// continuous series (hashrate) round one decade finer.
#[must_use]
pub fn y_axis_max(value: f64, integer_round: bool) -> f64 {
    /// The axis for a series with no magnitude to scale to.
    const FLAT_TOP: f64 = 3.0;

    if value == 0.0 {
        return FLAT_TOP;
    }
    let exponent = if integer_round {
        value.log10().floor()
    } else {
        value.log10().floor() - 1.0
    };
    let divisor = 3.0 * f64::powf(10.0, exponent);
    let top = (value / divisor).ceil() * divisor;
    // A denormal value underflows the divisor to zero and one near `f64::MAX`
    // overflows it to infinity; either product is NaN, which the axis must not
    // hand on — its top ends up as a `clamp` bound in [`series_points`].
    if top.is_finite() && top > 0.0 {
        top
    } else {
        FLAT_TOP
    }
}

/// Axis labels from top to bottom: max, ⅔·max, ⅓·max, 0.
#[must_use]
pub fn axis_thirds(max: f64) -> [f64; 4] {
    [max, 2.0 * max / 3.0, max / 3.0, 0.0]
}

/// Evenly spaced x-axis marks across a time window, endpoints included:
/// each mark's timestamp with its fraction of the span. The caller formats
/// the timestamps (host strftime on wasm, a pure formatter in fixtures).
#[must_use]
pub fn x_axis_marks(from: i64, to: i64, count: usize) -> Vec<(i64, f32)> {
    if to <= from || count < 2 {
        return Vec::new();
    }
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "label counts and chart spans are tiny; exact in f32/f64"
    )]
    (0..count)
        .map(|i| {
            let fraction = i as f32 / (count - 1) as f32;
            let at = from + ((to - from) as f64 * f64::from(fraction)) as i64;
            (at, fraction)
        })
        .collect()
}

/// Fraction (0..=1) of the chart's time span at which a moment sits,
/// or `None` when it falls outside the span. Places payout markers on
/// the time axis.
#[must_use]
pub fn time_fraction(at: i64, from: i64, to: i64) -> Option<f64> {
    let span = to - from;
    if span <= 0 || at < from || at > to {
        return None;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "chart spans are hours-to-days of seconds, far inside f64's exact range"
    )]
    Some((at - from) as f64 / span as f64)
}

/// Map a series onto a `width` × `height` box against a fixed y-axis top:
/// x positions samples by their timestamp across the series window, y maps
/// `[0, axis_max]` onto the box inverted (zero at the bottom), with a
/// vertical `inset` so the stroke isn't clipped at the edges.
///
/// The axis top comes from [`y_axis_max`] rather than the sample maximum
/// so the line shares its scale with the axis labels.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "sample counts and chart spans stay well within f64's exact range; sub-pixel error is invisible"
)]
#[must_use]
pub fn series_points(
    series: &Series,
    axis_max: f64,
    width: f32,
    height: f32,
    inset: f32,
) -> Vec<(f32, f32)> {
    let (Some(from), Some(to)) = (series.from, series.to) else {
        return Vec::new();
    };
    let span = to - from;
    // The `clamp` below panics on a NaN bound, whoever computed the top.
    if span <= 0 || series.samples.len() < 2 || !axis_max.is_finite() || axis_max <= 0.0 {
        return Vec::new();
    }
    let top = inset;
    let bottom = height - inset;
    series
        .samples
        .iter()
        .map(|sample| {
            let x = width * ((sample.at - from) as f64 / span as f64) as f32;
            let normalized = (sample.value.clamp(0.0, axis_max) / axis_max) as f32;
            let y = bottom - normalized * (bottom - top);
            (x, y)
        })
        .collect()
}

/// Thin a point list to at most `max_points`, keeping first and last: the
/// 7-day frame carries ~2000 five-minute slots, several per rendered pixel.
#[expect(
    clippy::integer_division,
    reason = "index selection wants the floor; the dropped remainder is the point of decimation"
)]
#[must_use]
pub fn decimate(points: Vec<(f32, f32)>, max_points: usize) -> Vec<(f32, f32)> {
    if points.len() <= max_points || max_points < 2 {
        return points;
    }
    let last = points.len() - 1;
    (0..max_points)
        .map(|i| points[i * last / (max_points - 1)])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Sample;

    fn series(samples: &[(i64, f64)]) -> Series {
        Series {
            from: Some(0),
            to: Some(100),
            samples: samples
                .iter()
                .map(|&(at, value)| Sample { at, value })
                .collect(),
        }
    }

    #[test]
    fn points_span_the_window_not_the_samples() {
        let points = series_points(&series(&[(0, 0.0), (50, 5.0)]), 10.0, 200.0, 100.0, 0.0);
        assert_eq!(points.len(), 2);
        assert!((points[0].0).abs() < 1e-6);
        assert!(
            (points[1].0 - 100.0).abs() < 1e-6,
            "sample at half the window sits at half the width"
        );
    }

    #[test]
    fn y_maps_zero_to_bottom_and_axis_max_to_top() {
        let points = series_points(&series(&[(0, 0.0), (100, 10.0)]), 10.0, 100.0, 50.0, 2.0);
        assert!((points[0].1 - 48.0).abs() < 1e-6);
        assert!((points[1].1 - 2.0).abs() < 1e-6);
    }

    #[test]
    fn values_above_the_axis_clamp_to_the_top() {
        let points = series_points(&series(&[(0, 99.0), (100, 0.0)]), 10.0, 100.0, 50.0, 0.0);
        assert!((points[0].1).abs() < 1e-6);
    }

    #[test]
    fn degenerate_series_yield_no_points() {
        assert!(series_points(&series(&[(0, 1.0)]), 10.0, 100.0, 50.0, 0.0).is_empty());
        let mut no_window = series(&[(0, 1.0), (50, 2.0)]);
        no_window.from = None;
        assert!(series_points(&no_window, 10.0, 100.0, 50.0, 0.0).is_empty());
    }

    #[test]
    fn y_axis_max_rounds_to_thirds_friendly_top() {
        assert!((y_axis_max(0.0, false) - 3.0).abs() < f64::EPSILON);
        // 349.8 → exponent 1 → divisor 30 → ceil(11.66) = 12 → 360
        assert!((y_axis_max(349.8, false) - 360.0).abs() < 1e-9);
        // integer rounding: 2458 → divisor 3000 → 3000
        assert!((y_axis_max(2_458.0, true) - 3_000.0).abs() < 1e-9);
    }

    /// Every top the axis yields becomes a `clamp` bound, and a reply's number
    /// reaches it unchecked.
    #[test]
    fn y_axis_max_survives_the_extremes_of_f64() {
        for (value, integer_round) in [
            // The divisor's decade underflows to zero.
            (1e-323, false),
            (f64::MIN_POSITIVE, false),
            // Tripling the decade overflows it to infinity.
            (f64::MAX, true),
            (f64::INFINITY, false),
        ] {
            let top = y_axis_max(value, integer_round);
            assert!(
                top.is_finite() && top > 0.0,
                "y_axis_max({value:e}, {integer_round}) is not a usable axis top: {top}"
            );
        }
    }

    #[test]
    fn a_non_finite_axis_yields_no_points() {
        let samples = series(&[(0, 1.0), (50, 2.0)]);
        assert!(series_points(&samples, f64::NAN, 100.0, 50.0, 0.0).is_empty());
        assert!(series_points(&samples, f64::INFINITY, 100.0, 50.0, 0.0).is_empty());
    }

    #[test]
    fn axis_labels_divide_into_thirds() {
        let labels = axis_thirds(360.0);
        assert!((labels[1] - 240.0).abs() < 1e-9);
        assert!((labels[2] - 120.0).abs() < 1e-9);
        assert!((labels[3]).abs() < f64::EPSILON);
    }

    #[test]
    fn x_axis_marks_span_endpoints_evenly() {
        let marks = x_axis_marks(0, 600, 5);
        assert_eq!(marks.len(), 5);
        assert_eq!(marks[0], (0, 0.0));
        assert_eq!(marks[2].0, 300);
        assert_eq!(marks[4], (600, 1.0));
        assert!(x_axis_marks(600, 600, 5).is_empty());
        assert!(x_axis_marks(0, 600, 1).is_empty());
    }

    #[test]
    fn time_fraction_clamps_to_the_span() {
        assert_eq!(time_fraction(150, 100, 200), Some(0.5));
        assert_eq!(time_fraction(99, 100, 200), None);
        assert_eq!(time_fraction(201, 100, 200), None);
        assert_eq!(time_fraction(100, 100, 100), None);
    }

    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "indices up to 2016 are exact in f32"
    )]
    fn decimate_keeps_endpoints_and_bounds_count() {
        let points: Vec<(f32, f32)> = (0..2_016).map(|i| (i as f32, 0.0)).collect();
        let thinned = decimate(points.clone(), 256);
        assert_eq!(thinned.len(), 256);
        assert_eq!(thinned[0], points[0]);
        assert_eq!(thinned[255], points[2_015]);
        assert_eq!(decimate(points[..100].to_vec(), 256).len(), 100);
    }
}
