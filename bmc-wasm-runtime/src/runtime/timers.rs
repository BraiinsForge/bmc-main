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

//! Periodic widget timers, ticked off the runtime's monotonic clock.

use std::num::NonZeroU32;

use bmc_wasm_protocol::TimerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerRefusal {
    PeriodTooShort,
    AtCap,
    /// The widget exports no `__on_timer`, so no tick could reach it.
    NoOnTimerExport,
    /// The widget has used every timer id.
    IdsExhausted,
}

#[derive(Debug)]
struct Timer {
    id: TimerId,
    period_ms: u64,
    due_at_ms: u64,
}

impl Timer {
    /// When the next tick is due, read against `now_ms`.
    ///
    /// `set_time` takes any reading, so the clock can go backwards;
    /// a rewound clock waits at most one period rather than until it catches up.
    fn due_at(&self, now_ms: u64) -> u64 {
        self.due_at_ms.min(now_ms.saturating_add(self.period_ms))
    }
}

/// A widget's running timers.
///
/// A `Vec` rather than a map:
/// it holds a handful of entries, and ticks due together fire in start order.
#[derive(Debug)]
pub(crate) struct TimerTable {
    timers: Vec<Timer>,
    max_timers: usize,
    min_period_ms: NonZeroU32,
    next_id: u32,
    exports_on_timer: bool,
    clock_started: bool,
}

impl TimerTable {
    pub(crate) fn new(max_timers: usize, min_period_ms: NonZeroU32) -> Self {
        Self {
            timers: Vec::new(),
            max_timers,
            min_period_ms,
            next_id: 1,
            exports_on_timer: false,
            clock_started: false,
        }
    }

    pub(crate) fn set_exports_on_timer(&mut self, present: bool) {
        self.exports_on_timer = present;
    }

    /// Take a reading of the runtime's clock.
    ///
    /// The clock reads 0 until the embedder first sets it,
    /// which can be long after `init` started a timer:
    /// the testbed runs every widget off one clock that started with the app.
    /// Timers started before that first reading count from it instead.
    pub(crate) fn observe_clock(&mut self, now_ms: u64) {
        if self.clock_started {
            return;
        }
        self.clock_started = true;
        for timer in &mut self.timers {
            timer.due_at_ms = now_ms.saturating_add(timer.period_ms);
        }
    }

    /// Start a timer whose first tick is due one period after `now_ms`.
    pub(crate) fn start(&mut self, period_ms: u32, now_ms: u64) -> Result<TimerId, TimerRefusal> {
        if !self.exports_on_timer {
            return Err(TimerRefusal::NoOnTimerExport);
        }
        if period_ms < self.min_period_ms.get() {
            return Err(TimerRefusal::PeriodTooShort);
        }
        if self.timers.len() >= self.max_timers {
            return Err(TimerRefusal::AtCap);
        }
        if self.next_id == u32::MAX {
            return Err(TimerRefusal::IdsExhausted);
        }
        let id = TimerId::alloc(&mut self.next_id);
        let period_ms = u64::from(period_ms);
        self.timers.push(Timer {
            id,
            period_ms,
            due_at_ms: now_ms.saturating_add(period_ms),
        });
        Ok(id)
    }

    pub(crate) fn stop(&mut self, id: TimerId) {
        self.timers.retain(|timer| timer.id != id);
    }

    pub(crate) fn is_running(&self, id: TimerId) -> bool {
        self.timers.iter().any(|timer| timer.id == id)
    }

    /// Collect one tick per due timer, with the number of periods it covers.
    ///
    /// Missed periods coalesce into that one tick, and the next tick stays
    /// on the timer's original period boundaries.
    pub(crate) fn take_due(&mut self, now_ms: u64) -> Vec<(TimerId, u32)> {
        let mut ticks = Vec::new();
        for timer in &mut self.timers {
            timer.due_at_ms = timer.due_at(now_ms);
            if now_ms < timer.due_at_ms {
                continue;
            }
            #[expect(
                clippy::integer_division,
                reason = "a tick covers whole periods; the remainder is the next one under way"
            )]
            let elapsed = (now_ms - timer.due_at_ms) / timer.period_ms + 1;
            timer.due_at_ms = timer
                .due_at_ms
                .saturating_add(elapsed.saturating_mul(timer.period_ms));
            ticks.push((timer.id, u32::try_from(elapsed).unwrap_or(u32::MAX)));
        }
        ticks
    }

    /// Milliseconds from `now_ms` until the earliest tick is due.
    pub(crate) fn next_due_in_ms(&self, now_ms: u64) -> Option<u64> {
        self.timers
            .iter()
            .map(|timer| timer.due_at(now_ms).saturating_sub(now_ms))
            .min()
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::{TimerRefusal, TimerTable};

    const SECOND: u32 = 1_000;

    fn table() -> TimerTable {
        table_with_cap(4)
    }

    fn table_with_cap(max_timers: usize) -> TimerTable {
        table_with(max_timers, SECOND)
    }

    fn table_with(max_timers: usize, min_period_ms: u32) -> TimerTable {
        let min_period_ms =
            NonZeroU32::new(min_period_ms).expect("BUG: test minimums are non-zero");
        let mut table = TimerTable::new(max_timers, min_period_ms);
        table.set_exports_on_timer(true);
        table
    }

    #[test]
    fn nothing_is_due_before_the_first_period_ends() {
        let mut table = table();
        table.start(SECOND, 0).expect("BUG: a valid timer");

        assert!(table.take_due(999).is_empty());
    }

    #[test]
    fn an_on_time_tick_covers_one_period() {
        let mut table = table();
        let id = table.start(SECOND, 0).expect("BUG: a valid timer");

        assert_eq!(table.take_due(1_000), vec![(id, 1)]);
    }

    #[test]
    fn missed_periods_coalesce_into_one_tick() {
        let mut table = table();
        let id = table.start(SECOND, 0).expect("BUG: a valid timer");
        table.take_due(1_000);

        assert_eq!(
            table.take_due(4_500),
            vec![(id, 3)],
            "ticks were due at 2 s, 3 s and 4 s"
        );
    }

    #[test]
    fn a_late_tick_keeps_the_original_period_boundaries() {
        let mut table = table();
        let id = table.start(SECOND, 0).expect("BUG: a valid timer");
        table.take_due(1_700);

        assert!(table.take_due(1_999).is_empty());
        assert_eq!(table.take_due(2_000), vec![(id, 1)]);
    }

    #[test]
    fn a_rewound_clock_brings_the_next_tick_within_one_period() {
        let mut table = table();
        let id = table.start(SECOND, 50_000).expect("BUG: a valid timer");

        assert!(table.take_due(0).is_empty());
        assert_eq!(table.take_due(1_000), vec![(id, 1)]);
    }

    #[test]
    fn a_rewound_clock_reports_the_next_tick_within_one_period() {
        let mut table = table();
        table.start(SECOND, 50_000).expect("BUG: a valid timer");

        assert_eq!(table.next_due_in_ms(0), Some(SECOND.into()));
    }

    #[test]
    fn a_timer_started_before_the_first_clock_reading_counts_from_it() {
        let mut table = table();
        let id = table.start(SECOND, 0).expect("BUG: a valid timer");

        table.observe_clock(600_000);

        assert!(table.take_due(600_999).is_empty());
        assert_eq!(table.take_due(601_000), vec![(id, 1)]);
    }

    #[test]
    fn only_the_first_clock_reading_moves_a_timer() {
        let mut table = table();
        table.observe_clock(0);
        let id = table.start(SECOND, 0).expect("BUG: a valid timer");

        table.observe_clock(500);

        assert_eq!(table.take_due(1_000), vec![(id, 1)]);
    }

    #[test]
    fn a_period_below_the_configured_minimum_is_refused() {
        let mut table = table_with(4, 5 * SECOND);

        assert_eq!(
            table.start(5 * SECOND - 1, 0),
            Err(TimerRefusal::PeriodTooShort)
        );
        assert!(table.start(5 * SECOND, 0).is_ok());
    }

    #[test]
    fn a_timer_past_the_cap_is_refused() {
        let mut table = table_with_cap(1);
        table
            .start(SECOND, 0)
            .expect("BUG: the first timer fits the cap");

        assert_eq!(table.start(SECOND, 0), Err(TimerRefusal::AtCap));
    }

    #[test]
    fn a_widget_without_the_on_timer_export_is_refused() {
        assert_eq!(
            TimerTable::new(4, NonZeroU32::MIN).start(SECOND, 0),
            Err(TimerRefusal::NoOnTimerExport)
        );
    }

    #[test]
    fn a_widget_that_used_every_id_is_refused_instead_of_panicking() {
        let mut table = table();
        table.next_id = u32::MAX - 1;
        table
            .start(SECOND, 0)
            .expect("BUG: the last id is still free");

        assert_eq!(table.start(SECOND, 0), Err(TimerRefusal::IdsExhausted));
    }

    #[test]
    fn a_stopped_timer_no_longer_ticks_or_counts_against_the_cap() {
        let mut table = table_with_cap(1);
        let id = table.start(SECOND, 0).expect("BUG: a valid timer");

        table.stop(id);

        assert!(!table.is_running(id));
        assert!(table.take_due(5_000).is_empty());
        table
            .start(SECOND, 5_000)
            .expect("BUG: the stopped timer freed its slot");
    }

    #[test]
    fn stopping_a_stopped_timer_is_harmless() {
        let mut table = table();
        let id = table.start(SECOND, 0).expect("BUG: a valid timer");
        table.stop(id);

        table.stop(id);

        assert!(!table.is_running(id));
    }

    #[test]
    fn the_next_due_tick_is_the_earliest_one() {
        let mut table = table();
        table.start(3 * SECOND, 0).expect("BUG: a valid timer");
        table.start(SECOND, 0).expect("BUG: a valid timer");

        assert_eq!(table.next_due_in_ms(300), Some(700));
        assert_eq!(TimerTable::new(4, NonZeroU32::MIN).next_due_in_ms(0), None);
    }
}
