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

//! Fixture pool data for the storybook screens, shaped after the design's
//! sample values (349.8 PH/s fleet, ~2400 workers).

use bmc_wasm_sdk::types::Hashrate;
use units::availability::Availability;

use crate::model::{
    NextPayout, PayoutKind, PoolData, Rewards, Sample, Series, SizeBucket, Source, WorkerCounts,
};
use crate::screens::big_chart::BigChartViewData;
use crate::screens::overview::OverviewViewData;

/// A 12-hour window of five-minute slots shaped like real pool telemetry:
/// jittery around a plateau, with slow drift and two curtailment-style dips
/// (as in the design's reference chart). `ramp_decades` exponentially ramps
/// the level across the window (0 = flat) so a single chart can span wildly
/// different magnitudes. A tiny LCG keeps it deterministic — stories and
/// captures must not change between runs.
fn series(base: f64, swing: f64, ramp_decades: f64) -> Series {
    const SLOT_SECS: i64 = 300;
    const SLOTS: i64 = 12 * 12;
    // Curtailment windows in slot indices: a deep evening dip and a short one.
    const DIPS: [(i64, i64); 2] = [(38, 46), (98, 102)];

    let mut rng: u64 = 0x5DEE_CE66;
    let mut noise = move || {
        rng = rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let bits = u16::try_from((rng >> 33) & 0xFFFF).expect("BUG: masked to 16 bits");
        f64::from(bits) / f64::from(u16::MAX) - 0.5
    };

    let samples = (0..SLOTS)
        .map(|i| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "slot indices are tiny; exact in f64"
            )]
            let drift = swing
                * 0.6
                * (i as f64 / f64::from(u32::try_from(SLOTS).expect("BUG: SLOTS fits u32"))).sin();
            let jitter = swing * 0.25 * noise();
            let curtailed = DIPS.iter().any(|&(from, to)| (from..to).contains(&i));
            let level = if curtailed {
                base * 0.55 + jitter
            } else {
                base + drift + jitter
            };
            #[expect(
                clippy::cast_precision_loss,
                reason = "slot indices are tiny; exact in f64"
            )]
            // Climbs from the base up to base·10^decades at the window's end.
            let ramp = 10.0_f64.powf(ramp_decades * (i as f64 / (SLOTS - 1) as f64));
            Sample {
                at: i * SLOT_SECS,
                value: level * ramp,
            }
        })
        .collect();
    Series {
        from: Some(0),
        to: Some(SLOTS * SLOT_SECS),
        samples,
    }
}

/// The sample fleet: ~2400 workers, payout underway. The hashrate history
/// starts at a single-miner 5 TH/s baseline and climbs `spread_decades`
/// decades within the window (0 = flat at the baseline, ~5 lands at the
/// design's few-hundred-PH/s scale, 9 ends in ZH/s territory), stress-testing
/// the axis scaling and SI formatting when one range spans magnitudes.
#[must_use]
pub fn sample_data(spread_decades: f64) -> PoolData {
    const BASE_TH: f64 = 5.0;
    let latest = BASE_TH * 10.0_f64.powf(spread_decades);
    PoolData {
        hashrate_5m: Availability::Available(Hashrate::from_terahashes_per_second(latest)),
        rewards: Availability::Available(Rewards {
            today_btc: 0.170_468,
            today_usd: 10.038,
        }),
        hashrate_history: Availability::Available(series(BASE_TH, BASE_TH * 0.17, spread_decades)),
        workers: Availability::Available(WorkerCounts {
            active: 1_628,
            low: 758,
            offline: 102,
            disabled: 7,
        }),
        workers_history: Availability::Available(series(2_400.0, 350.0, 0.0)),
        next_payout: Availability::Available(NextPayout {
            // Fixture anchor; the reltime node renders against the wall
            // clock, so stories show a long-elapsed estimate.
            estimate_at: Some(42 * 60),
            progress_pct: Some(78.0),
        }),
        // The design's marker scenario: lone payouts, a near-simultaneous
        // lightning pair, and a mixed cluster — the pair and cluster sit
        // minutes apart so their icons overlap and exercise the outline
        // separation.
        payouts: Availability::Available(
            [
                (5_616, PayoutKind::Onchain),
                (15_984, PayoutKind::Onchain),
                (26_784, PayoutKind::Lightning),
                (30_000, PayoutKind::Lightning),
                (30_450, PayoutKind::Lightning),
                (37_584, PayoutKind::Onchain),
                (38_016, PayoutKind::Lightning),
                (38_448, PayoutKind::Lightning),
            ]
            .into_iter()
            .map(|(at, kind)| crate::model::Payout {
                at,
                amount_btc: 0.000_380,
                kind: Some(kind),
            })
            .collect(),
        ),
        access_denied: false,
    }
}

/// The design's Data-variant frame sizes; the Chart frames are wider.
fn overview_size(bucket: SizeBucket) -> (f32, f32) {
    match bucket {
        SizeBucket::Small => (306.0, 220.0),
        SizeBucket::Medium => (620.0, 220.0),
        SizeBucket::Large => (620.0, 448.0),
        SizeBucket::Full => (1_280.0, 480.0),
    }
}

fn chart_size(bucket: SizeBucket) -> (f32, f32) {
    match bucket {
        SizeBucket::Small => (317.0, 238.0),
        SizeBucket::Medium => (638.0, 238.0),
        SizeBucket::Large => (638.0, 480.0),
        SizeBucket::Full => (1_280.0, 480.0),
    }
}

/// What an idle account answers: zeros and empty lists, not absences —
/// every source delivered.
#[must_use]
pub fn empty_data() -> PoolData {
    PoolData {
        hashrate_5m: Availability::Available(Hashrate::from_terahashes_per_second(0.0)),
        rewards: Availability::Available(Rewards {
            today_btc: 0.0,
            today_usd: 0.0,
        }),
        hashrate_history: Availability::Available(series(0.0, 0.0, 0.0)),
        workers: Availability::Available(WorkerCounts::default()),
        workers_history: Availability::Available(series(0.0, 0.0, 0.0)),
        next_payout: Availability::Available(NextPayout::default()),
        payouts: Availability::Available(Vec::new()),
        access_denied: false,
    }
}

/// What every source failing before its first answer leaves behind: no data
/// and nothing more to wait for.
#[must_use]
pub fn failed_data() -> PoolData {
    let mut data = PoolData::default();
    for source in Source::ALL {
        data.mark_failed(source);
    }
    data
}

fn denied_data() -> PoolData {
    PoolData {
        access_denied: true,
        ..PoolData::default()
    }
}

/// Overview screen over the given data, bound to the sample account.
#[must_use]
pub fn overview_with(bucket: SizeBucket, data: PoolData) -> OverviewViewData {
    let (width, height) = overview_size(bucket);
    OverviewViewData {
        bucket,
        width,
        height,
        account: Some("user.braiins".to_owned()),
        bind_hint: crate::screens::parts::BindHint::default(),
        worker_states: true,
        data,
    }
}

/// Overview screen over the sample fleet.
#[must_use]
pub fn sample_overview(
    bucket: SizeBucket,
    worker_states: bool,
    spread_decades: f64,
) -> OverviewViewData {
    let mut view = overview_with(bucket, sample_data(spread_decades));
    view.worker_states = worker_states;
    view
}

/// Overview screen with every source still loading.
#[must_use]
pub fn sample_overview_loading(bucket: SizeBucket) -> OverviewViewData {
    overview_with(bucket, PoolData::default())
}

/// Overview screen over an idle account: everything delivered, nothing there.
#[must_use]
pub fn sample_overview_empty(bucket: SizeBucket) -> OverviewViewData {
    overview_with(bucket, empty_data())
}

/// Overview screen with every source failed before its first answer.
#[must_use]
pub fn sample_overview_failed(bucket: SizeBucket) -> OverviewViewData {
    overview_with(bucket, failed_data())
}

/// The hint the host fills from the deck's own network state.
fn unbound_hint() -> crate::screens::parts::BindHint {
    crate::screens::parts::BindHint {
        ssid: "Braiins-Guest".to_owned(),
        url: "http://192.168.1.42".to_owned(),
    }
}

/// Overview screen with no account bound.
#[must_use]
pub fn sample_overview_unbound(bucket: SizeBucket) -> OverviewViewData {
    OverviewViewData {
        account: None,
        bind_hint: unbound_hint(),
        ..overview_with(bucket, PoolData::default())
    }
}

/// Overview screen with the account's key refused by the API.
#[must_use]
pub fn sample_overview_denied(bucket: SizeBucket) -> OverviewViewData {
    overview_with(bucket, denied_data())
}

/// Big Chart screen with no account bound.
#[must_use]
pub fn sample_big_chart_unbound(bucket: SizeBucket) -> BigChartViewData {
    BigChartViewData {
        account: None,
        bind_hint: unbound_hint(),
        ..big_chart_with(bucket, PoolData::default())
    }
}

/// Big Chart screen over the given data, bound to the sample account.
#[must_use]
pub fn big_chart_with(bucket: SizeBucket, data: PoolData) -> BigChartViewData {
    let x_labels = data
        .hashrate_history
        .as_option()
        .and_then(|series| Some((series.from?, series.to?)))
        .map(|(from, to)| {
            crate::chart::x_axis_marks(from, to, 5)
                .into_iter()
                .map(|(at, fraction)| (fraction, hour_label(at)))
                .collect()
        })
        .unwrap_or_default();
    let (width, height) = chart_size(bucket);
    BigChartViewData {
        bucket,
        width,
        height,
        account: Some("user.braiins".to_owned()),
        bind_hint: crate::screens::parts::BindHint::default(),
        worker_states: true,
        data,
        x_labels,
    }
}

/// Big Chart screen over the sample fleet.
#[must_use]
pub fn sample_big_chart(
    bucket: SizeBucket,
    worker_states: bool,
    spread_decades: f64,
) -> BigChartViewData {
    let mut view = big_chart_with(bucket, sample_data(spread_decades));
    view.worker_states = worker_states;
    view
}

/// Big Chart screen with every source still loading.
#[must_use]
pub fn sample_big_chart_loading(bucket: SizeBucket) -> BigChartViewData {
    big_chart_with(bucket, PoolData::default())
}

/// Big Chart screen over an idle account: everything delivered, nothing there.
#[must_use]
pub fn sample_big_chart_empty(bucket: SizeBucket) -> BigChartViewData {
    big_chart_with(bucket, empty_data())
}

/// Big Chart screen with every source failed before its first answer.
#[must_use]
pub fn sample_big_chart_failed(bucket: SizeBucket) -> BigChartViewData {
    big_chart_with(bucket, failed_data())
}

/// Big Chart screen with the account's key refused by the API.
#[must_use]
pub fn sample_big_chart_denied(bucket: SizeBucket) -> BigChartViewData {
    big_chart_with(bucket, denied_data())
}

/// Pure "HH:MM" of a fixture timestamp's time of day.
/// Hand-padded: ufmt zero-pads only hex,
/// and the host time formatters are unavailable natively.
#[expect(
    clippy::integer_division,
    reason = "clock arithmetic: whole hours/minutes and their decimal digits"
)]
fn hour_label(at: i64) -> String {
    let secs_of_day = at.rem_euclid(24 * 3_600);
    let push_two_digits = |label: &mut String, value: i64| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a decimal digit fits u8"
        )]
        let digit = |d: i64| char::from(b'0' + d as u8);
        label.push(digit(value / 10));
        label.push(digit(value.rem_euclid(10)));
    };
    let mut label = String::with_capacity(5);
    push_two_digits(&mut label, secs_of_day / 3_600);
    label.push(':');
    push_two_digits(&mut label, secs_of_day.rem_euclid(3_600) / 60);
    label
}
