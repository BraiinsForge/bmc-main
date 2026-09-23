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

//! Which layout a viewport gets, the prediction and what is left until it,
//! and how fresh the prediction is.

use std::time::Duration;

use bmc_wasm_sdk::{SizeVariant, WidgetSize, WidgetViewport};

/// How long a 429 holds the next attempt back.
pub const RATE_LIMIT_RETRY: Duration = Duration::from_mins(10);

const SECS_PER_MINUTE: i64 = 60;
const SECS_PER_HOUR: i64 = 3_600;
const SECS_PER_DAY: i64 = 86_400;
/// A healthy payload can sit a full TTL in Nexus's cache and another between our polls.
const STALE_TTL_MULTIPLIER: u64 = 2;

/// The frames a layout is picked for: the four BMC100 slots and BMM101's 480×320.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeBucket {
    Full,
    Large,
    Medium,
    Small,
    Bmm101,
}

impl SizeBucket {
    #[must_use]
    pub const fn design_size(self) -> (u32, u32) {
        match self {
            Self::Full => (1_280, 480),
            Self::Large => (638, 480),
            Self::Medium => (638, 238),
            Self::Small => (317, 238),
            Self::Bmm101 => (480, 320),
        }
    }
}

/// A rectangular viewport classified once for every layout:
/// its pixels with their closest BMC100 variant, and the bucket that picks the layout.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub size: WidgetSize,
    pub bucket: SizeBucket,
}

impl Frame {
    #[must_use]
    pub fn of(viewport: WidgetViewport) -> Self {
        Self {
            size: WidgetSize::from_dimensions(viewport.width, viewport.height),
            bucket: size_bucket(viewport.width, viewport.height),
        }
    }
}

/// The two BMM frames the narrow bucket has to tell apart.
const BMM100_HEIGHT: u32 = 240;
const BMM101_HEIGHT: u32 = SizeBucket::Bmm101.design_size().1;
/// Split at their midpoint, so either frame keeps its bucket a few pixels either way.
const BMM101_MIN_HEIGHT: u32 = u32::midpoint(BMM100_HEIGHT, BMM101_HEIGHT);
const BMM101_MAX_WIDTH: u32 = SizeBucket::Bmm101.design_size().0;

/// A landscape frame no wider than BMM101 and at least as tall as the BMM split
/// is BMM101; everything else takes the closest BMC100 variant, as the SDK does.
#[must_use]
pub fn size_bucket(width: u32, height: u32) -> SizeBucket {
    if width <= BMM101_MAX_WIDTH && height < width && height >= BMM101_MIN_HEIGHT {
        return SizeBucket::Bmm101;
    }
    match SizeVariant::closest(width, height) {
        SizeVariant::Full => SizeBucket::Full,
        SizeVariant::Large => SizeBucket::Large,
        SizeVariant::Medium => SizeBucket::Medium,
        SizeVariant::Small => SizeBucket::Small,
    }
}

/// Days / hours / minutes remaining, already floored into place-values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Countdown {
    pub days: i64,
    pub hours: i64,
    pub minutes: i64,
}

/// Split a second count into whole days, hours and minutes, clamped at zero.
/// Seconds are dropped — the widget only shows minute resolution.
fn decompose(total_seconds: i64) -> Countdown {
    let total = total_seconds.max(0);
    Countdown {
        days: total / SECS_PER_DAY,
        hours: (total % SECS_PER_DAY) / SECS_PER_HOUR,
        minutes: (total % SECS_PER_HOUR) / SECS_PER_MINUTE,
    }
}

/// Nexus's server-computed halving prediction: the tip, the halving block and when it lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prediction {
    pub current_height: u32,
    pub target_block: u32,
    pub predicted_unix: i64,
}

impl Prediction {
    /// Counted against the local clock, so it keeps ticking between polls.
    #[must_use]
    pub fn countdown(self, now_secs: i64) -> Countdown {
        decompose(self.predicted_unix.saturating_sub(now_secs))
    }

    #[must_use]
    pub fn blocks_remaining(self) -> u32 {
        self.target_block.saturating_sub(self.current_height)
    }
}

/// When the served payload was computed, and how long Nexus says it holds.
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

    /// Stale once older than a healthy cycle allows, the slowest reply included.
    #[must_use]
    pub fn stale_anchor(self, now_secs: i64, reply_timeout_secs: u64) -> Option<i64> {
        let payload_unix_secs = self.payload_unix_secs?;
        let age_secs = u64::try_from(now_secs.saturating_sub(payload_unix_secs)).unwrap_or(0);
        let healthy_max_age_secs = self
            .ttl_secs
            .saturating_mul(STALE_TTL_MULTIPLIER)
            .saturating_add(reply_timeout_secs);
        if age_secs <= healthy_max_age_secs {
            return None;
        }
        Some(payload_unix_secs)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    fn decompose_splits_place_values() {
        // 2 days, 3 hours, 4 minutes, 30 seconds (seconds dropped).
        let total = 2 * 86_400 + 3 * 3_600 + 4 * 60 + 30;
        assert_eq!(
            decompose(total),
            Countdown {
                days: 2,
                hours: 3,
                minutes: 4
            }
        );
    }

    #[test]
    fn decompose_clamps_negative_to_zero() {
        let cd = decompose(-500);
        assert_eq!((cd.days, cd.hours, cd.minutes), (0, 0, 0));
    }

    #[test]
    fn decompose_large_span() {
        // ~631 days out (a fresh halving era), like the live nexus payload.
        let cd = decompose(54_533_098);
        assert_eq!((cd.days, cd.hours, cd.minutes), (631, 4, 4));
    }

    fn prediction(current_height: u32, target_block: u32) -> Prediction {
        Prediction {
            current_height,
            target_block,
            predicted_unix: 0,
        }
    }

    #[test]
    fn blocks_remaining_is_target_minus_tip() {
        assert_eq!(prediction(959_111, 1_050_000).blocks_remaining(), 90_889);
    }

    #[test]
    fn blocks_remaining_saturates_past_target() {
        assert_eq!(prediction(1_050_001, 1_050_000).blocks_remaining(), 0);
    }

    #[test]
    fn every_design_size_lands_in_its_own_bucket() {
        for bucket in [
            SizeBucket::Full,
            SizeBucket::Large,
            SizeBucket::Medium,
            SizeBucket::Small,
            SizeBucket::Bmm101,
        ] {
            let (width, height) = bucket.design_size();
            assert_eq!(size_bucket(width, height), bucket, "{width}x{height}");
        }
    }

    /// A frame that misses the BMM101 rule by a pixel falls to the SDK's closest variant.
    #[test]
    fn the_bmm101_bucket_ends_at_the_midpoint_of_the_bmm_heights_and_at_its_width() {
        assert_eq!(size_bucket(320, 240), SizeBucket::Small, "BMM100");
        assert_eq!(size_bucket(480, 280), SizeBucket::Bmm101);
        assert_eq!(size_bucket(480, 279), SizeBucket::Medium);
        assert_eq!(size_bucket(481, 320), SizeBucket::Large);
    }

    #[test]
    fn the_square_bfm100_stays_large() {
        assert_eq!(size_bucket(480, 480), SizeBucket::Large);
    }

    #[test]
    fn nexus_payload_becomes_stale_after_two_refresh_intervals_and_a_reply_timeout() {
        let freshness = Freshness {
            payload_unix_secs: Some(870),
            ttl_secs: 60,
        };
        assert_eq!(freshness.stale_anchor(1_000, 10), None);
        assert_eq!(freshness.stale_anchor(1_001, 10), Some(870));
    }

    /// Received 57 s old at 1 000; the next reply is due a TTL later and runs 9 s slow.
    #[test]
    fn a_slow_reply_to_a_cache_aged_payload_is_not_stale_before_it_lands() {
        let freshness = Freshness {
            payload_unix_secs: Some(1_000 - 57),
            ttl_secs: 60,
        };
        assert_eq!(freshness.stale_anchor(1_000 + 60 + 9, 10), None);
    }
}
