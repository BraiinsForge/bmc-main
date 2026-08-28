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

use units::availability::Availability;

const STALE_TTL_MULTIPLIER: u64 = 2;
pub(crate) const HASHES_PER_TERAHASH: f64 = 1e12;
pub(crate) const TERAHASHES_PER_EXAHASH: f64 = 1_000_000.0;
pub(crate) const TERAHASHES_PER_PETAHASH: f64 = 1_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeBucket {
    Small,
    Medium,
    Large,
    Full,
}

impl SizeBucket {
    #[must_use]
    pub const fn design_size(self) -> (f32, f32) {
        match self {
            Self::Small => (317.0, 238.0),
            Self::Medium => (638.0, 238.0),
            Self::Large => (638.0, 480.0),
            Self::Full => (1_280.0, 480.0),
        }
    }
}

#[must_use]
pub const fn size_bucket(width: u32, height: u32) -> SizeBucket {
    if width >= 900 && height > 330 {
        SizeBucket::Full
    } else if width <= 480 {
        SizeBucket::Small
    } else if height <= 330 {
        SizeBucket::Medium
    } else {
        SizeBucket::Large
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    Info,
    History,
}

impl Resource {
    pub const ALL: [Self; 2] = [Self::Info, Self::History];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "mining-info",
            Self::History => "mining-history",
        }
    }

    #[must_use]
    pub const fn initial_interval_ms(self) -> u32 {
        match self {
            Self::Info => 60_000,
            Self::History => 10 * 60_000,
        }
    }
}

#[must_use]
pub const fn resource_needed(resource: Resource, bucket: SizeBucket) -> bool {
    match resource {
        Resource::Info => true,
        Resource::History => !matches!(bucket, SizeBucket::Small),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Freshness {
    pub payload_unix_secs: Option<i64>,
    pub ttl_secs: u64,
}

impl Freshness {
    #[must_use]
    pub fn interval_ms(self) -> u32 {
        u32::try_from(self.ttl_secs.saturating_mul(1_000)).unwrap_or(u32::MAX)
    }

    #[must_use]
    pub fn stale_anchor(self, now_secs: i64) -> Option<i64> {
        let payload_unix_secs = self.payload_unix_secs?;
        let age_secs = u64::try_from(now_secs.saturating_sub(payload_unix_secs)).unwrap_or(0);
        if age_secs <= self.ttl_secs.saturating_mul(STALE_TTL_MULTIPLIER) {
            return None;
        }
        Some(payload_unix_secs)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    pub values: Vec<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PriceStats {
    pub price: Option<f64>,
    pub change_24h_percent: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DifficultyStats {
    pub difficulty: Option<f64>,
    pub previous_adjustment_percent: Option<f64>,
    pub estimated_adjustment_percent: Option<f64>,
    pub estimated_adjustment_at: Option<i64>,
    pub epoch_block: Option<u64>,
    pub epoch_block_time_secs: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HashrateStats {
    pub current_ehs: Option<f64>,
    pub avg_fees_btc: Option<f64>,
    pub fees_percent: Option<f64>,
    pub hashprice_per_th_day: Option<f64>,
    pub revenue: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DayHistory {
    pub price: Series,
    pub hashrate: Series,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BitcoinData {
    pub price_stats: Availability<PriceStats>,
    pub difficulty_stats: Availability<DifficultyStats>,
    pub hashrate_stats: Availability<HashrateStats>,
    pub latest_block: Availability<u64>,
    pub day_history: Availability<DayHistory>,
    pub year_history: Availability<Series>,
    pub blocks_24h: Availability<u64>,
}

impl BitcoinData {
    pub fn mark_failed(&mut self, resource: Resource) {
        match resource {
            Resource::Info => {
                self.price_stats.mark_failed();
                self.difficulty_stats.mark_failed();
                self.hashrate_stats.mark_failed();
                self.latest_block.mark_failed();
                self.blocks_24h.mark_failed();
            }
            Resource::History => {
                self.day_history.mark_failed();
                self.year_history.mark_failed();
            }
        }
    }

    #[must_use]
    pub fn has_initial_failure(&self, bucket: SizeBucket) -> bool {
        self.price_stats.failed()
            || self.difficulty_stats.failed()
            || self.hashrate_stats.failed()
            || self.latest_block.failed()
            || self.blocks_24h.failed()
            || (resource_needed(Resource::History, bucket)
                && (self.day_history.failed() || self.year_history.failed()))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Status {
    Ready,
    Stale(i64),
    Failed,
    RateLimited,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_design_and_bmm_sizes() {
        assert_eq!(size_bucket(317, 238), SizeBucket::Small);
        assert_eq!(size_bucket(638, 238), SizeBucket::Medium);
        assert_eq!(size_bucket(638, 480), SizeBucket::Large);
        assert_eq!(size_bucket(1_280, 480), SizeBucket::Full);
        assert_eq!(size_bucket(320, 240), SizeBucket::Small);
        assert_eq!(size_bucket(480, 320), SizeBucket::Small);
        assert_eq!(size_bucket(1_280, 238), SizeBucket::Medium);
    }

    #[test]
    fn history_is_skipped_only_for_small() {
        assert!(!resource_needed(Resource::History, SizeBucket::Small));
        for bucket in [SizeBucket::Medium, SizeBucket::Large, SizeBucket::Full] {
            assert!(resource_needed(Resource::History, bucket));
        }
        for bucket in [
            SizeBucket::Small,
            SizeBucket::Medium,
            SizeBucket::Large,
            SizeBucket::Full,
        ] {
            assert!(resource_needed(Resource::Info, bucket));
        }
    }

    #[test]
    fn nexus_payload_becomes_stale_after_two_refresh_intervals() {
        let freshness = Freshness {
            payload_unix_secs: Some(880),
            ttl_secs: 60,
        };
        assert_eq!(freshness.stale_anchor(1_000), None);
        assert_eq!(freshness.stale_anchor(1_001), Some(880));
        assert_eq!(freshness.stale_anchor(1_500), Some(880));
    }
}
