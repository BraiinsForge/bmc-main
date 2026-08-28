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

use crate::model::{
    BitcoinData, DayHistory, DifficultyStats, HashrateStats, PriceStats, Resource, Series,
    SizeBucket, Status,
};
use crate::screens::ViewData;

const NOW: i64 = 1_775_000_000;

fn wave(base: f64, swing: f64, points: usize, rising: f64) -> Series {
    let values = (0..points)
        .map(|index| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "fixture sample counts are tiny and exact in f64"
            )]
            let x = index as f64;
            let noise_index =
                u8::try_from((index * 37) % 17).expect("BUG: modulo 17 always fits in u8");
            let noise = f64::from(noise_index) / 17.0 - 0.5;
            base + (x * 0.73).sin() * swing * 0.55
                + (x * 0.19).cos() * swing * 0.35
                + noise * swing * 0.5
                + rising * x
        })
        .collect();
    Series { values }
}

fn wave_with_change(base: f64, swing: f64, points: usize, change_percent: f64) -> Series {
    let mut series = wave(base, swing, points, 0.2);
    if let Some(first) = series.values.first().copied()
        && let Some(last) = series.values.last_mut()
    {
        *last = first * (1.0 + change_percent / 100.0);
    }
    series
}

#[must_use]
pub fn healthy_data() -> BitcoinData {
    BitcoinData {
        price_stats: Availability::Available(PriceStats {
            price: Some(169_420.0),
            change_24h_percent: Some(5.31),
        }),
        difficulty_stats: Availability::Available(DifficultyStats {
            difficulty: Some(129.7e12),
            previous_adjustment_percent: Some(-2.81),
            estimated_adjustment_percent: Some(10.4),
            estimated_adjustment_at: Some(NOW + 9 * 86_400),
            epoch_block: Some(1_293),
            epoch_block_time_secs: Some(9 * 60 + 19),
        }),
        hashrate_stats: Availability::Available(HashrateStats {
            current_ehs: Some(877.8),
            avg_fees_btc: Some(0.021),
            fees_percent: Some(0.66),
            hashprice_per_th_day: Some(0.0562),
            revenue: Some(49.35e6),
        }),
        latest_block: Availability::Available(914_038),
        day_history: Availability::Available(DayHistory {
            price: wave(160_000.0, 3_800.0, 38, 280.0),
            hashrate: wave_with_change(835.0, 31.0, 38, 2.1),
        }),
        year_history: Availability::Available(wave(122.0, 5.0, 48, 0.12)),
        blocks_24h: Availability::Available(151),
    }
}

#[must_use]
pub fn view(bucket: SizeBucket, data: BitcoinData, status: Status) -> ViewData {
    ViewData {
        bucket,
        data,
        status,
        now_secs: NOW,
    }
}

#[must_use]
pub fn healthy(bucket: SizeBucket) -> ViewData {
    view(bucket, healthy_data(), Status::Ready)
}

#[must_use]
pub fn loading(bucket: SizeBucket) -> ViewData {
    view(bucket, BitcoinData::default(), Status::Ready)
}

#[must_use]
pub fn failed(bucket: SizeBucket) -> ViewData {
    let mut data = BitcoinData::default();
    for resource in Resource::ALL {
        data.mark_failed(resource);
    }
    view(bucket, data, Status::Failed)
}

#[must_use]
pub fn stale(bucket: SizeBucket) -> ViewData {
    // The gallery clock starts at zero, so this reads as 18 minutes elapsed.
    view(bucket, healthy_data(), Status::Stale(-18 * 60))
}

#[must_use]
pub fn rate_limited(bucket: SizeBucket) -> ViewData {
    view(bucket, healthy_data(), Status::RateLimited)
}

#[must_use]
pub fn extremes(bucket: SizeBucket) -> ViewData {
    let mut data = healthy_data();
    if let Availability::Available(stats) = &mut data.hashrate_stats {
        stats.current_ehs = Some(1_250_000.0);
        stats.hashprice_per_th_day = Some(9_999.999_9);
        stats.revenue = Some(9_999_999_999.99);
    }
    if let Availability::Available(stats) = &mut data.difficulty_stats {
        stats.previous_adjustment_percent = Some(-99.99);
        stats.estimated_adjustment_percent = Some(999.99);
    }
    view(bucket, data, Status::Ready)
}

#[must_use]
pub fn unit_rollover(bucket: SizeBucket) -> ViewData {
    let mut data = healthy_data();
    if let Availability::Available(stats) = &mut data.hashrate_stats {
        stats.hashprice_per_th_day = Some(999.999_99);
        stats.revenue = Some(999_999_999.99);
    }
    view(bucket, data, Status::Ready)
}

#[must_use]
pub fn flat_history(bucket: SizeBucket) -> ViewData {
    let mut data = healthy_data();
    if let Availability::Available(history) = &mut data.day_history {
        history.price.values.fill(169_420.0);
        history.hashrate.values.fill(877.8);
    }
    if let Availability::Available(history) = &mut data.year_history {
        history.values.fill(129.7);
    }
    view(bucket, data, Status::Ready)
}
