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

use std::sync::{Arc, Mutex};

use bmc_shared_time::time::Timezone;
use chrono::{DateTime, LocalResult, NaiveDateTime, NaiveTime, Offset, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use tokio::sync::{Notify, watch};
use tracing::warn;

#[cfg(test)]
mod tests;

pub(crate) type WallClock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// A stretch of local time that repeats every day, `[from, to)`,
/// wrapping past midnight when `from` is later than `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DailyWindow {
    pub(crate) from: NaiveTime,
    pub(crate) to: NaiveTime,
}

impl DailyWindow {
    pub(crate) fn contains(self, time: NaiveTime) -> bool {
        if self.from <= self.to {
            time >= self.from && time < self.to
        } else {
            time >= self.from || time < self.to
        }
    }

    /// Whether local time in `tz` lies inside the window at `now`, each edge taking effect once.
    /// Through the second pass of an autumn fold the window keeps the state the first pass ended in,
    /// so it neither drops out nor re-enters for the repeated hour.
    fn contains_at(self, tz: Tz, now: DateTime<Utc>) -> bool {
        let first_pass_end = match tz.from_local_datetime(&now.with_timezone(&tz).naive_local()) {
            LocalResult::Ambiguous(first_pass, _) if first_pass < now => {
                first_offset_change(tz, first_pass.with_timezone(&Utc), now)
            }
            LocalResult::Single(_) | LocalResult::Ambiguous(..) | LocalResult::None => None,
        };
        let as_of = first_pass_end.map_or(now, |fold| fold - TimeDelta::nanoseconds(1));
        self.contains(as_of.with_timezone(&tz).time())
    }

    /// The first instant after `now` at which `contains_at` may answer differently:
    /// the next edge, or an earlier UTC-offset change, which moves local time without passing an edge.
    fn next_change(self, tz: Tz, now: DateTime<Utc>) -> DateTime<Utc> {
        let next_edge = self.next_edge(tz, now).unwrap_or_else(|| {
            warn!(window = ?self, %tz, %now, "No edge of the daily window found, re-checking in a day");
            now + TimeDelta::days(1)
        });
        first_offset_change(tz, now, next_edge).unwrap_or(next_edge)
    }

    /// The first edge after `now`, from yesterday's to the day after tomorrow's.
    /// A spring-forward gap can swallow both of tomorrow's edges.
    fn next_edge(self, tz: Tz, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let today = now.with_timezone(&tz).date_naive();
        today
            .pred_opt()
            .unwrap_or(today)
            .iter_days()
            .take(4)
            .flat_map(|date| [date.and_time(self.from), date.and_time(self.to)])
            .filter_map(|local| first_instant_of(tz, local))
            .filter(|instant| *instant > now)
            .min()
    }
}

/// The first instant at which `tz` shows `local`, none in a spring-forward gap.
fn first_instant_of(tz: Tz, local: NaiveDateTime) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&local)
        .earliest()
        .map(|instant| instant.with_timezone(&Utc))
}

/// The first instant in `(after, until]` with a UTC offset other than the one at `after`.
///
/// Searches whole seconds, which is where offset changes fall.
fn first_offset_change(
    tz: Tz,
    after: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let offset_at =
        |instant: DateTime<Utc>| tz.offset_from_utc_datetime(&instant.naive_utc()).fix();
    let offset_at_second = |second| DateTime::from_timestamp(second, 0).map(offset_at);
    let offset_before = offset_at(after);
    if offset_at(until) == offset_before {
        return None;
    }

    let (mut unchanged, mut changed) = (after.timestamp(), until.timestamp());
    while changed - unchanged > 1 {
        let middle = unchanged.midpoint(changed);
        if offset_at_second(middle) == Some(offset_before) {
            unchanged = middle;
        } else {
            changed = middle;
        }
    }
    DateTime::from_timestamp(changed, 0)
}

/// Publishes whether local wall-clock time lies inside a [`DailyWindow`].
///
/// The answer is re-checked at every edge and UTC-offset change, on every clock step and timezone change,
/// and whenever the window is replaced.
#[derive(Clone)]
pub(crate) struct DailyWindowWatch(Arc<Shared>);

struct Shared {
    window: Mutex<Option<DailyWindow>>,
    timezone: watch::Receiver<Timezone>,
    clock: WallClock,
    inside: watch::Sender<bool>,
    replaced: Notify,
}

impl DailyWindowWatch {
    /// Starts with no window, so never inside until [`Self::set`] gives it one.
    pub(crate) fn start(
        timezone: watch::Receiver<Timezone>,
        clock_steps: watch::Receiver<u64>,
    ) -> Self {
        Self::start_with_clock(timezone, clock_steps, Arc::new(Utc::now))
    }

    pub(crate) fn start_with_clock(
        timezone: watch::Receiver<Timezone>,
        clock_steps: watch::Receiver<u64>,
        clock: WallClock,
    ) -> Self {
        let shared = Arc::new(Shared {
            window: Mutex::new(None),
            timezone: timezone.clone(),
            clock,
            inside: watch::channel(false).0,
            replaced: Notify::new(),
        });
        tokio::spawn(follow(shared.clone(), timezone, clock_steps));
        Self(shared)
    }

    /// Replaces the window, `None` meaning never inside, and publishes the new answer before returning.
    pub(crate) fn set(&self, window: Option<DailyWindow>) {
        let mut current = self.0.lock_window();
        *current = window;
        self.0.publish(window);
        drop(current);
        self.0.replaced.notify_one();
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.0.inside.subscribe()
    }
}

impl Shared {
    fn lock_window(&self) -> std::sync::MutexGuard<'_, Option<DailyWindow>> {
        self.window.lock().expect("BUG: daily window lock poisoned")
    }

    /// Publishes the answer for the current window and returns when it may next change.
    fn recheck(&self) -> Option<DateTime<Utc>> {
        // Publishing under the lock keeps a concurrent `set` from being overwritten by a stale answer.
        let window = self.lock_window();
        self.publish(*window)
    }

    fn publish(&self, window: Option<DailyWindow>) -> Option<DateTime<Utc>> {
        let now = (self.clock)();
        let tz = *self.timezone.borrow().chrono();
        let inside = window.is_some_and(|window| window.contains_at(tz, now));
        if *self.inside.borrow() != inside {
            self.inside.send_replace(inside);
        }
        window.map(|window| window.next_change(tz, now))
    }
}

async fn follow(
    shared: Arc<Shared>,
    mut timezone: watch::Receiver<Timezone>,
    mut clock_steps: watch::Receiver<u64>,
) {
    let mut timezone_open = true;
    let mut clock_steps_open = true;

    loop {
        let next_change = shared.recheck();
        let until_next_change = async {
            match next_change {
                Some(at) => {
                    let delay = (at - (shared.clock)()).to_std().unwrap_or_default();
                    tokio::time::sleep(delay).await;
                }
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            () = until_next_change => {}
            () = shared.replaced.notified() => {}
            changed = timezone.changed(), if timezone_open => timezone_open = changed.is_ok(),
            changed = clock_steps.changed(), if clock_steps_open => clock_steps_open = changed.is_ok(),
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use bmc_shared_time::time::Timezone;
    use chrono::{DateTime, NaiveTime, TimeZone, Utc};
    use tokio::time::Instant;

    use super::WallClock;

    /// How long a task gets to handle what a test sent it, short of any edge in the tests.
    pub(crate) const SETTLE: Duration = Duration::from_secs(1);

    pub(crate) fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("BUG: test timestamps must be unambiguous")
    }

    pub(crate) fn hm(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).expect("BUG: test times must be valid")
    }

    /// CET (UTC+1) in winter, CEST (UTC+2) in summer.
    pub(crate) fn prague() -> Timezone {
        Timezone::lookup("Europe/Prague")
            .expect("BUG: Europe/Prague is a known timezone")
            .clone()
    }

    /// A wall clock that runs with tokio's clock, so a paused test advances it by sleeping,
    /// and that a test can step like NTP would.
    #[derive(Clone)]
    pub(crate) struct FakeWallClock {
        started: Instant,
        start: Arc<Mutex<DateTime<Utc>>>,
    }

    impl FakeWallClock {
        pub(crate) fn starting_at(now: DateTime<Utc>) -> Self {
            Self {
                started: Instant::now(),
                start: Arc::new(Mutex::new(now)),
            }
        }

        pub(crate) fn step_to(&self, now: DateTime<Utc>) {
            let elapsed = chrono::TimeDelta::from_std(self.started.elapsed())
                .expect("BUG: a test runs for less than chrono's range");
            *self.start.lock().expect("BUG: fake clock lock poisoned") = now - elapsed;
        }

        pub(crate) fn wall_clock(&self) -> WallClock {
            let this = self.clone();
            Arc::new(move || this.now())
        }

        fn now(&self) -> DateTime<Utc> {
            let elapsed = chrono::TimeDelta::from_std(self.started.elapsed())
                .expect("BUG: a test runs for less than chrono's range");
            *self.start.lock().expect("BUG: fake clock lock poisoned") + elapsed
        }
    }
}
