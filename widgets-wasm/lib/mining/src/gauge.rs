// Copyright (C) 2026  Braiins Systems s.r.o.

//! Gauge math: state classification, the lit-tick count, and the 28 absolute
//! tick spans. Holds no renderer/SDK type so it unit-tests on the host; callers
//! map `GaugeState` to colors (see `style`) and wrap the spans in `ArcSegments`
//! at the draw site.
//!
//! The gauge sweep is anchored to the miner's tuner constraints: the configured
//! `min` sits a quarter of the way around, `default` three-quarters, `max` at
//! the full sweep, with linear interpolation between. State is hashrate-only —
//! within ±`GOOD_BAND_PERCENT` of the default target reads `Good`, beyond it
//! `Over`/`Underclocked`.

pub const TICK_COUNT: usize = 28;

// Half-width of the "Good" band around the default hashrate target, in percent:
// within ±N% of default reads Good, beyond it Over/Underclocked. Product-tunable.
pub const GOOD_BAND_PERCENT: f64 = 3.0;

// Gauge sweep fractions for the three tuner-target anchors.
const MIN_ANCHOR: f32 = 0.25;
const DEFAULT_ANCHOR: f32 = 0.75;
const MAX_ANCHOR: f32 = 1.0;

// Hashrate (TH/s) at or below this reads as OFF rather than a live miner: a
// stopped miner reports ~0, and tiny residual values are not meaningful hashing.
const OFF_MAX_THS: f64 = 0.01;

// Portion of each tick slot left as the inter-tick gap; the rest is the lit
// segment. ~0.04 yields a ~2px gap at the ring radius, matching the design.
const TICK_GAP_FRACTION: f32 = 0.04;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GaugeState {
    NotAvailable,
    Off,
    Underclocked,
    Good,
    Overclocked,
}

pub struct Gauge {
    pub state: GaugeState,
    pub lit_count: usize,
}

// A tuner constraint's min / default / max for one quantity (hashrate in TH/s
// or power in W). The gauge anchors its sweep to these three points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetRange {
    pub min: f64,
    pub default: f64,
    pub max: f64,
}

// Sweep fraction (0..1) for `value` against the target anchors, piecewise-linear
// through `0 -> 0`, `min -> 1/4`, `default -> 3/4`, `max -> 4/4`. A value at or
// below zero maps to 0, at or above max to the full sweep.
#[must_use]
pub fn target_fraction(value: f64, range: &TargetRange) -> f32 {
    let fraction = if value <= 0.0 {
        0.0
    } else if value <= range.min {
        lerp_segment(value, 0.0, range.min, 0.0, MIN_ANCHOR)
    } else if value <= range.default {
        lerp_segment(value, range.min, range.default, MIN_ANCHOR, DEFAULT_ANCHOR)
    } else if value <= range.max {
        lerp_segment(value, range.default, range.max, DEFAULT_ANCHOR, MAX_ANCHOR)
    } else {
        MAX_ANCHOR
    };
    fraction.clamp(0.0, MAX_ANCHOR)
}

// Linear map of `value` in `[lo, hi]` onto `[lo_anchor, hi_anchor]`. A zero-width
// or inverted segment (`hi <= lo`, e.g. a degenerate `min == default`) snaps to
// the upper anchor rather than dividing by zero.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the interpolation parameter is a clamped 0..1 ratio that loses no meaningful precision in f32"
)]
fn lerp_segment(value: f64, lo: f64, hi: f64, lo_anchor: f32, hi_anchor: f32) -> f32 {
    if hi <= lo {
        return hi_anchor;
    }
    let t = ((value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
    lo_anchor + t * (hi_anchor - lo_anchor)
}

// Classify the gauge state from the hashrate against its tuner target. `Off`
// (a stopped miner) takes precedence over a missing target; either input absent
// otherwise reads `NotAvailable`.
#[must_use]
pub fn gauge_state(hashrate_ths: Option<f64>, hashrate_target: Option<&TargetRange>) -> GaugeState {
    let Some(hashrate) = hashrate_ths else {
        return GaugeState::NotAvailable;
    };
    if hashrate <= OFF_MAX_THS {
        return GaugeState::Off;
    }
    let Some(target) = hashrate_target else {
        return GaugeState::NotAvailable;
    };
    if hashrate >= target.default * (1.0 + GOOD_BAND_PERCENT / 100.0) {
        GaugeState::Overclocked
    } else if hashrate <= target.default * (1.0 - GOOD_BAND_PERCENT / 100.0) {
        GaugeState::Underclocked
    } else {
        GaugeState::Good
    }
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "lit-tick count is a small clamped non-negative value"
)]
fn lit_count_from_fraction(fraction: f32) -> usize {
    (fraction.clamp(0.0, 1.0) * TICK_COUNT as f32).round() as usize
}

// Convenience for the single-ring face: the hashrate state and its lit-tick
// count in one call. `Off` lights a single tick, `NotAvailable` none.
#[must_use]
pub fn gauge(hashrate_ths: Option<f64>, hashrate_target: Option<&TargetRange>) -> Gauge {
    let state = gauge_state(hashrate_ths, hashrate_target);
    let lit_count = match state {
        GaugeState::NotAvailable => 0,
        GaugeState::Off => 1,
        GaugeState::Underclocked | GaugeState::Good | GaugeState::Overclocked => {
            let fraction = hashrate_ths
                .zip(hashrate_target)
                .map_or(0.0, |(hashrate, range)| target_fraction(hashrate, range));
            lit_count_from_fraction(fraction)
        }
    };
    Gauge { state, lit_count }
}

// Sweep end angle (radians, clockwise from 12 o'clock) of the lit overlay for
// `lit_count` ticks: the slot boundary just past the last lit tick. Encoding
// the fill as the arc sweep lets a host transition interpolate it on change;
// the lit prefix spans all fall within `[0, this]`.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "tick index/count are small UI geometry counts"
)]
pub fn lit_sweep_end(lit_count: usize) -> f32 {
    let slot = std::f32::consts::TAU / TICK_COUNT as f32;
    lit_count.min(TICK_COUNT) as f32 * slot
}

// The 28 absolute tick spans, an even partition of 0..TAU clockwise from 12
// o'clock with a per-slot gap. Both gauge sweeps draw from this one list (base
// = all, lit = a prefix slice) so lit ticks register exactly over base ticks.
#[expect(
    clippy::cast_precision_loss,
    reason = "tick index/count are small UI geometry counts"
)]
pub const TICK_SPANS: [(f32, f32); TICK_COUNT] = {
    let slot = std::f32::consts::TAU / TICK_COUNT as f32;
    let seg = slot * (1.0 - TICK_GAP_FRACTION);
    let mut spans = [(0.0, 0.0); TICK_COUNT];
    let mut i = 0;
    while i < TICK_COUNT {
        let start = i as f32 * slot;
        spans[i] = (start, start + seg);
        i += 1;
    }
    spans
};

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_frac(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    const RANGE: TargetRange = TargetRange {
        min: 10.0,
        default: 30.0,
        max: 40.0,
    };

    #[test]
    fn target_fraction_lands_on_each_anchor() {
        assert_frac(target_fraction(0.0, &RANGE), 0.0);
        assert_frac(target_fraction(10.0, &RANGE), MIN_ANCHOR);
        assert_frac(target_fraction(30.0, &RANGE), DEFAULT_ANCHOR);
        assert_frac(target_fraction(40.0, &RANGE), MAX_ANCHOR);
    }

    #[test]
    fn target_fraction_interpolates_linearly_between_anchors() {
        // Halfway 0->min, min->default, default->max respectively.
        assert_frac(target_fraction(5.0, &RANGE), 0.125);
        assert_frac(target_fraction(20.0, &RANGE), 0.5);
        assert_frac(target_fraction(35.0, &RANGE), 0.875);
    }

    #[test]
    fn target_fraction_clamps_below_zero_and_above_max() {
        assert_frac(target_fraction(-5.0, &RANGE), 0.0);
        assert_frac(target_fraction(100.0, &RANGE), MAX_ANCHOR);
    }

    #[test]
    fn target_fraction_never_produces_nan_on_degenerate_ranges() {
        let degenerate = [
            TargetRange {
                min: 30.0,
                default: 30.0,
                max: 40.0,
            },
            TargetRange {
                min: 10.0,
                default: 30.0,
                max: 30.0,
            },
            TargetRange {
                min: 30.0,
                default: 20.0,
                max: 10.0,
            },
        ];
        for range in degenerate {
            for value in [-1.0, 0.0, 15.0, 25.0, 35.0, 50.0] {
                let f = target_fraction(value, &range);
                assert!(f.is_finite(), "non-finite for {range:?} @ {value}");
                assert!(
                    (0.0..=1.0).contains(&f),
                    "out of range {f} for {range:?} @ {value}"
                );
            }
        }
    }

    #[test]
    fn state_not_available_when_hashrate_missing() {
        assert_eq!(gauge_state(None, Some(&RANGE)), GaugeState::NotAvailable);
    }

    #[test]
    fn state_not_available_when_hashing_but_target_missing() {
        assert_eq!(gauge_state(Some(30.0), None), GaugeState::NotAvailable);
    }

    #[test]
    fn state_off_takes_precedence_over_missing_target() {
        for hr in [0.0, 0.005, 0.01] {
            assert_eq!(gauge_state(Some(hr), None), GaugeState::Off, "@ {hr}");
        }
    }

    #[test]
    fn state_good_within_band_over_and_under_outside() {
        let target = TargetRange {
            min: 50.0,
            default: 100.0,
            max: 120.0,
        };
        let over_edge = 100.0 * (1.0 + GOOD_BAND_PERCENT / 100.0);
        let under_edge = 100.0 * (1.0 - GOOD_BAND_PERCENT / 100.0);
        // Just inside the band on both sides is Good.
        assert_eq!(gauge_state(Some(100.0), Some(&target)), GaugeState::Good);
        assert_eq!(
            gauge_state(Some(over_edge - 0.1), Some(&target)),
            GaugeState::Good
        );
        assert_eq!(
            gauge_state(Some(under_edge + 0.1), Some(&target)),
            GaugeState::Good
        );
        // At and beyond the edges flips to Over/Underclocked.
        assert_eq!(
            gauge_state(Some(over_edge), Some(&target)),
            GaugeState::Overclocked
        );
        assert_eq!(
            gauge_state(Some(under_edge), Some(&target)),
            GaugeState::Underclocked
        );
    }

    #[test]
    fn gauge_lit_count_tracks_target_fraction() {
        // default sits at 3/4 -> round(0.75 * 28) = 21; min at 1/4 -> 7; max -> 28.
        assert_eq!(gauge(Some(10.0), Some(&RANGE)).lit_count, 7);
        assert_eq!(gauge(Some(30.0), Some(&RANGE)).lit_count, 21);
        assert_eq!(gauge(Some(40.0), Some(&RANGE)).lit_count, 28);
    }

    #[test]
    fn gauge_lit_count_for_off_and_not_available() {
        assert_eq!(gauge(Some(0.0), Some(&RANGE)).lit_count, 1);
        assert_eq!(gauge(Some(30.0), None).lit_count, 0);
        assert_eq!(gauge(None, Some(&RANGE)).lit_count, 0);
    }

    #[test]
    fn lit_sweep_end_spans_full_circle_at_max_and_clamps() {
        use std::f32::consts::{PI, TAU};
        assert!(lit_sweep_end(0).abs() < 1e-6);
        assert!((lit_sweep_end(TICK_COUNT / 2) - PI).abs() < 1e-6);
        assert!((lit_sweep_end(TICK_COUNT) - TAU).abs() < 1e-6);
        assert!((lit_sweep_end(TICK_COUNT + 5) - TAU).abs() < 1e-6);
    }

    #[test]
    fn lit_prefix_spans_fall_within_their_sweep_end() {
        let spans = TICK_SPANS;
        for lit_count in [1, 7, 14, 28] {
            let end = lit_sweep_end(lit_count);
            let last = spans[lit_count - 1].1;
            assert!(
                last <= end + 1e-6,
                "lit span {lit_count} ends past its sweep"
            );
        }
    }

    #[test]
    fn tick_spans_are_28_ordered_non_overlapping() {
        let spans = TICK_SPANS;
        assert_eq!(spans.len(), TICK_COUNT);
        for window in spans.windows(2) {
            assert!(window[0].0 < window[0].1, "span start precedes its end");
            assert!(window[0].1 <= window[1].0, "spans do not overlap");
        }
    }
}
