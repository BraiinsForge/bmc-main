// Copyright (C) 2021  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

use std::time::{Duration, SystemTime};

#[derive(Debug)]
pub struct Every {
    last: SystemTime,
    how_often: Duration,
}

impl Every {
    #[must_use]
    pub fn new(how_often: Duration) -> Self {
        Self {
            last: SystemTime::now(),
            how_often,
        }
    }

    pub fn has_expired(&mut self) -> bool {
        if let Ok(elapsed) = self.last.elapsed()
            && elapsed >= self.how_often
        {
            self.last = SystemTime::now();
            return true;
        }

        false
    }
}

#[cfg(feature = "enabled")]
#[macro_export]
macro_rules! every_expired {
    ($a:expr) => {
        $a.has_expired()
    };
}

#[cfg(not(feature = "enabled"))]
#[macro_export]
macro_rules! every_expired {
    ($a:expr) => {
        false
    };
}

#[derive(Debug)]
pub struct StopWatch {
    start: SystemTime,
    pub sum: Duration,
    pub min: Option<Duration>,
    pub max: Option<Duration>,
    pub n: usize,
}

impl Default for StopWatch {
    fn default() -> Self {
        Self {
            start: SystemTime::now(),
            sum: Duration::from_secs(0),
            min: None,
            max: None,
            n: 0,
        }
    }
}

impl StopWatch {
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "sample count won't exceed u32::MAX"
    )]
    pub fn avg(&self) -> Duration {
        if self.n > 0 {
            self.sum / (self.n as u32)
        } else {
            Duration::from_secs(0)
        }
    }

    pub fn reset(&mut self) {
        self.min = None;
        self.max = None;
        self.sum = Duration::from_secs(0);
        self.n = 0;
    }

    pub fn start(&mut self) {
        self.start = SystemTime::now();
    }

    pub fn stop(&mut self) {
        self.n += 1;
        if let Ok(elapsed) = self.start.elapsed() {
            self.min = self.min.map(|min| min.min(elapsed)).or(Some(elapsed));
            self.max = self.max.map(|max| max.max(elapsed)).or(Some(elapsed));
            self.sum += elapsed;
        }
    }
}

#[cfg(not(feature = "enabled"))]
#[macro_export]
macro_rules! stopwatch_start {
    ($a:expr) => {};
}

#[cfg(not(feature = "enabled"))]
#[macro_export]
macro_rules! stopwatch_stop {
    ($a:expr) => {};
}

#[cfg(not(feature = "enabled"))]
#[macro_export]
macro_rules! stopwatch_reset {
    ($a:expr) => {};
}

#[cfg(feature = "enabled")]
#[macro_export]
macro_rules! stopwatch_start {
    ($a:expr) => {
        $a.start();
    };
}

#[cfg(feature = "enabled")]
#[macro_export]
macro_rules! stopwatch_stop {
    ($a:expr) => {
        $a.stop();
    };
}

#[cfg(feature = "enabled")]
#[macro_export]
macro_rules! stopwatch_reset {
    ($a:expr) => {
        $a.reset();
    };
}

impl std::fmt::Display for StopWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{:?}/{:?}",
            self.n,
            self.avg(),
            self.max.unwrap_or(Duration::from_secs(0))
        )
    }
}

#[derive(Debug)]
pub struct Jitter {
    pub max: Duration,
    pub n: usize,
    pub sum: Duration,
}

impl Jitter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            max: Duration::from_secs(0),
            sum: Duration::from_secs(0),
            n: 0,
        }
    }

    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "sample count won't exceed u32::MAX"
    )]
    pub fn avg(&self) -> Duration {
        if self.n > 0 {
            self.sum / (self.n as u32)
        } else {
            Duration::from_secs(0)
        }
    }

    pub fn reset(&mut self) {
        self.n = 0;
        self.max = Duration::from_secs(0);
        self.sum = Duration::from_secs(0);
    }

    pub fn add(&mut self, dt: Duration) {
        self.max = self.max.max(dt);
        self.sum += dt;
        self.n += 1;
    }

    pub fn add_interval(&mut self, a: SystemTime, b: SystemTime) {
        let dt = if a > b {
            a.duration_since(b)
                .expect("BUG: duration_since failed after comparison confirmed a > b")
        } else {
            b.duration_since(a)
                .expect("BUG: duration_since failed after comparison confirmed b >= a")
        };
        self.add(dt);
    }

    pub fn add_elapsed(&mut self, a: SystemTime) {
        self.add_interval(a, SystemTime::now());
    }
}

impl Default for Jitter {
    fn default() -> Self {
        Self::new()
    }
}

/// Durations collected over one reporting window, summarised by percentile.
///
/// Unlike [`Jitter`], it keeps every sample, so size the window to the event rate.
#[derive(Debug, Default)]
pub struct Samples {
    samples: Vec<Duration>,
}

/// Nearest-rank percentiles of a [`Samples`] window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SamplesSummary {
    pub n: usize,
    pub p50: Duration,
    pub p95: Duration,
    pub max: Duration,
}

impl Samples {
    pub fn add(&mut self, sample: Duration) {
        self.samples.push(sample);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Summarises the window and empties it, keeping the allocation for the next one.
    pub fn take_summary(&mut self) -> Option<SamplesSummary> {
        self.samples.sort_unstable();
        let summary = (!self.samples.is_empty()).then(|| SamplesSummary {
            n: self.samples.len(),
            p50: self.nearest_rank(500),
            p95: self.nearest_rank(950),
            max: *self.samples.last().expect("BUG: checked non-empty above"),
        });
        self.samples.clear();
        summary
    }

    fn nearest_rank(&self, per_mille: usize) -> Duration {
        let rank = (self.samples.len() * per_mille).div_ceil(1000).max(1);
        self.samples[rank - 1]
    }
}

impl std::fmt::Display for SamplesSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{:.1?}/{:.1?}/{:.1?}",
            self.n, self.p50, self.p95, self.max
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Samples, SamplesSummary};
    use std::time::Duration;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn empty_window_has_no_summary() {
        assert_eq!(Samples::default().take_summary(), None);
    }

    #[test]
    fn summary_uses_nearest_rank_percentiles() {
        let mut samples = Samples::default();
        for n in (1..=20).rev() {
            samples.add(ms(n));
        }

        assert_eq!(
            samples.take_summary(),
            Some(SamplesSummary {
                n: 20,
                p50: ms(10),
                p95: ms(19),
                max: ms(20),
            })
        );
    }

    #[test]
    fn single_sample_is_every_percentile() {
        let mut samples = Samples::default();
        samples.add(ms(7));

        assert_eq!(
            samples.take_summary(),
            Some(SamplesSummary {
                n: 1,
                p50: ms(7),
                p95: ms(7),
                max: ms(7),
            })
        );
    }

    #[test]
    fn taking_the_summary_starts_a_new_window() {
        let mut samples = Samples::default();
        samples.add(ms(3));
        let _ = samples.take_summary();

        assert!(
            samples.is_empty(),
            "the next window must not carry old samples"
        );
    }
}
