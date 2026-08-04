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

//! Widget state: what the API replies fill in and what the render path reads.

use bmc_wasm_sdk::Hashrate;
use units::availability::Availability;

use crate::manifest_params::{ChartFrame, Style};

/// Layout band picked from the actual viewport, mirroring the design's
/// Small / Medium / Large / Fullscreen widget variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeBucket {
    Small,
    Medium,
    Large,
    Full,
}

/// Design reference sizes: S 306×220, M 620×220, L 620×448, Fullscreen 1280×480.
/// Thresholds sit between neighbouring variants so each snaps to its band.
#[must_use]
pub fn size_bucket(width: u32, height: u32) -> SizeBucket {
    if width >= 900 {
        SizeBucket::Full
    } else if width <= 450 {
        SizeBucket::Small
    } else if height <= 330 {
        SizeBucket::Medium
    } else {
        SizeBucket::Large
    }
}

/// The API sources the widget polls. Each maps to one endpoint;
/// [`PayoutsRecent`](Source::PayoutsRecent) changes its query shape by style
/// (Overview reads one recent page, Big Chart pages across the chart frame).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    HashrateCurrent,
    RewardsLatest,
    HashrateHistory,
    WorkersCurrent,
    WorkersHistory,
    Financials,
    PayoutsRecent,
}

impl Source {
    pub const ALL: [Self; 7] = [
        Self::HashrateCurrent,
        Self::RewardsLatest,
        Self::HashrateHistory,
        Self::WorkersCurrent,
        Self::WorkersHistory,
        Self::Financials,
        Self::PayoutsRecent,
    ];

    /// Name for log lines; guest logging formats with ufmt,
    /// which has no `Debug`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::HashrateCurrent => "hashrate/current",
            Self::RewardsLatest => "rewards/latest",
            Self::HashrateHistory => "hashrate/history",
            Self::WorkersCurrent => "workers/current",
            Self::WorkersHistory => "workers/history",
            Self::Financials => "financials",
            Self::PayoutsRecent => "payouts/recent",
        }
    }
}

/// Whether a source feeds the (style × size) variant currently on screen.
/// `worker_states` additionally gates the worker sources: with the toggle
/// off nothing worker-related renders — no counts tile, no chart line
/// — so the data is not fetched.
#[must_use]
pub fn source_needed(
    source: Source,
    style: Style,
    bucket: SizeBucket,
    worker_states: bool,
) -> bool {
    use SizeBucket::{Full, Large, Medium};
    let by_variant = match source {
        Source::HashrateCurrent => true,
        Source::RewardsLatest => {
            style == Style::Overview && matches!(bucket, Full | Large | Medium)
        }
        Source::HashrateHistory => style == Style::BigChart || matches!(bucket, Full | Large),
        Source::WorkersCurrent => match style {
            Style::BigChart => matches!(bucket, Full | Large | Medium),
            Style::Overview => matches!(bucket, Full | Medium),
        },
        Source::WorkersHistory => match style {
            Style::BigChart => matches!(bucket, Full | Large | Medium),
            Style::Overview => bucket == Full,
        },
        Source::Financials => style == Style::Overview && matches!(bucket, Full | Large),
        Source::PayoutsRecent => match style {
            Style::Overview => matches!(bucket, Full | Large),
            Style::BigChart => bucket == Full,
        },
    };
    let worker_source = matches!(source, Source::WorkersCurrent | Source::WorkersHistory);
    by_variant && (worker_states || !worker_source)
}

/// Chart frame span in seconds.
#[must_use]
pub fn chart_frame_secs(frame: ChartFrame) -> i64 {
    match frame {
        ChartFrame::Hours4 => 4 * 3_600,
        ChartFrame::Hours12 => 12 * 3_600,
        ChartFrame::Hours24 => 24 * 3_600,
        ChartFrame::Days7 => 7 * 24 * 3_600,
    }
}

/// One history sample: slot start (unix seconds) and the sampled value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub at: i64,
    pub value: f64,
}

/// Time-ordered history series over a known window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub samples: Vec<Sample>,
}

impl Series {
    /// Fold another page into this series, keeping samples time-ordered and
    /// the window spanning both parts.
    pub fn merge(&mut self, mut page: Self) {
        self.from = merge_bound(self.from, page.from, i64::min);
        self.to = merge_bound(self.to, page.to, i64::max);
        self.samples.append(&mut page.samples);
        self.samples.sort_by_key(|sample| sample.at);
    }
}

fn merge_bound(a: Option<i64>, b: Option<i64>, pick: fn(i64, i64) -> i64) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(pick(a, b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// Worker counts by state, straight from `/user/workers/current`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerCounts {
    pub active: u64,
    pub low: u64,
    pub offline: u64,
    pub disabled: u64,
}

/// Today's reward estimate from `/user/rewards/latest`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rewards {
    pub today_btc: f64,
    pub today_usd: f64,
}

/// Next-payout estimate from `/user/financials`, folded across the user's
/// financial accounts (soonest estimate, furthest progress).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NextPayout {
    pub estimate_at: Option<i64>,
    pub progress_pct: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayoutKind {
    Onchain,
    Lightning,
}

/// A completed payout: when it happened and how much.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Payout {
    pub at: i64,
    pub amount_btc: f64,
    /// `None` for a rail the reply did not name in terms this widget knows.
    /// A payout is its amount and its time; the rail only picks a chart
    /// marker, so an unrecognised one costs the marker, not the payout.
    pub kind: Option<PayoutKind>,
}

/// Everything the render path reads. Each field mirrors one source and stays
/// `Unavailable` until that source delivers.
#[derive(Clone, Debug, Default)]
pub struct PoolData {
    pub hashrate_5m: Availability<Hashrate>,
    pub rewards: Availability<Rewards>,
    pub hashrate_history: Availability<Series>,
    pub workers: Availability<WorkerCounts>,
    pub workers_history: Availability<Series>,
    pub next_payout: Availability<NextPayout>,
    pub payouts: Availability<Vec<Payout>>,
    /// The API refused the key (HTTP 401/403).
    /// Without this, a bad key would render as loading forever
    /// — skeletons are only for sources that have not answered.
    pub access_denied: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_snap_to_design_sizes() {
        assert_eq!(size_bucket(306, 220), SizeBucket::Small);
        assert_eq!(size_bucket(620, 220), SizeBucket::Medium);
        assert_eq!(size_bucket(620, 448), SizeBucket::Large);
        assert_eq!(size_bucket(1280, 480), SizeBucket::Full);
    }

    #[test]
    fn hashrate_is_always_needed() {
        for style in [Style::Overview, Style::BigChart] {
            for bucket in [
                SizeBucket::Small,
                SizeBucket::Medium,
                SizeBucket::Large,
                SizeBucket::Full,
            ] {
                assert!(source_needed(Source::HashrateCurrent, style, bucket, false));
            }
        }
    }

    #[test]
    fn big_chart_fetches_history_at_every_size() {
        for bucket in [
            SizeBucket::Small,
            SizeBucket::Medium,
            SizeBucket::Large,
            SizeBucket::Full,
        ] {
            assert!(source_needed(
                Source::HashrateHistory,
                Style::BigChart,
                bucket,
                true,
            ));
        }
    }

    #[test]
    fn worker_states_toggle_gates_worker_sources_only() {
        assert!(!source_needed(
            Source::WorkersCurrent,
            Style::Overview,
            SizeBucket::Full,
            false,
        ));
        assert!(!source_needed(
            Source::WorkersHistory,
            Style::BigChart,
            SizeBucket::Full,
            false,
        ));
        assert!(source_needed(
            Source::RewardsLatest,
            Style::Overview,
            SizeBucket::Full,
            false,
        ));
    }

    #[test]
    fn overview_small_reads_hashrate_only() {
        let needed: Vec<Source> = Source::ALL
            .into_iter()
            .filter(|s| source_needed(*s, Style::Overview, SizeBucket::Small, true))
            .collect();
        assert_eq!(needed, [Source::HashrateCurrent]);
    }

    #[test]
    fn series_merge_orders_samples_and_spans_windows() {
        let mut series = Series {
            from: Some(200),
            to: Some(300),
            samples: vec![Sample {
                at: 250,
                value: 2.0,
            }],
        };
        series.merge(Series {
            from: Some(100),
            to: Some(400),
            samples: vec![
                Sample {
                    at: 150,
                    value: 1.0,
                },
                Sample {
                    at: 350,
                    value: 3.0,
                },
            ],
        });
        assert_eq!(series.from, Some(100));
        assert_eq!(series.to, Some(400));
        let order: Vec<i64> = series.samples.iter().map(|s| s.at).collect();
        assert_eq!(order, [150, 250, 350]);
    }
}
