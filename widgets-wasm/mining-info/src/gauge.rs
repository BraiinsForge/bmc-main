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

// Portion of each tick slot left as the inter-tick gap; the rest is the lit
// segment. ~0.04 yields a ~2px gap at the ring radius, matching the design.
const TICK_GAP_FRACTION: f32 = 0.04;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GaugeState {
    Off,
    UnknownScale,
    Underclocked,
    Good,
    Overclocked,
}

pub(crate) struct Gauge {
    pub(crate) state: GaugeState,
    pub(crate) lit_count: usize,
}

// OFF is decided solely by actual hashing so missing scale data never reads as
// OFF: a stopped miner reports exactly 0.0, so no epsilon is used. When hashing
// but `mcr_percent` is unavailable the scale is unknown — no lit ticks, neutral
// label — distinct from OFF.
pub(crate) fn gauge(hashrate_ths: Availability<f64>, mcr_percent: Availability<f64>) -> Gauge {
    let hashing = matches!(hashrate_ths, Availability::Available(hr) if hr > 0.0);
    if !hashing {
        return Gauge {
            state: GaugeState::Off,
            lit_count: 1,
        };
    }
    let Availability::Available(mcr) = mcr_percent else {
        return Gauge {
            state: GaugeState::UnknownScale,
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

// The 28 absolute tick spans, an even partition of 0..TAU clockwise from 12
// o'clock with a per-slot gap. Both gauge sweeps draw from this one list (base
// = all, lit = a prefix slice) so lit ticks register exactly over base ticks.
#[expect(
    clippy::cast_precision_loss,
    reason = "tick index/count are small UI geometry counts"
)]
pub(crate) fn tick_spans() -> Vec<(f32, f32)> {
    let slot = std::f32::consts::TAU / TICK_COUNT as f32;
    let seg = slot * (1.0 - TICK_GAP_FRACTION);
    (0..TICK_COUNT)
        .map(|i| {
            let start = i as f32 * slot;
            (start, start + seg)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_when_hashrate_unavailable_regardless_of_mcr() {
        let g = gauge(Availability::Unavailable, Availability::Available(100.0));
        assert_eq!(g.state, GaugeState::Off);
        assert_eq!(g.lit_count, 1);
    }

    #[test]
    fn off_when_hashrate_is_zero() {
        let g = gauge(Availability::Available(0.0), Availability::Available(100.0));
        assert_eq!(g.state, GaugeState::Off);
    }

    #[test]
    fn unknown_scale_when_hashing_but_mcr_unavailable() {
        let g = gauge(Availability::Available(4.0), Availability::Unavailable);
        assert_eq!(g.state, GaugeState::UnknownScale);
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
    fn tick_spans_are_28_ordered_non_overlapping() {
        let spans = tick_spans();
        assert_eq!(spans.len(), TICK_COUNT);
        for window in spans.windows(2) {
            assert!(window[0].0 < window[0].1, "span start precedes its end");
            assert!(window[0].1 <= window[1].0, "spans do not overlap");
        }
    }

    #[test]
    fn lit_spans_are_a_prefix_of_the_shared_list() {
        let spans = tick_spans();
        let lit_count = 14;
        let lit = &spans[..lit_count];
        for (i, span) in lit.iter().enumerate() {
            assert_eq!(*span, spans[i], "lit sweep reuses the absolute spans");
        }
    }
}
