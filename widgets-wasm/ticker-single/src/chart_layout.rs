// Copyright (C) 2026  Braiins Systems s.r.o.

//! Pure geometry and selection math for the candlestick chart view.
//! No host calls — everything here is unit-testable on the host.

use bmc_wasm_sdk::LocalDateTime;
use bmc_wasm_sdk::system::TimeFormat;
use prices::candle::CandleBar;
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
/// of the slot (min 1px, like deckfeeder); body height min 1px so a
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
    let step = [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|m| m * magnitude)
        .find(|s| *s >= raw_step)
        .expect("BUG: 10×magnitude always >= raw_step");
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
    let mut out = Vec::new();
    let mut x = start;
    while x < end {
        out.push((x, (x + on).min(end)));
        x += on + off;
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

/// Indices of bars that get an x-axis label: one per `min_px` of plot
/// width, skipping any label whose centre falls within 40px of the left
/// edge (deckfeeder's crowding rule).
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bar counts and pixel widths are small positive values"
)]
pub fn label_indices(n: usize, plot_w: f32, min_px: f32) -> Vec<usize> {
    if n == 0 || plot_w <= 0.0 {
        return Vec::new();
    }
    let slot = plot_w / n as f32;
    let stride = (min_px / slot).ceil().max(1.0) as usize;
    (0..n)
        .step_by(stride)
        .filter(|&i| slot * (i as f32 + 0.5) >= 40.0)
        .collect()
}

// The SDK's `format::push_int`/`push_pad2` live in a wasm-gated module
// (`sdk/src/lib.rs` gates `pub mod format` on wasm32), so this host-tested
// module carries its own digit helpers. No `std` format macros — the
// no-fmt-in-wasm gate rejects them.
fn push_int(out: &mut String, n: u32) {
    let mut digits = [0_u8; 10];
    let mut i = digits.len();
    let mut rest = n;
    loop {
        i -= 1;
        digits[i] = b'0' + u8::try_from(rest % 10).expect("BUG: a decimal digit fits u8");
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    for &d in &digits[i..] {
        out.push(char::from(d));
    }
}

fn push_pad2(out: &mut String, n: u8) {
    if n < 10 {
        out.push('0');
    }
    push_int(out, u32::from(n));
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
    MONTHS
        .get(usize::from(month).wrapping_sub(1))
        .unwrap_or(&"?")
}

/// Render one label from a wall-clock view. The 12-hour clock drops the
/// meridiem, matching the SDK `format_time` convention.
#[must_use]
pub fn label_text(kind: LabelKind, ldt: &LocalDateTime, time_format: TimeFormat) -> String {
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
        LabelKind::DayMonth => {
            push_pad2(&mut out, ldt.day);
            out.push(' ');
            out.push_str(month_abbr(ldt.month));
        }
        LabelKind::MonthYear => {
            out.push_str(month_abbr(ldt.month));
            out.push(' ');
            push_int(&mut out, u32::from(ldt.year));
        }
        LabelKind::Year => push_int(&mut out, u32::from(ldt.year)),
    }
    out
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::LocalDateTime;
    use bmc_wasm_sdk::system::TimeFormat;
    use prices::candle::CandleBar;
    use prices::period::Period;

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
    fn dashes_alternate_and_clip_at_the_end() {
        assert_eq!(
            dash_spans(0.0, 10.0, 4.0, 4.0),
            vec![(0.0, 4.0), (8.0, 10.0)]
        );
        assert!(dash_spans(5.0, 5.0, 4.0, 4.0).is_empty());
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
    fn label_indices_stride_to_70px_and_skip_the_crowded_left_edge() {
        // 100 bars on 700px → 7px slots → stride 10.
        assert_eq!(
            label_indices(100, 700.0, 70.0),
            vec![10, 20, 30, 40, 50, 60, 70, 80, 90]
        );
        // Wide slots keep every index, but index 0 sits at x=30 (< 40px) and is dropped.
        assert_eq!(
            label_indices(10, 600.0, 50.0),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9]
        );
        assert!(label_indices(0, 700.0, 70.0).is_empty());
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
            label_text(LabelKind::Time, &ldt(14, 5), TimeFormat::Hour24),
            "14:05"
        );
        assert_eq!(
            label_text(LabelKind::Time, &ldt(14, 5), TimeFormat::Hour12),
            "02:05"
        );
        assert_eq!(
            label_text(LabelKind::Time, &ldt(0, 30), TimeFormat::Hour12),
            "12:30"
        );
        assert_eq!(
            label_text(LabelKind::DayMonth, &ldt(0, 0), TimeFormat::Hour24),
            "05 Jun"
        );
        assert_eq!(
            label_text(LabelKind::MonthYear, &ldt(0, 0), TimeFormat::Hour24),
            "Jun 2026"
        );
        assert_eq!(
            label_text(LabelKind::Year, &ldt(0, 0), TimeFormat::Hour24),
            "2026"
        );
    }
}
