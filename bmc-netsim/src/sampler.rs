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

//! Drive a device's opt-in sampler into its cache: backfill each ring to
//! capacity at past `t`, then append fresh samples on a background tick.

use std::sync::Arc;
use std::time::Instant;

use tokio::time::{Duration, interval};

use crate::blueprint::{Sampler, SeriesSpec};
use crate::cache::{Cache, Sample};
use crate::noise::mix;

/// The `(series name, sample)` for one series evaluated at `t_s`.
fn sample_at(series: &SeriesSpec, seed: u64, t_s: f64) -> (&str, Sample) {
    let value = series.value.eval(t_s, mix(seed, &series.name));
    (series.name.as_str(), Sample { t_s, value })
}

/// Fill every ring to capacity via `push_row`, at `t = -(cap-1)·period … 0`.
pub fn backfill(cache: &Cache, sampler: &Sampler, seed: u64) {
    let depth = sampler.series.iter().map(|s| s.capacity).max().unwrap_or(0);
    for step in (0..depth).rev() {
        let t_s = offset_s(step, sampler.period_s);
        let row: Vec<(&str, Sample)> = sampler
            .series
            .iter()
            .filter(|series| step < series.capacity)
            .map(|series| sample_at(series, seed, t_s))
            .collect();
        cache.push_row(&row);
    }
}

/// Backfill, then append one row per tick every `period_s`. `start` is shared
/// with the responder so a fresh sample matches the live reading.
pub async fn run(cache: Arc<Cache>, sampler: Sampler, seed: u64, start: Instant) {
    assert!(
        sampler.period_s > 0.0,
        "BUG: sampler period must be positive, got {}",
        sampler.period_s,
    );
    backfill(&cache, &sampler, seed);
    let mut ticker = interval(Duration::from_secs_f64(sampler.period_s));
    loop {
        ticker.tick().await;
        let t_s = start.elapsed().as_secs_f64();
        let row: Vec<(&str, Sample)> = sampler
            .series
            .iter()
            .map(|series| sample_at(series, seed, t_s))
            .collect();
        cache.push_row(&row);
    }
}

/// Scenario time of the `step`-th backfill sample counting back from now.
#[expect(
    clippy::cast_precision_loss,
    reason = "ring capacity is at most a few hundred, exact in f64"
)]
fn offset_s(step: usize, period_s: f64) -> f64 {
    -(step as f64) * period_s
}

#[cfg(test)]
mod tests {
    use super::backfill;
    use crate::blueprint::{Sampler, SeriesSpec};
    use crate::cache::Cache;
    use crate::value::Value;

    #[test]
    fn backfill_fills_ring_to_capacity_in_time_order() {
        let sampler = Sampler {
            period_s: 2.0,
            series: vec![SeriesSpec {
                name: "hashrate".to_owned(),
                value: Value::Drift {
                    center: 100.0,
                    amp: 2.0,
                    period_s: 300.0,
                    jitter: 0.0,
                },
                capacity: 4,
            }],
        };
        let cache = Cache::new([("hashrate".to_owned(), 4)]);
        backfill(&cache, &sampler, 0xABCD);
        // Ring filled to capacity, oldest first, spaced by period, ending at t=0.
        let times: Vec<f64> = cache
            .rows(&["hashrate"])
            .iter()
            .map(|row| row.t_s)
            .collect();
        assert_eq!(times, vec![-6.0, -4.0, -2.0, 0.0]);
    }
}
