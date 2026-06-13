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

//! Sparkline geometry, shared by both ticker widgets and mining-info. Free of
//! SDK draw types so it builds and unit-tests on the host; the wasm render
//! path turns these points into canvas draw commands.

/// Trend direction: the series ends at or above where it started.
#[must_use]
pub fn is_rising(series: &[f64]) -> bool {
    match (series.first(), series.last()) {
        (Some(first), Some(last)) if series.len() >= 2 => last >= first,
        _ => false,
    }
}

/// Map a price series onto a `width × height` box: x spreads the samples evenly
/// across the full width, y normalizes price into the box inverted (lowest
/// price at the bottom, highest at the top) with a vertical inset so the stroke
/// is not clipped at the edges. A flat series sits on the centre line. Returns
/// an empty vec for fewer than two samples.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "sample counts and fiat prices stay well within f32's exact range; sub-pixel error is irrelevant for a sparkline"
)]
pub fn series_points(series: &[f64], width: f32, height: f32, inset: f32) -> Vec<(f32, f32)> {
    if series.len() < 2 {
        return Vec::new();
    }
    let min = series.iter().copied().fold(f64::INFINITY, f64::min);
    let max = series.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;
    let top = inset;
    let bottom = height - inset;
    let last = series.len() - 1;
    series
        .iter()
        .enumerate()
        .map(|(index, &value)| {
            let x = width * index as f32 / last as f32;
            let y = if range > 0.0 {
                let normalized = ((value - min) / range) as f32;
                bottom - normalized * (bottom - top)
            } else {
                f32::midpoint(top, bottom)
            };
            (x, y)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rising_compares_first_and_last_sample() {
        assert!(is_rising(&[1.0, 0.5, 2.0]));
        assert!(!is_rising(&[2.0, 3.0, 1.0]));
        assert!(is_rising(&[5.0, 5.0]));
        assert!(!is_rising(&[]));
        assert!(!is_rising(&[7.0]));
    }

    #[test]
    fn series_needs_at_least_two_points() {
        assert!(series_points(&[1.0], 100.0, 40.0, 2.0).is_empty());
    }

    #[test]
    fn series_spans_full_width_and_inverts_price_to_screen_y() {
        let points = series_points(&[10.0, 20.0, 30.0], 100.0, 40.0, 2.0);
        assert_eq!(points.len(), 3);
        assert!((points[0].0 - 0.0).abs() < 1e-6);
        assert!((points[2].0 - 100.0).abs() < 1e-6);
        assert!((points[0].1 - 38.0).abs() < 1e-6);
        assert!((points[2].1 - 2.0).abs() < 1e-6);
    }

    #[test]
    fn flat_series_centers_vertically() {
        for (_, y) in series_points(&[5.0, 5.0, 5.0], 100.0, 40.0, 2.0) {
            assert!((y - 20.0).abs() < 1e-6);
        }
    }
}
