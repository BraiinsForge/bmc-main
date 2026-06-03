// Copyright (C) 2026  Braiins Systems s.r.o.

//! Gauge math for the round Mining/Geek screens: state classification, the
//! lit-tick count, and the 28 absolute tick spans. Holds no renderer/SDK type
//! so it unit-tests on the host; the round renderer maps `GaugeState` to colors
//! and wraps the spans in `ArcSegments` at the draw site.

use crate::model::Availability;

pub(crate) const TICK_COUNT: usize = 28;

// MCR band edges in percent, inclusive at the lower edge. 130 is the
// product-chosen overclock edge; 85 matches bos-main's underperforming_mcr.
pub(crate) const OVERCLOCK_MCR: f64 = 130.0;
pub(crate) const GOOD_MCR: f64 = 85.0;

// Hashrate (TH/s) at or below this reads as OFF rather than a live miner: a
// stopped miner reports ~0, and tiny residual values are not meaningful hashing.
const OFF_MAX_THS: f64 = 0.01;

// Portion of each tick slot left as the inter-tick gap; the rest is the lit
// segment. ~0.04 yields a ~2px gap at the ring radius, matching the design.
const TICK_GAP_FRACTION: f32 = 0.04;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GaugeState {
    NotAvailable,
    Off,
    Underclocked,
    Good,
    Overclocked,
}

pub(crate) struct Gauge {
    pub(crate) state: GaugeState,
    pub(crate) lit_count: usize,
}

// `Off` is reserved for a miner that is effectively not hashing: a reported
// hashrate at or below `OFF_MAX_THS`. Any input with no usable telemetry —
// hashrate unavailable, or hashing while `mcr_percent` is unavailable — is
// `NotAvailable`, which renders neutral with no lit ticks.
pub(crate) fn gauge(hashrate_ths: Availability<f64>, mcr_percent: Availability<f64>) -> Gauge {
    let Availability::Available(hashrate) = hashrate_ths else {
        return Gauge {
            state: GaugeState::NotAvailable,
            lit_count: 0,
        };
    };
    if hashrate <= OFF_MAX_THS {
        return Gauge {
            state: GaugeState::Off,
            lit_count: 1,
        };
    }
    let Availability::Available(mcr) = mcr_percent else {
        return Gauge {
            state: GaugeState::NotAvailable,
            lit_count: 0,
        };
    };
    let state = if mcr >= OVERCLOCK_MCR {
        GaugeState::Overclocked
    } else if mcr >= GOOD_MCR {
        GaugeState::Good
    } else {
        GaugeState::Underclocked
    };
    Gauge {
        state,
        lit_count: lit_count_from_mcr(mcr),
    }
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "lit-tick count is a small clamped non-negative value"
)]
fn lit_count_from_mcr(mcr: f64) -> usize {
    let fill = (mcr / OVERCLOCK_MCR).clamp(0.0, 1.0);
    (fill * TICK_COUNT as f64).round() as usize
}

// Sweep end angle (radians, clockwise from 12 o'clock) of the lit overlay for
// `lit_count` ticks: the slot boundary just past the last lit tick. Encoding
// the fill as the arc sweep lets a host transition interpolate it on change;
// the lit prefix spans all fall within `[0, this]`.
#[expect(
    clippy::cast_precision_loss,
    reason = "tick index/count are small UI geometry counts"
)]
pub(crate) fn lit_sweep_end(lit_count: usize) -> f32 {
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
pub(crate) const TICK_SPANS: [(f32, f32); TICK_COUNT] = {
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

    #[test]
    fn not_available_when_hashrate_unavailable_regardless_of_mcr() {
        let g = gauge(Availability::Unavailable, Availability::Available(100.0));
        assert_eq!(g.state, GaugeState::NotAvailable);
        assert_eq!(g.lit_count, 0);
    }

    #[test]
    fn off_when_hashrate_at_or_below_off_threshold() {
        for hr in [0.0, 0.005, 0.01] {
            let g = gauge(Availability::Available(hr), Availability::Available(100.0));
            assert_eq!(
                g.state,
                GaugeState::Off,
                "hashrate {hr} TH/s should read Off"
            );
            assert_eq!(g.lit_count, 1);
        }
    }

    #[test]
    fn hashing_just_above_off_threshold_classifies_by_mcr() {
        let g = gauge(Availability::Available(0.02), Availability::Available(66.0));
        assert_eq!(g.state, GaugeState::Underclocked);
    }

    #[test]
    fn not_available_when_hashing_but_mcr_unavailable() {
        let g = gauge(Availability::Available(4.0), Availability::Unavailable);
        assert_eq!(g.state, GaugeState::NotAvailable);
        assert_eq!(g.lit_count, 0);
    }

    #[test]
    fn underclocked_below_good_edge() {
        let g = gauge(Availability::Available(3.0), Availability::Available(66.0));
        assert_eq!(g.state, GaugeState::Underclocked);
    }

    #[test]
    fn good_is_inclusive_at_85_and_excludes_130() {
        assert_eq!(
            gauge(Availability::Available(4.0), Availability::Available(85.0)).state,
            GaugeState::Good
        );
        assert_eq!(
            gauge(Availability::Available(4.0), Availability::Available(129.9)).state,
            GaugeState::Good
        );
    }

    #[test]
    fn overclocked_at_and_above_130_fills_all_ticks() {
        let edge = gauge(Availability::Available(6.0), Availability::Available(130.0));
        assert_eq!(edge.state, GaugeState::Overclocked);
        assert_eq!(edge.lit_count, TICK_COUNT);
        let above = gauge(Availability::Available(6.0), Availability::Available(200.0));
        assert_eq!(above.lit_count, TICK_COUNT);
    }

    #[test]
    fn lit_count_tracks_mcr_fraction() {
        // 65/130 = 0.5 → 14 of 28.
        let g = gauge(Availability::Available(3.0), Availability::Available(65.0));
        assert_eq!(g.lit_count, 14);
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

    #[test]
    fn lit_spans_are_a_prefix_of_the_shared_list() {
        let spans = TICK_SPANS;
        let lit_count = 14;
        let lit = &spans[..lit_count];
        for (i, span) in lit.iter().enumerate() {
            assert_eq!(*span, spans[i], "lit sweep reuses the absolute spans");
        }
    }
}
