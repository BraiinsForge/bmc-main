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

//! How long a process waits for, and then holds, the GPU render lock.

use std::time::{Duration, Instant};

use ii_stopwatch::{Samples, SamplesSummary};

/// How often wait and hold times are logged while the lock is in use. An unused lock logs nothing.
const LOCK_REPORT_INTERVAL: Duration = Duration::from_secs(5);

/// One reporting window of how long this process waited for, and then held, the lock.
#[derive(Debug)]
pub(crate) struct LockStats {
    window_start: Instant,
    wait: Samples,
    hold: Samples,
    longest_wait: Option<(Duration, &'static str)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LockReport {
    wait: Option<SamplesSummary>,
    hold: Option<SamplesSummary>,
    longest_wait_scope: Option<&'static str>,
    window: Duration,
}

impl LockStats {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            window_start: now,
            wait: Samples::default(),
            hold: Samples::default(),
            longest_wait: None,
        }
    }

    pub(crate) fn waited(&mut self, scope: &'static str, wait: Duration) {
        self.wait.add(wait);
        if self.longest_wait.is_none_or(|(longest, _)| wait > longest) {
            self.longest_wait = Some((wait, scope));
        }
    }

    /// Records a release and returns the window's report once it is due.
    pub(crate) fn held(&mut self, hold: Duration, now: Instant) -> Option<LockReport> {
        self.hold.add(hold);
        let window = now.saturating_duration_since(self.window_start);
        (window >= LOCK_REPORT_INTERVAL).then(|| {
            self.window_start = now;
            LockReport {
                wait: self.wait.take_summary(),
                hold: self.hold.take_summary(),
                longest_wait_scope: self.longest_wait.take().map(|(_, scope)| scope),
                window,
            }
        })
    }
}

impl std::fmt::Display for LockReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let summary =
            |s: Option<SamplesSummary>| s.map_or_else(|| "0".to_owned(), |s| s.to_string());
        write!(
            f,
            "wait={} hold={} longest_wait_in={} window={:.1?}",
            summary(self.wait),
            summary(self.hold),
            self.longest_wait_scope.unwrap_or("-"),
            self.window
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{LOCK_REPORT_INTERVAL, LockStats};
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn no_lock_report_before_the_interval_has_elapsed() {
        let start = Instant::now();
        let mut stats = LockStats::new(start);
        stats.waited("compositor_render_scene", ms(2));

        assert_eq!(stats.held(ms(10), start + LOCK_REPORT_INTERVAL / 2), None);
    }

    #[test]
    fn lock_report_names_the_scope_that_waited_longest() {
        let start = Instant::now();
        let mut stats = LockStats::new(start);
        stats.waited("host_widget_delivery", ms(1));
        stats.held(ms(4), start);
        stats.waited("compositor_render_scene", ms(9));
        stats.held(ms(13), start);
        stats.waited("host_widget_delivery", ms(3));

        let report = stats
            .held(ms(4), start + LOCK_REPORT_INTERVAL)
            .expect("BUG: the window has reached its interval");
        assert_eq!(report.longest_wait_scope, Some("compositor_render_scene"));
        assert_eq!(report.wait.map(|w| (w.n, w.max)), Some((3, ms(9))));
        assert_eq!(report.hold.map(|h| (h.n, h.max)), Some((3, ms(13))));
    }

    #[test]
    fn lock_report_starts_a_fresh_window() {
        let start = Instant::now();
        let mut stats = LockStats::new(start);
        stats.waited("compositor_render_scene", ms(9));
        let _ = stats.held(ms(13), start + LOCK_REPORT_INTERVAL);

        stats.waited("host_widget_delivery", ms(1));
        let report = stats
            .held(ms(2), start + LOCK_REPORT_INTERVAL * 2)
            .expect("BUG: the second window has reached its interval");
        assert_eq!(
            report.wait.map(|w| w.max),
            Some(ms(1)),
            "old samples leaked into the window"
        );
        assert_eq!(report.longest_wait_scope, Some("host_widget_delivery"));
    }
}
