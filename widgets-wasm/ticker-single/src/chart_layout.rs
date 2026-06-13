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

//! Pure geometry and selection math for the candlestick chart view.
//! No host calls — everything here is unit-testable on the host.

use bmc_wasm_sdk::LocalDateTime;
use bmc_wasm_sdk::system::{DateFormat, TimeFormat};
use prices::candle::CandleBar;
use prices::format::push_uint;
use prices::period::Period;

/// Minimum horizontal slot per candle (body + gap) for readability.
pub const MIN_SLOT_PX: f32 = 4.0;

/// How many candles fit a plot of this width at [`MIN_SLOT_PX`].
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "plot widths are small positive pixel counts"
)]
pub fn max_candles(plot_width: f32) -> usize {
    (plot_width / MIN_SLOT_PX).floor().max(1.0) as usize
}

/// Merge adjacent bars so at most `max` remain. OHLC composes exactly
/// (open of first, close of last, extremes of the group); volumes sum,
/// staying `None` when no bar in the group carries one. The trailing
/// group may hold fewer bars — a partial coarse candle, like any live
/// chart tail.
#[must_use]
pub fn merge_bars(bars: &[CandleBar], max: usize) -> Vec<CandleBar> {
    if max == 0 || bars.len() <= max {
        return bars.to_vec();
    }
    let group = bars.len().div_ceil(max);
    bars.chunks(group)
        .map(|chunk| {
            let first = chunk
                .first()
                .expect("BUG: chunks never yields an empty chunk");
            let last = chunk
                .last()
                .expect("BUG: chunks never yields an empty chunk");
            let volumes: Vec<f64> = chunk.iter().filter_map(|b| b.volume).collect();
            CandleBar {
                t_secs: first.t_secs,
                open: first.open,
                high: chunk
                    .iter()
                    .map(|b| b.high)
                    .fold(f64::NEG_INFINITY, f64::max),
                low: chunk.iter().map(|b| b.low).fold(f64::INFINITY, f64::min),
                close: last.close,
                volume: (!volumes.is_empty()).then(|| volumes.iter().sum()),
            }
        })
        .collect()
}

/// Vertical price domain: low/high extremes widened to include the
/// current price (so the price line and badge stay inside the plot).
#[must_use]
pub fn price_range(bars: &[CandleBar], current: f64) -> Option<(f64, f64)> {
    let low = bars.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    let high = bars
        .iter()
        .map(|b| b.high)
        .fold(f64::NEG_INFINITY, f64::max);
    if !low.is_finite() || !high.is_finite() {
        return None;
    }
    Some((low.min(current), high.max(current)))
}

/// Screen y for a price inside a rect spanning `rect_y..rect_y + rect_h`
/// (highest price at the top). A degenerate flat range centres.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "prices fit f32 at pixel precision"
)]
pub fn price_to_y(price: f64, min: f64, max: f64, rect_y: f32, rect_h: f32) -> f32 {
    let range = max - min;
    if range <= 0.0 {
        return rect_y + rect_h / 2.0;
    }
    let normalized = ((price - min) / range) as f32;
    rect_y + rect_h - normalized * rect_h
}

/// Keep the centre of a label or badge inside a vertical extent.
#[must_use]
pub fn clamp_center_y(y: f32, height: f32, item_height: f32) -> f32 {
    let half = item_height / 2.0;
    y.clamp(half, (height - half).max(half))
}

/// One drawable candle: a centred wick span and a body rect, both in
/// screen coordinates, with the bar's direction.
pub struct CandleShape {
    pub x_center: f32,
    pub body_w: f32,
    pub body_top: f32,
    pub body_h: f32,
    pub wick_top: f32,
    pub wick_h: f32,
    pub up: bool,
}

/// Project bars into candle shapes inside the plot rect. Bodies take 70%
/// of the slot with a 1px minimum; body height is also at least 1px so a
/// doji stays visible.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "bar counts stay far below f32 precision loss"
)]
pub fn candle_shapes(
    bars: &[CandleBar],
    rect_x: f32,
    rect_w: f32,
    rect_y: f32,
    rect_h: f32,
    min: f64,
    max: f64,
) -> Vec<CandleShape> {
    let slot = rect_w / bars.len().max(1) as f32;
    let body_w = (slot * 0.7).max(1.0);
    bars.iter()
        .enumerate()
        .map(|(i, b)| {
            let y_open = price_to_y(b.open, min, max, rect_y, rect_h);
            let y_close = price_to_y(b.close, min, max, rect_y, rect_h);
            let y_high = price_to_y(b.high, min, max, rect_y, rect_h);
            let y_low = price_to_y(b.low, min, max, rect_y, rect_h);
            CandleShape {
                x_center: rect_x + slot * (i as f32 + 0.5),
                body_w,
                body_top: y_open.min(y_close),
                body_h: (y_open - y_close).abs().max(1.0),
                wick_top: y_high,
                wick_h: (y_low - y_high).max(1.0),
                up: b.close >= b.open,
            }
        })
        .collect()
}

/// Round tick values (1/2/5 × 10ⁿ steps) covering `min..=max`, aiming
/// for about `target` intervals. Empty on a degenerate range.
#[must_use]
#[expect(clippy::cast_precision_loss, reason = "tick targets are single digits")]
#[expect(
    clippy::float_cmp,
    reason = "exact equality detects an increment stalled below the ULP"
)]
pub fn nice_ticks(min: f64, max: f64, target: usize) -> Vec<f64> {
    if !(min.is_finite() && max.is_finite()) || max <= min || target == 0 {
        return Vec::new();
    }
    let raw_step = (max - min) / target as f64;
    let magnitude = 10f64.powf(raw_step.log10().floor());
    // A subnormal range underflows `magnitude` to zero, leaving no candidate
    // step ≥ `raw_step`; such a range has no drawable grid.
    let Some(step) = [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|m| m * magnitude)
        .find(|s| *s >= raw_step)
    else {
        return Vec::new();
    };
    let mut value = (min / step).ceil() * step;
    let mut out = Vec::new();
    while value <= max + step * 1e-9 {
        out.push(value);
        let next = value + step;
        if next == value {
            break;
        }
        value = next;
    }
    out
}

/// Split `start..end` into 1-D dash segments (`on` painted, `off` gap):
/// the engine has no dashed-stroke primitive, so dashes are drawn as
/// thin rects from these spans.
#[must_use]
pub fn dash_spans(start: f32, end: f32, on: f32, off: f32) -> Vec<(f32, f32)> {
    let stride = on + off;
    assert!(stride > 0.0, "dash pattern must advance");

    let mut out = Vec::new();
    let mut x = start;
    while x < end {
        out.push((x, (x + on).min(end)));
        x += stride;
    }
    out
}

/// Bar heights for the volume strip, normalized to the maximum volume.
/// `None` entries (missing volume) draw nothing. All-`None` input means
/// the caller hides the strip entirely.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "volumes normalize to small pixel heights"
)]
pub fn volume_heights(bars: &[CandleBar], strip_h: f32) -> Vec<Option<f32>> {
    let max = bars.iter().filter_map(|b| b.volume).fold(0.0_f64, f64::max);
    bars.iter()
        .map(|b| {
            let v = b.volume?;
            if max <= 0.0 {
                return None;
            }
            Some(((v / max) as f32) * strip_h)
        })
        .collect()
}

/// Centres within this many pixels of the left edge would clip; an
/// affected boundary may slide right within its own run instead.
const LEFT_CLIP_PX: f32 = 40.0;

/// Indices of bars that get an x-axis label: the first bar of each run
/// of equal label texts (a year/month/day boundary), thinned so kept
/// labels stay at least `min_px` apart, skipping empty texts. A boundary
/// whose centre falls within [`LEFT_CLIP_PX`] of the left edge slides to
/// the first bar of its run that clears the edge (the text stays
/// truthful), but yields entirely when the slide would land within
/// `min_px` of the next true boundary or the run never clears the edge.
/// Boundaries thinned by the `min_px` density rule never relocate — that
/// would make the spacing uneven again.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "bar counts stay far below f32 precision loss"
)]
pub fn label_indices(texts: &[String], plot_w: f32, min_px: f32) -> Vec<usize> {
    if texts.is_empty() || plot_w <= 0.0 {
        return Vec::new();
    }
    let slot = plot_w / texts.len() as f32;
    let centre = |i: usize| slot * (i as f32 + 0.5);
    let mut out = Vec::new();
    let mut last_kept_x: Option<f32> = None;
    for (i, text) in texts.iter().enumerate() {
        if text.is_empty() || (i > 0 && texts[i - 1] == *text) {
            continue;
        }
        let x = centre(i);
        if last_kept_x.is_some_and(|last| x - last < min_px) {
            continue;
        }
        let kept = if x < LEFT_CLIP_PX {
            let Some(j) = (i + 1..texts.len())
                .take_while(|&j| texts[j] == *text)
                .find(|&j| centre(j) >= LEFT_CLIP_PX)
            else {
                continue;
            };
            let next_boundary =
                (i + 1..texts.len()).find(|&k| !texts[k].is_empty() && texts[k] != texts[k - 1]);
            if next_boundary.is_some_and(|k| centre(k) - centre(j) < min_px) {
                continue;
            }
            j
        } else {
            i
        };
        out.push(kept);
        last_kept_x = Some(centre(kept));
    }
    out
}

/// Minimum centre-to-centre spacing for x-axis labels so adjacent
/// centre-aligned texts do not overlap: the widest label's estimated pixel
/// width plus a small gap. `max_chars` is the longest label's character count
/// and `axis_font` its font size; `CHAR_W` approximates a proportional glyph's
/// advance as a fraction of the font size. `floor` keeps narrow labels at their
/// established spacing so only wide labels (e.g. `MonthYear` "Sep 2021") widen.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "label char counts and font sizes stay far below f32 precision loss"
)]
pub fn label_min_px(max_chars: usize, axis_font: u32, floor: f32) -> f32 {
    const CHAR_W: f32 = 0.6;
    const LABEL_GAP: f32 = 8.0;
    let estimated = max_chars as f32 * axis_font as f32 * CHAR_W + LABEL_GAP;
    estimated.max(floor)
}

fn push_pad2(out: &mut String, n: u8) {
    if n < 10 {
        out.push('0');
    }
    push_uint(out, u64::from(n));
}

/// What an x-axis label shows, per the spec's period tiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    Time,
    DayMonth,
    MonthYear,
    Year,
}

#[must_use]
pub fn label_kind(period: Period) -> LabelKind {
    match period {
        Period::H1 | Period::H3 | Period::H6 | Period::H12 | Period::D1 => LabelKind::Time,
        Period::D3
        | Period::D7
        | Period::D14
        | Period::Mo1
        | Period::Mo3
        | Period::Mo6
        | Period::Mo9
        | Period::Y1 => LabelKind::DayMonth,
        Period::Y2 | Period::Y3 | Period::Y5 => LabelKind::MonthYear,
        Period::Y10 | Period::Y25 | Period::Full => LabelKind::Year,
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn month_abbr(month: u8) -> &'static str {
    debug_assert!((1..=12).contains(&month), "BUG: month out of 1..=12 range");
    MONTHS
        .get(usize::from(month).wrapping_sub(1))
        .unwrap_or(&"?")
}

/// Whether the operator's date format puts the month before the day: the
/// `M/D/YYYY` order and the year-first ISO orders. Day-first formats (and an
/// unset preference) read day-first.
#[must_use]
pub fn month_leads(format: Option<DateFormat>) -> bool {
    matches!(
        format,
        Some(
            DateFormat::MDYyyySlash
                | DateFormat::YyyyMDSlash
                | DateFormat::YyyyMmDdDot
                | DateFormat::YyyyMmDdDash
        )
    )
}

/// Render one label from a wall-clock view. The 12-hour clock drops the
/// meridiem, matching the SDK `format_time` convention; `month_first` (from
/// [`month_leads`]) orders the day-month labels per the operator's date
/// format.
#[must_use]
pub fn label_text(
    kind: LabelKind,
    ldt: &LocalDateTime,
    time_format: TimeFormat,
    month_first: bool,
) -> String {
    let mut out = String::new();
    match kind {
        LabelKind::Time => {
            let hour = match time_format {
                TimeFormat::Hour24 => ldt.hour,
                TimeFormat::Hour12 => match ldt.hour % 12 {
                    0 => 12,
                    h => h,
                },
            };
            push_pad2(&mut out, hour);
            out.push(':');
            push_pad2(&mut out, ldt.minute);
        }
        LabelKind::DayMonth if month_first => {
            out.push_str(month_abbr(ldt.month));
            out.push(' ');
            push_pad2(&mut out, ldt.day);
        }
        LabelKind::DayMonth => {
            push_pad2(&mut out, ldt.day);
            out.push(' ');
            out.push_str(month_abbr(ldt.month));
        }
        LabelKind::MonthYear => {
            out.push_str(month_abbr(ldt.month));
            out.push(' ');
            push_uint(&mut out, u64::from(ldt.year));
        }
        LabelKind::Year => push_uint(&mut out, u64::from(ldt.year)),
    }
    out
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::LocalDateTime;
    use bmc_wasm_sdk::system::TimeFormat;
    use prices::candle::CandleBar;
    use prices::period::Period;

    #[test]
    fn month_abbr_maps_valid_months() {
        assert_eq!(month_abbr(1), "Jan");
        assert_eq!(month_abbr(12), "Dec");
    }

    #[test]
    #[should_panic(expected = "BUG: month out of 1..=12 range")]
    fn month_abbr_asserts_on_zero() {
        let _ = month_abbr(0);
    }

    use super::*;

    fn bar(
        t_secs: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: Option<f64>,
    ) -> CandleBar {
        CandleBar {
            t_secs,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    fn ldt(hour: u8, minute: u8) -> LocalDateTime {
        LocalDateTime {
            year: 2026,
            month: 6,
            day: 5,
            hour,
            minute,
            second: 0,
            weekday: 4,
        }
    }

    #[test]
    fn merge_keeps_short_series_and_composes_ohlcv_exactly() {
        let bars = vec![
            bar(0, 10.0, 12.0, 9.0, 11.0, Some(1.0)),
            bar(60, 11.0, 15.0, 10.0, 14.0, None),
            bar(120, 14.0, 14.5, 8.0, 9.0, Some(2.0)),
        ];
        assert_eq!(merge_bars(&bars, 3), bars);
        let merged = merge_bars(&bars, 2);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0], bar(0, 10.0, 15.0, 9.0, 14.0, Some(1.0)));
        assert_eq!(merged[1], bar(120, 14.0, 14.5, 8.0, 9.0, Some(2.0)));
    }

    #[test]
    fn merge_with_no_volume_anywhere_stays_none() {
        let bars = vec![
            bar(0, 1.0, 1.0, 1.0, 1.0, None),
            bar(60, 1.0, 1.0, 1.0, 1.0, None),
        ];
        assert_eq!(merge_bars(&bars, 1)[0].volume, None);
    }

    #[test]
    fn price_range_spans_extremes_and_current() {
        let bars = vec![bar(0, 10.0, 12.0, 9.0, 11.0, None)];
        assert_eq!(price_range(&bars, 11.0), Some((9.0, 12.0)));
        assert_eq!(price_range(&bars, 20.0), Some((9.0, 20.0)));
        assert_eq!(price_range(&[], 5.0), None);
    }

    #[test]
    fn price_to_y_inverts_prices_into_screen_coords() {
        assert!((price_to_y(9.0, 9.0, 12.0, 10.0, 30.0) - 40.0).abs() < 1e-6);
        assert!((price_to_y(12.0, 9.0, 12.0, 10.0, 30.0) - 10.0).abs() < 1e-6);
    }

    #[test]
    fn label_centres_stay_inside_the_plot_at_price_extremes() {
        assert_eq!(clamp_center_y(0.0, 100.0, 28.0), 14.0);
        assert_eq!(clamp_center_y(100.0, 100.0, 28.0), 86.0);
        assert_eq!(clamp_center_y(50.0, 100.0, 28.0), 50.0);
    }

    #[test]
    fn candle_slots_spread_across_the_width_with_70pct_bodies() {
        let bars = vec![
            bar(0, 10.0, 12.0, 9.0, 11.0, None),
            bar(60, 11.0, 11.0, 10.0, 10.0, None),
        ];
        let shapes = candle_shapes(&bars, 0.0, 100.0, 0.0, 30.0, 9.0, 12.0);
        assert_eq!(shapes.len(), 2);
        assert!((shapes[0].x_center - 25.0).abs() < 1e-6);
        assert!((shapes[1].x_center - 75.0).abs() < 1e-6);
        assert!((shapes[0].body_w - 35.0).abs() < 1e-6);
        assert!(shapes[0].up);
        assert!(!shapes[1].up);
    }

    #[test]
    fn ticks_land_on_round_steps_inside_the_range() {
        assert_eq!(
            nice_ticks(0.0, 10.0, 5),
            vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]
        );
        assert_eq!(nice_ticks(97.0, 121.0, 4), vec![100.0, 110.0, 120.0]);
        assert_eq!(
            nice_ticks(97.0, 121.0, 6),
            vec![100.0, 105.0, 110.0, 115.0, 120.0]
        );
        assert!(nice_ticks(5.0, 5.0, 4).is_empty());
    }

    #[test]
    fn ticks_terminate_on_a_near_flat_range() {
        assert!(nice_ticks(1.0, 1.0 + f64::EPSILON, 4).len() <= 6);
    }

    #[test]
    fn subnormal_range_yields_no_ticks_instead_of_panicking() {
        // A step this small underflows the magnitude to 0.0; candle values
        // come from the network, so the degenerate range must not panic.
        assert!(nice_ticks(0.0, f64::from_bits(4), 4).is_empty());
    }

    #[test]
    fn dashes_alternate_and_clip_at_the_end() {
        assert_eq!(
            dash_spans(0.0, 10.0, 4.0, 4.0),
            vec![(0.0, 4.0), (8.0, 10.0)]
        );
        assert!(dash_spans(5.0, 5.0, 4.0, 4.0).is_empty());
    }

    #[test]
    #[should_panic(expected = "dash pattern must advance")]
    fn dashes_reject_a_non_advancing_pattern() {
        let _ = dash_spans(0.0, 10.0, 0.0, 0.0);
    }

    #[test]
    fn volume_heights_normalize_to_the_max_and_skip_missing() {
        let bars = vec![
            bar(0, 1.0, 1.0, 1.0, 1.0, Some(2.0)),
            bar(60, 1.0, 1.0, 1.0, 1.0, None),
            bar(120, 1.0, 1.0, 1.0, 1.0, Some(4.0)),
        ];
        assert_eq!(
            volume_heights(&bars, 20.0),
            vec![Some(10.0), None, Some(20.0)]
        );
        assert!(
            volume_heights(&bars[1..2], 20.0)
                .iter()
                .all(Option::is_none)
        );
    }

    #[test]
    fn label_indices_relocate_edge_clipped_years_within_their_runs() {
        // BTC full period at full size: 142 monthly bars starting Sep 2014
        // (4 bars of 2014, 12 each of 2015–2025, 6 of 2026) on 1144px.
        let mut texts = Vec::new();
        for (year, months) in std::iter::once((2014_u32, 4_usize))
            .chain((2015..=2025).map(|y| (y, 12)))
            .chain(std::iter::once((2026, 6)))
        {
            let mut t = String::new();
            push_uint(&mut t, u64::from(year));
            for _ in 0..months {
                texts.push(t.clone());
            }
        }
        assert_eq!(texts.len(), 142);
        let kept = label_indices(&texts, 1144.0, 70.0);
        // slot ≈ 8.06px: the 2014 (centre ≈ 4px) and 2015 (centre ≈ 36px)
        // boundaries fall inside the 40px left margin. The 40px rule is a
        // clipping constraint, not a density one: 2015 slides right within
        // its own run to the first bar past the margin (index 5, ≈ 44px),
        // which still leaves ≥ 70px to the 2016 boundary (≈ 133px). 2014's
        // run never clears the margin, so it is dropped. Every later year's
        // first bar is kept, 12 bars apart — even spacing.
        assert_eq!(
            kept,
            vec![5, 16, 28, 40, 52, 64, 76, 88, 100, 112, 124, 136]
        );
        assert!(kept[1..].windows(2).all(|w| w[1] - w[0] == 12));
    }

    #[test]
    fn label_indices_relocate_the_leading_day_of_an_hourly_week() {
        // 7d period: 168 hourly bars (7 days × 24) starting at midnight
        // 06 Jun on 1144px → slot ≈ 6.81px. The 06 Jun boundary (index 0,
        // centre ≈ 3.4px) would be clipped at the left edge; without
        // relocation the chart opens with an unlabeled day. It slides to
        // the first bar past the 40px margin (index 6, ≈ 44px), leaving
        // ≈ 123px to the 07 Jun boundary — both are kept.
        let mut texts = Vec::new();
        for day in 6_u8..=12 {
            let mut t = String::new();
            push_pad2(&mut t, day);
            t.push_str(" Jun");
            for _ in 0..24 {
                texts.push(t.clone());
            }
        }
        assert_eq!(texts.len(), 168);
        assert_eq!(
            label_indices(&texts, 1144.0, 70.0),
            vec![6, 24, 48, 72, 96, 120, 144]
        );
    }

    #[test]
    fn label_indices_with_unique_texts_degenerate_to_a_stride() {
        // 100 unique texts on 700px → 7px slots; every bar is a boundary.
        // First centre past 40px is i=6 (45.5px); 70px min gap → every 10th.
        let texts: Vec<String> = (0..100_u32)
            .map(|i| {
                let mut t = String::from("t");
                push_uint(&mut t, u64::from(i));
                t
            })
            .collect();
        assert_eq!(
            label_indices(&texts, 700.0, 70.0),
            vec![6, 16, 26, 36, 46, 56, 66, 76, 86, 96]
        );
    }

    #[test]
    fn label_indices_yield_the_edge_label_to_a_close_true_boundary() {
        assert!(label_indices(&[], 700.0, 70.0).is_empty());
        let empties = vec![String::new(); 5];
        assert!(label_indices(&empties, 700.0, 70.0).is_empty());
        // 60px slots: "A"'s boundary (index 0, centre 30px) is edge-clipped
        // and could slide to index 1 (centre 90px), but that lands within
        // 70px of the true "B" boundary (centre 150px) — the true boundary
        // wins and "A" is dropped.
        let texts: Vec<String> = ["A", "A", "B", "B"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(label_indices(&texts, 240.0, 70.0), vec![2]);
        assert!(label_indices(&texts, 0.0, 70.0).is_empty());
        // a label never slides past its own run: "A"'s run ends at index 0,
        // so it cannot borrow a "B" bar to clear the edge.
        let texts: Vec<String> = ["A", "B", "B", "B"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(label_indices(&texts, 240.0, 70.0), vec![1]);
    }

    #[test]
    fn label_min_px_widens_for_wide_labels_and_floors_narrow_ones() {
        // "Sep 2021" (8 chars) at axis_font 20 must demand more than the 70px
        // floor so adjacent centre-aligned labels do not overlap.
        let wide = label_min_px(8, 20, 70.0);
        assert!(wide > 70.0, "wide MonthYear label must widen spacing");
        assert!((wide - (8.0 * 20.0 * 0.6 + 8.0)).abs() < 1e-3);
        // "2021" (4 chars) estimates below the floor, so the floor wins.
        assert_eq!(label_min_px(4, 20, 70.0), 70.0);
        // Spacing scales down with the font (smaller size bands).
        assert!(label_min_px(8, 10, 70.0) < wide);
    }

    #[test]
    fn label_kinds_follow_the_period_tiers() {
        assert_eq!(label_kind(Period::H1), LabelKind::Time);
        assert_eq!(label_kind(Period::D1), LabelKind::Time);
        assert_eq!(label_kind(Period::D3), LabelKind::DayMonth);
        assert_eq!(label_kind(Period::Y1), LabelKind::DayMonth);
        assert_eq!(label_kind(Period::Y2), LabelKind::MonthYear);
        assert_eq!(label_kind(Period::Y5), LabelKind::MonthYear);
        assert_eq!(label_kind(Period::Y10), LabelKind::Year);
        assert_eq!(label_kind(Period::Full), LabelKind::Year);
    }

    #[test]
    fn label_text_per_kind_and_clock_format() {
        assert_eq!(
            label_text(LabelKind::Time, &ldt(14, 5), TimeFormat::Hour24, false),
            "14:05"
        );
        assert_eq!(
            label_text(LabelKind::Time, &ldt(14, 5), TimeFormat::Hour12, false),
            "02:05"
        );
        assert_eq!(
            label_text(LabelKind::Time, &ldt(0, 30), TimeFormat::Hour12, false),
            "12:30"
        );
        assert_eq!(
            label_text(LabelKind::DayMonth, &ldt(0, 0), TimeFormat::Hour24, false),
            "05 Jun"
        );
        assert_eq!(
            label_text(LabelKind::MonthYear, &ldt(0, 0), TimeFormat::Hour24, false),
            "Jun 2026"
        );
        assert_eq!(
            label_text(LabelKind::Year, &ldt(0, 0), TimeFormat::Hour24, false),
            "2026"
        );
    }

    #[test]
    fn month_first_formats_flip_only_the_day_month_label() {
        // An `M/D/YYYY` (or year-first) operator reads "Jun 05", not "05 Jun";
        // the month-year and year labels have no day to reorder.
        assert!(month_leads(Some(DateFormat::MDYyyySlash)));
        assert!(month_leads(Some(DateFormat::YyyyMmDdDash)));
        assert!(!month_leads(Some(DateFormat::DdMmYyyyDot)));
        assert!(!month_leads(None));
        assert_eq!(
            label_text(LabelKind::DayMonth, &ldt(0, 0), TimeFormat::Hour24, true),
            "Jun 05"
        );
        assert_eq!(
            label_text(LabelKind::MonthYear, &ldt(0, 0), TimeFormat::Hour24, true),
            "Jun 2026"
        );
    }
}
