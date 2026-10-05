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

use chrono::{DateTime, Utc};

use super::*;
use crate::daily_window::testing::{FakeWallClock, SETTLE, at, hm, prague};

/// The time an RTC that lost power boots with.
fn rtc_reset() -> DateTime<Utc> {
    at(2000, 1, 1, 0, 0)
}

struct Fixture {
    _temp: tempfile::TempDir,
    timezone_sender: watch::Sender<Timezone>,
    clock_steps: watch::Sender<u64>,
    clock: FakeWallClock,
    controller: NightModeController,
    is_active: watch::Receiver<bool>,
}

impl Fixture {
    /// A controller with night mode enabled over `from..to`, booted at `now`.
    async fn booted_at(now: DateTime<Utc>, from: NaiveTime, to: NaiveTime) -> Self {
        let temp = tempfile::tempdir().expect("BUG: create night mode test directory");
        let (mut config_handle, _) = ConfigHandle::init(
            temp.path().join("bmc-config.json"),
            50,
            50,
            50,
            50,
            bmc_platform::Product::Bmc100,
        )
        .await;
        config_handle.set_night_mode_enabled(true);
        config_handle.set_night_mode_interval(from, to);
        let (timezone_sender, timezone_receiver) = watch::channel(Timezone::default());
        let (clock_steps, clock_steps_receiver) = watch::channel(0);
        let clock = FakeWallClock::starting_at(now);
        let window = DailyWindowWatch::start_with_clock(
            timezone_receiver,
            clock_steps_receiver,
            clock.wall_clock(),
        );

        let controller =
            NightModeController::with_window(Arc::new(RwLock::new(config_handle)), window).await;
        let is_active = controller.subscribe();

        Self {
            _temp: temp,
            timezone_sender,
            clock_steps,
            clock,
            controller,
            is_active,
        }
    }

    fn is_active(&self) -> bool {
        *self.is_active.borrow()
    }

    /// Steps the wall clock to `now` the way NTP or `date -s` would.
    fn step_to(&self, now: DateTime<Utc>) {
        self.clock.step_to(now);
        self.clock_steps.send_modify(|steps| *steps += 1);
    }

    async fn toggle(&self) {
        self.controller
            .toggle()
            .await
            .expect("BUG: toggling night mode must succeed in tests");
    }

    /// Waits for night mode to reach `expected` once the controller has handled what it was sent.
    async fn expect_soon(&mut self, expected: bool) {
        tokio::time::timeout(
            SETTLE,
            self.is_active.wait_for(|is_active| *is_active == expected),
        )
        .await
        .unwrap_or_else(|_| panic!("night mode did not become {expected}"))
        .expect("BUG: the controller holds the sender");
    }
}

#[tokio::test(start_paused = true)]
async fn clock_step_out_of_the_window_turns_night_mode_off() {
    let mut fixture = Fixture::booted_at(rtc_reset(), hm(22, 30), hm(6, 30)).await;
    assert!(fixture.is_active(), "midnight lies inside 22:30..06:30");

    fixture.step_to(at(2026, 9, 28, 14, 0));

    fixture.expect_soon(false).await;
}

#[tokio::test(start_paused = true)]
async fn clock_step_into_the_window_turns_night_mode_on() {
    let mut fixture = Fixture::booted_at(rtc_reset(), hm(1, 0), hm(7, 0)).await;
    assert!(!fixture.is_active(), "midnight lies outside 01:00..07:00");

    fixture.step_to(at(2026, 9, 28, 3, 0));

    fixture.expect_soon(true).await;
}

#[tokio::test(start_paused = true)]
async fn timezone_change_into_the_window_turns_night_mode_on() {
    let mut fixture = Fixture::booted_at(at(2026, 9, 28, 21, 0), hm(22, 30), hm(6, 30)).await;
    assert!(!fixture.is_active(), "21:00 GMT lies outside 22:30..06:30");

    fixture.timezone_sender.send_replace(prague());

    fixture.expect_soon(true).await;
}

#[tokio::test(start_paused = true)]
async fn changing_the_window_applies_it_at_once() {
    let fixture = Fixture::booted_at(at(2026, 9, 28, 14, 0), hm(22, 30), hm(6, 30)).await;

    fixture
        .controller
        .set_interval(hm(13, 0), hm(15, 0))
        .await
        .expect("BUG: setting the night mode interval must succeed in tests");

    assert!(
        fixture.is_active(),
        "14:00 lies inside the new 13:00..15:00"
    );
}

#[tokio::test(start_paused = true)]
async fn manual_override_survives_a_timezone_change_inside_its_window() {
    let fixture = Fixture::booted_at(at(2026, 9, 28, 23, 0), hm(22, 30), hm(6, 30)).await;
    fixture.toggle().await;

    fixture.timezone_sender.send_replace(prague());
    tokio::time::sleep(SETTLE).await;

    assert!(
        !fixture.is_active(),
        "01:00 in Prague is still inside the window, so the manual off holds"
    );
}

#[tokio::test(start_paused = true)]
async fn manual_override_survives_a_clock_step_inside_its_window() {
    let fixture = Fixture::booted_at(at(2026, 9, 28, 23, 0), hm(22, 30), hm(6, 30)).await;
    fixture.toggle().await;

    fixture.step_to(at(2026, 9, 29, 2, 0));
    tokio::time::sleep(SETTLE).await;

    assert!(
        !fixture.is_active(),
        "the manual off holds while the clock stays inside the window"
    );
}

#[tokio::test(start_paused = true)]
async fn manual_override_ends_when_a_clock_step_skips_the_window_edge() {
    let mut fixture = Fixture::booted_at(at(2026, 9, 28, 23, 0), hm(22, 30), hm(6, 30)).await;
    fixture.toggle().await;

    fixture.step_to(at(2026, 9, 29, 7, 0));
    tokio::time::sleep(SETTLE).await;
    fixture.step_to(at(2026, 9, 29, 23, 0));

    fixture.expect_soon(true).await;
}

#[tokio::test(start_paused = true)]
async fn override_taken_outside_the_window_ends_when_the_window_does() {
    let mut fixture = Fixture::booted_at(at(2026, 9, 28, 14, 0), hm(22, 30), hm(6, 30)).await;
    fixture.toggle().await;

    fixture.step_to(at(2026, 9, 28, 23, 0));
    tokio::time::sleep(SETTLE).await;
    fixture.step_to(at(2026, 9, 29, 7, 0));

    fixture.expect_soon(false).await;
}
