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

//! Flip-to-flip pacing statistics.
//!
//! Only *motion* frames say anything about pacing: a scene transition or a drag, which the compositor animates itself.
//! Every other frame waits for a widget to commit, so its gap is the widget's cadence.
//! A motion frame shown `n` vblanks after the motion frame before it missed `n - 1`.

use std::{fmt, time::Duration};

use ii_stopwatch::{Samples, SamplesSummary};

/// How often a summary is logged while frames are flipping. No flips, no line.
pub const FLIP_REPORT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct FlipStats {
    vblank_period: Duration,
    last_flip: Option<Flipped>,
    pending_motion: Option<bool>,
    window_start: Option<Duration>,
    flips: usize,
    motion_gaps: Samples,
    motion_missed: usize,
}

#[derive(Clone, Copy, Debug)]
struct Flipped {
    at: Duration,
    motion: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlipReport {
    pub flips: usize,
    pub motion_gaps: Option<SamplesSummary>,
    pub motion_missed: usize,
    pub window: Duration,
}

impl FlipStats {
    #[must_use]
    pub fn new(vblank_period: Duration) -> Self {
        assert!(
            !vblank_period.is_zero(),
            "a vblank period of zero cannot pace anything"
        );
        Self {
            vblank_period,
            last_flip: None,
            pending_motion: None,
            window_start: None,
            flips: 0,
            motion_gaps: Samples::default(),
            motion_missed: 0,
        }
    }

    pub fn render_started(&mut self, motion: bool) {
        self.pending_motion = Some(motion);
    }

    /// Records a flip at the kernel's timestamp and returns the window's report once it is due.
    pub fn flipped(&mut self, at: Duration) -> Option<FlipReport> {
        let window_start = *self.window_start.get_or_insert(at);
        let motion = self.pending_motion.take().unwrap_or(false);
        self.flips += 1;
        if let Some(last) = self.last_flip
            && motion
            && last.motion
        {
            let gap = at.saturating_sub(last.at);
            self.motion_gaps.add(gap);
            self.motion_missed += self.vblanks_in(gap).saturating_sub(1);
        }
        self.last_flip = Some(Flipped { at, motion });

        let window = at.saturating_sub(window_start);
        (window >= FLIP_REPORT_INTERVAL).then(|| {
            self.window_start = Some(at);
            FlipReport {
                flips: std::mem::take(&mut self.flips),
                motion_gaps: self.motion_gaps.take_summary(),
                motion_missed: std::mem::take(&mut self.motion_missed),
                window,
            }
        })
    }

    fn vblanks_in(&self, gap: Duration) -> usize {
        let period = self.vblank_period.as_nanos();
        usize::try_from((gap.as_nanos() + period.div_euclid(2)).div_euclid(period))
            .unwrap_or(usize::MAX)
    }
}

impl FlipReport {
    /// Missed vblanks as a share of the vblanks motion frames spanned.
    #[must_use]
    pub fn motion_missed_per_mille(&self) -> usize {
        let frames = self.motion_gaps.map_or(0, |gaps| gaps.n);
        (self.motion_missed * 1000)
            .checked_div(frames + self.motion_missed)
            .unwrap_or(0)
    }
}

impl fmt::Display for FlipReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "flips={}", self.flips)?;
        if let Some(gaps) = self.motion_gaps {
            let per_mille = self.motion_missed_per_mille();
            write!(
                f,
                " motion_gap={gaps} motion_missed={} ({}.{} %)",
                self.motion_missed,
                per_mille.div_euclid(10),
                per_mille.rem_euclid(10)
            )?;
        }
        write!(f, " window={:.1?}", self.window)
    }
}

#[cfg(test)]
mod tests {
    use super::{FLIP_REPORT_INTERVAL, FlipReport, FlipStats};
    use std::time::Duration;

    const PERIOD: Duration = Duration::from_micros(16_011);

    fn at(vblanks: u32) -> Duration {
        PERIOD * vblanks
    }

    /// Flips a frame every `step` vblanks from vblank `from` until a report is due.
    /// Returns the report and the vblank it came at.
    fn run_at_steady_rate(
        stats: &mut FlipStats,
        from: u32,
        step: u32,
        motion: bool,
    ) -> (FlipReport, u32) {
        let mut vblank = from;
        loop {
            stats.render_started(motion);
            if let Some(report) = stats.flipped(at(vblank)) {
                return (report, vblank);
            }
            vblank += step;
        }
    }

    #[test]
    fn motion_at_every_vblank_misses_nothing() {
        let (report, _) = run_at_steady_rate(&mut FlipStats::new(PERIOD), 0, 1, true);

        assert_eq!(report.motion_missed, 0);
        assert_eq!(report.motion_gaps.map(|g| g.p50), Some(PERIOD));
    }

    #[test]
    fn motion_at_every_second_vblank_misses_half() {
        let (report, _) = run_at_steady_rate(&mut FlipStats::new(PERIOD), 0, 2, true);

        assert_eq!(report.motion_gaps.map(|g| g.p50), Some(PERIOD * 2));
        assert_eq!(report.motion_missed_per_mille(), 500);
    }

    #[test]
    fn widget_paced_frames_are_counted_but_never_missed() {
        let (report, _) = run_at_steady_rate(&mut FlipStats::new(PERIOD), 0, 2, false);

        assert!(report.flips > 0, "every flip counts towards the rate");
        assert_eq!(
            (report.motion_gaps, report.motion_missed),
            (None, 0),
            "a widget's cadence is not a pacing miss"
        );
    }

    #[test]
    fn the_first_motion_frame_after_widget_frames_has_no_gap() {
        let mut stats = FlipStats::new(PERIOD);
        stats.render_started(false);
        stats.flipped(at(0));
        stats.render_started(true);
        stats.flipped(at(5));

        let report = stats
            .flipped(at(1000))
            .expect("BUG: the window has run past its interval");
        assert_eq!(
            report.motion_gaps, None,
            "the gap before a transition starts is not part of its pacing"
        );
    }

    #[test]
    fn a_flip_without_a_recorded_render_start_is_not_motion() {
        let mut stats = FlipStats::new(PERIOD);
        stats.render_started(true);
        stats.flipped(at(0));
        stats.flipped(at(3));

        let report = stats
            .flipped(at(1000))
            .expect("BUG: the window has run past its interval");
        assert_eq!(report.motion_gaps, None);
    }

    #[test]
    fn kernel_timestamp_jitter_rounds_to_the_nearest_vblank() {
        let mut stats = FlipStats::new(PERIOD);
        stats.render_started(true);
        stats.flipped(at(0));
        stats.render_started(true);
        stats.flipped(at(1) + Duration::from_micros(400));

        let report = stats
            .flipped(at(1000))
            .expect("BUG: the window has run past its interval");
        assert_eq!(report.motion_missed, 0);
    }

    #[test]
    fn no_report_before_the_interval_has_elapsed() {
        let mut stats = FlipStats::new(PERIOD);
        stats.flipped(Duration::ZERO);

        assert_eq!(stats.flipped(FLIP_REPORT_INTERVAL / 2), None);
    }

    #[test]
    fn a_report_starts_a_fresh_window() {
        let mut stats = FlipStats::new(PERIOD);
        let (first, last_flip) = run_at_steady_rate(&mut stats, 0, 2, true);
        assert!(
            first.motion_missed > 0,
            "the first window should see the misses"
        );

        let (second, _) = run_at_steady_rate(&mut stats, last_flip + 1, 1, true);
        assert_eq!(
            second.motion_missed, 0,
            "misses must not leak into the next window"
        );
    }

    #[test]
    fn report_line_reads_as_percent() {
        let (report, _) = run_at_steady_rate(&mut FlipStats::new(PERIOD), 0, 2, true);

        assert!(
            report.to_string().contains("(50.0 %)"),
            "unexpected line: {report}"
        );
    }
}
