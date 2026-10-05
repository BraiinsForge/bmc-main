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

use std::time::Duration;

use super::testing::{FakeWallClock, SETTLE, at, hm, prague};
use super::*;

fn window((from_hour, from_minute): (u32, u32), (to_hour, to_minute): (u32, u32)) -> DailyWindow {
    DailyWindow {
        from: hm(from_hour, from_minute),
        to: hm(to_hour, to_minute),
    }
}

#[test]
fn contains_wraps_past_midnight() {
    let night = window((22, 30), (6, 30));

    assert!(night.contains(hm(22, 30)), "from is inclusive");
    assert!(
        night.contains(hm(0, 0)),
        "midnight lies inside 22:30..06:30"
    );
    assert!(!night.contains(hm(6, 30)), "to is exclusive");
    assert!(!night.contains(hm(14, 0)), "the afternoon lies outside");
}

#[test]
fn next_change_is_the_nearer_edge() {
    let night = window((22, 30), (6, 30));

    assert_eq!(
        night.next_change(Tz::UTC, at(2026, 9, 28, 14, 0)),
        at(2026, 9, 28, 22, 30)
    );
    assert_eq!(
        night.next_change(Tz::UTC, at(2026, 9, 28, 23, 0)),
        at(2026, 9, 29, 6, 30),
        "past midnight, the next edge is tomorrow's to"
    );
}

#[test]
fn next_change_wakes_at_spring_forward_when_from_falls_into_the_gap() {
    // 2026-03-29 in Prague, 02:00 CET jumps to 03:00 CEST at 01:00 UTC, so 02:30 never happens.
    let early = window((2, 30), (6, 0));
    let transition = at(2026, 3, 29, 1, 0);

    assert_eq!(
        early.next_change(*prague().chrono(), at(2026, 3, 29, 0, 0)),
        transition
    );
    assert!(
        early.contains(transition.with_timezone(prague().chrono()).time()),
        "the gap ends at 03:00, inside 02:30..06:00"
    );
}

#[test]
fn next_change_wakes_at_spring_forward_when_to_falls_into_the_gap() {
    // 2026-03-29 in Prague, 02:00 CET jumps to 03:00 CEST at 01:00 UTC, so 02:30 never happens.
    let night = window((22, 0), (2, 30));
    let transition = at(2026, 3, 29, 1, 0);

    assert_eq!(
        night.next_change(*prague().chrono(), at(2026, 3, 29, 0, 0)),
        transition
    );
    assert!(
        !night.contains(transition.with_timezone(prague().chrono()).time()),
        "the gap ends at 03:00, past 22:00..02:30"
    );
}

#[test]
fn next_change_skips_a_window_that_falls_wholly_into_the_gap() {
    // 2026-03-29 in Prague, 02:00 CET jumps to 03:00 CEST at 01:00 UTC, so 02:15..02:45 never happens.
    let early = window((2, 15), (2, 45));
    let tz = *prague().chrono();
    let transition = at(2026, 3, 29, 1, 0);

    assert_eq!(
        early.next_change(tz, at(2026, 3, 28, 2, 0)),
        transition,
        "the day before, past the window, the next edges are two days away"
    );
    assert_eq!(
        early.next_change(tz, transition),
        at(2026, 3, 30, 0, 15),
        "02:15 CEST on the following day"
    );
}

#[test]
fn contains_at_keeps_the_first_pass_through_the_autumn_fold() {
    // 2026-10-25 in Prague, 03:00 CEST falls back to 02:00 CET at 01:00 UTC,
    // so 02:15 shows at 00:15 UTC and again at 01:15 UTC.
    let tz = *prague().chrono();
    let first_pass = at(2026, 10, 25, 0, 15);
    let second_pass = at(2026, 10, 25, 1, 15);
    let cases = [
        (window((22, 30), (2, 30)), true, false),
        (window((2, 30), (6, 0)), false, true),
        (window((2, 15), (2, 45)), true, false),
    ];

    for (window, inside_first, inside_second) in cases {
        assert_eq!(
            window.contains_at(tz, first_pass),
            inside_first,
            "{window:?} at 02:15 CEST"
        );
        assert_eq!(
            window.contains_at(tz, second_pass),
            inside_second,
            "{window:?} at 02:15 CET keeps the state the first pass ended in"
        );
    }
}

#[test]
fn next_change_skips_the_second_pass_through_the_autumn_fold() {
    // 2026-10-25 in Prague, 03:00 CEST falls back to 02:00 CET at 01:00 UTC,
    // so 02:30 shows at 00:30 UTC and again at 01:30 UTC.
    let night = window((22, 30), (2, 30));
    let tz = *prague().chrono();

    assert_eq!(
        night.next_change(tz, at(2026, 10, 25, 0, 30)),
        at(2026, 10, 25, 1, 0)
    );
    assert_eq!(
        night.next_change(tz, at(2026, 10, 25, 1, 0)),
        at(2026, 10, 25, 21, 30),
        "the second 02:30 is no edge, so the next one is 22:30 CET"
    );
}

struct Fixture {
    timezone: watch::Sender<Timezone>,
    clock_steps: watch::Sender<u64>,
    clock: FakeWallClock,
    watch: DailyWindowWatch,
    inside: watch::Receiver<bool>,
}

impl Fixture {
    fn started_at(now: DateTime<Utc>, timezone: Timezone, window: DailyWindow) -> Self {
        let (timezone, timezone_receiver) = watch::channel(timezone);
        let (clock_steps, clock_steps_receiver) = watch::channel(0);
        let clock = FakeWallClock::starting_at(now);
        let watch = DailyWindowWatch::start_with_clock(
            timezone_receiver,
            clock_steps_receiver,
            clock.wall_clock(),
        );
        watch.set(Some(window));
        let inside = watch.subscribe();

        Self {
            timezone,
            clock_steps,
            clock,
            watch,
            inside,
        }
    }

    fn is_inside(&self) -> bool {
        *self.inside.borrow()
    }

    fn step_to(&self, now: DateTime<Utc>) {
        self.clock.step_to(now);
        self.clock_steps.send_modify(|steps| *steps += 1);
    }

    async fn expect_steady_for(&mut self, steady: Duration, expected: bool) {
        let left = tokio::time::timeout(steady, self.inside.wait_for(|inside| *inside != expected))
            .await
            .is_ok();
        assert!(!left, "the window left inside={expected} within {steady:?}");
    }

    async fn expect_within(&mut self, within: Duration, expected: bool) {
        tokio::time::timeout(within, self.inside.wait_for(|inside| *inside == expected))
            .await
            .unwrap_or_else(|_| {
                panic!("the window did not become inside={expected} within {within:?}")
            })
            .expect("BUG: the watch holds the sender");
    }
}

#[tokio::test(start_paused = true)]
async fn the_level_follows_the_edges() {
    let mut fixture = Fixture::started_at(
        at(2026, 9, 28, 22, 0),
        Timezone::default(),
        window((22, 30), (6, 30)),
    );
    assert!(!fixture.is_inside(), "22:00 lies before 22:30");

    fixture.expect_within(Duration::from_mins(31), true).await;
    fixture
        .expect_within(Duration::from_mins(8 * 60 + 1), false)
        .await;
}

#[tokio::test(start_paused = true)]
async fn a_clock_step_moves_the_level() {
    let mut fixture = Fixture::started_at(
        at(2000, 1, 1, 0, 0),
        Timezone::default(),
        window((1, 0), (7, 0)),
    );
    assert!(
        !fixture.is_inside(),
        "the RTC's midnight lies outside 01:00..07:00"
    );

    fixture.step_to(at(2026, 9, 28, 3, 0));

    fixture.expect_within(SETTLE, true).await;
}

#[tokio::test(start_paused = true)]
async fn a_timezone_change_moves_the_level() {
    let mut fixture = Fixture::started_at(
        at(2026, 9, 28, 21, 0),
        Timezone::default(),
        window((22, 30), (6, 30)),
    );
    assert!(!fixture.is_inside(), "21:00 GMT lies outside 22:30..06:30");

    fixture.timezone.send_replace(prague());

    fixture.expect_within(SETTLE, true).await;
}

#[tokio::test(start_paused = true)]
async fn set_publishes_the_new_answer_before_returning() {
    let fixture = Fixture::started_at(
        at(2026, 9, 28, 14, 0),
        Timezone::default(),
        window((22, 30), (6, 30)),
    );

    fixture.watch.set(Some(window((13, 0), (15, 0))));
    assert!(fixture.is_inside(), "14:00 lies inside 13:00..15:00");

    fixture.watch.set(None);
    assert!(!fixture.is_inside(), "without a window nothing is inside");
}

#[tokio::test(start_paused = true)]
async fn a_window_ending_in_the_autumn_fold_ends_once() {
    // 02:15 CEST; the clock then reaches 02:30 CEST, falls back to 02:00 CET and reaches 02:30 again.
    let mut fixture =
        Fixture::started_at(at(2026, 10, 25, 0, 15), prague(), window((22, 30), (2, 30)));
    assert!(fixture.is_inside(), "02:15 lies inside 22:30..02:30");

    fixture.expect_within(Duration::from_mins(16), false).await;
    fixture
        .expect_steady_for(Duration::from_hours(2), false)
        .await;
}

#[tokio::test(start_paused = true)]
async fn a_window_starting_in_the_autumn_fold_stays_on_through_it() {
    // 02:15 CEST; the clock then reaches 02:30 CEST, falls back to 02:00 CET and reaches 06:00 CET at 05:00 UTC.
    let mut fixture =
        Fixture::started_at(at(2026, 10, 25, 0, 15), prague(), window((2, 30), (6, 0)));
    assert!(!fixture.is_inside(), "02:15 lies before 02:30..06:00");

    fixture.expect_within(Duration::from_mins(16), true).await;
    fixture
        .expect_steady_for(Duration::from_mins(4 * 60 + 29), true)
        .await;
    fixture.expect_within(Duration::from_mins(2), false).await;
}

#[tokio::test(start_paused = true)]
async fn a_window_inside_the_spring_gap_is_skipped_for_a_day() {
    // 03:00 CET on 2026-03-28, past 02:15..02:45; the next night jumps from 02:00 to 03:00.
    let mut fixture =
        Fixture::started_at(at(2026, 3, 28, 2, 0), prague(), window((2, 15), (2, 45)));
    assert!(!fixture.is_inside(), "03:00 lies past 02:15..02:45");

    // 02:15 CEST on 2026-03-30 lies 46 h 15 min away.
    fixture
        .expect_steady_for(Duration::from_mins(46 * 60 + 14), false)
        .await;
    fixture.expect_within(Duration::from_mins(2), true).await;
}
