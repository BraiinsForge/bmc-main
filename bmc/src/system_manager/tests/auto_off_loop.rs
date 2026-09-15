// Copyright (C) 2025  Braiins Systems s.r.o.
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

use std::sync::Arc;
use std::time::Duration;

use crate::backlight::{DisplayBacklightController, DisplayBacklightDriver};
use crate::config::ConfigHandle;
use crate::night_mode::NightModeController;
use crate::system_manager::{MIN_SCREEN_OFF_TIMEOUT_SECS, ScreenRequest, SystemManager};
use bmc_scheduler::JobScheduler;
use bmc_shared_time::time::Timezone;
use tokio::sync::{Mutex, Notify, RwLock, broadcast, watch};

use super::ScriptedBacklightDriver;

const ALARM_RINGING: bool = true;
const NO_ALARM: bool = false;

/// How long a test gives the loop to react before calling the signal lost.
/// The loop answers a written request on its next poll,
/// with only uncontended lock reads in between, so this is slack.
/// The tests run on the paused clock, where the slack costs nothing:
/// time jumps ahead only once every task is idle.
const AUTO_OFF_REACTION: Duration = Duration::from_secs(5);

/// Enough turns of the scheduler for the loop to leave its select, decide,
/// and reach the panel lock. Overshooting is free; the lock holds it there.
const YIELDS_TO_REACH_THE_PANEL: usize = 16;

/// How long a test waits before concluding the loop ignored a signal,
/// as it was meant to. On the paused clock this is a full settle:
/// the jump happens only after the loop has nothing left to run.
const AUTO_OFF_SETTLE: Duration = Duration::from_millis(250);

/// How often a wait re-reads the panel. A `yield_now` spin would do,
/// but it never leaves the runtime idle, so the paused clock could not
/// reach the timeout and a failing wait would hang instead of failing.
const PANEL_POLL: Duration = Duration::from_millis(1);

/// `run_screen_auto_off` wired to a scriptable panel,
/// so a test can post the signals the button and alarm tasks post,
/// and read the panel back.
struct AutoOffHarness {
    _tmp: tempfile::TempDir,
    driver: ScriptedBacklightDriver,
    /// The same lock the loop takes for every panel read and write,
    /// so a test can hold the loop inside the blank sequence.
    panel: Arc<Mutex<ScriptedBacklightDriver>>,
    /// The lock the loop takes to read the timeout at the top of every pass,
    /// so a test can hold the loop there.
    config: Arc<RwLock<ConfigHandle>>,
    screen_request: watch::Sender<ScreenRequest>,
    alarm_ringing: watch::Sender<bool>,
    blanked: broadcast::Receiver<()>,
    night_mode: NightModeController,
    /// Kept alive: the night-mode controller stops tracking a closed clock.
    _timezone: watch::Sender<Timezone>,
}

impl AutoOffHarness {
    fn request_display_off(&self) {
        self.screen_request.send_replace(ScreenRequest::Blank);
    }

    fn report_activity(&self) {
        self.screen_request.send_replace(ScreenRequest::Wake);
    }

    fn ring_alarm(&self) {
        self.set_alarm(true);
    }

    fn stop_alarm(&self) {
        self.set_alarm(false);
    }

    fn set_alarm(&self, ringing: bool) {
        self.alarm_ringing
            .send(ringing)
            .expect("BUG: the auto-off loop must still hold the alarm receiver");
    }

    async fn set_screen_off_timeout(&self, timeout: Option<u32>) {
        self.night_mode
            .set_screen_off_timeout(timeout)
            .await
            .expect("BUG: the test config must accept a screen-off timeout");
    }

    /// Wait out the shortest timeout and the loop's reaction to it.
    async fn wait_for_timer_blank(&mut self) {
        tokio::time::timeout(
            Duration::from_secs(u64::from(MIN_SCREEN_OFF_TIMEOUT_SECS)) + AUTO_OFF_REACTION,
            self.blanked.recv(),
        )
        .await
        .expect("the timer never blanked the panel")
        .expect("BUG: the auto-off loop must still hold the blanked sender");
        assert!(
            !self.panel_is_lit(),
            "the timer's blank was announced while the panel was still lit"
        );
    }

    fn panel_is_lit(&self) -> bool {
        self.driver
            .is_visible()
            .expect("BUG: the scripted driver always reads")
    }

    /// Wait for the loop to announce a blank. Says nothing about the panel
    /// now: a wake that raced the blank may already have lit it again.
    async fn wait_for_blank_announcement(&mut self) {
        tokio::time::timeout(AUTO_OFF_REACTION, self.blanked.recv())
            .await
            .expect("the blank was never announced")
            .expect("BUG: the auto-off loop must still hold the blanked sender");
    }

    /// Wait for the panel to go dark and the blank to be announced.
    async fn wait_for_blank(&mut self) {
        self.wait_for_blank_announcement().await;
        assert!(
            !self.panel_is_lit(),
            "the blank was announced while the panel was still lit"
        );
    }

    async fn wait_for_wake(&self) {
        tokio::time::timeout(AUTO_OFF_REACTION, async {
            while !self.panel_is_lit() {
                tokio::time::sleep(PANEL_POLL).await;
            }
        })
        .await
        .expect("the panel never lit again");
    }

    /// Give the loop its window, then answer by both measures
    /// a blank leaves behind: the announcement and the panel itself.
    async fn blanked_after_settling(&mut self) -> bool {
        tokio::time::sleep(AUTO_OFF_SETTLE).await;
        self.blanked.try_recv().is_ok() || !self.panel_is_lit()
    }
}

async fn auto_off_harness(alarm_ringing: bool) -> AutoOffHarness {
    let driver = ScriptedBacklightDriver::new(true, 50);
    let tmp = tempfile::tempdir().expect("BUG: tempdir creation must succeed in tests");
    let (handle, _extracted_accounts) = ConfigHandle::init(
        tmp.path().join("bmc-config.json"),
        50,
        50,
        50,
        50,
        bmc_platform::Product::Bmc100,
    )
    .await;
    let config_handle = Arc::new(RwLock::new(handle));
    let panel = Arc::new(Mutex::new(driver.clone()));
    let backlight = DisplayBacklightController::new(config_handle.clone(), panel.clone());

    let (timezone_tx, timezone_rx) = watch::channel(Timezone::default());
    let scheduler = JobScheduler::init(timezone_rx.clone(), Some(tmp.path().join("crontab"))).await;
    let night_mode_controller =
        NightModeController::init(config_handle.clone(), scheduler, timezone_rx).await;

    let timeout_changed = config_handle
        .read()
        .await
        .subscribe_screen_off_timeout_change();
    let (screen_blanked_tx, blanked) = broadcast::channel(4);
    let (screen_request, _) = watch::channel(ScreenRequest::Wake);
    let brightness_modified = Arc::new(Notify::new());
    let (alarm_tx, alarm_rx) = watch::channel(alarm_ringing);

    // The wake path only pokes `brightness_modified`.
    // Without this task the panel comes back powered at brightness zero,
    // which reads as dark.
    tokio::spawn(
        SystemManager::<ScriptedBacklightDriver>::set_current_brightness(
            backlight.clone(),
            night_mode_controller.clone(),
            brightness_modified.clone(),
        ),
    );
    tokio::spawn(
        SystemManager::<ScriptedBacklightDriver>::run_screen_auto_off(
            backlight,
            night_mode_controller.clone(),
            brightness_modified,
            screen_request.clone(),
            timeout_changed,
            screen_blanked_tx,
            alarm_rx,
        ),
    );

    AutoOffHarness {
        _tmp: tmp,
        driver,
        panel,
        config: config_handle,
        screen_request,
        alarm_ringing: alarm_tx,
        blanked,
        night_mode: night_mode_controller,
        _timezone: timezone_tx,
    }
}

/// Night mode on with the shortest timeout, and the panel already dark from the timer,
/// so a test can pose what ought to relight it.
async fn timer_blanked_harness() -> AutoOffHarness {
    let mut harness = auto_off_harness(NO_ALARM).await;
    harness
        .set_screen_off_timeout(Some(MIN_SCREEN_OFF_TIMEOUT_SECS))
        .await;
    harness
        .night_mode
        .toggle()
        .await
        .expect("BUG: toggling night mode on a fresh config must succeed");
    harness.wait_for_timer_blank().await;
    harness
}

#[tokio::test(start_paused = true)]
async fn a_display_off_request_blanks_the_panel_and_announces_it() {
    let mut harness = auto_off_harness(NO_ALARM).await;

    harness.request_display_off();

    harness.wait_for_blank().await;
}

#[tokio::test(start_paused = true)]
async fn a_blanked_panel_stays_dark_until_activity() {
    let mut harness = auto_off_harness(NO_ALARM).await;
    harness.request_display_off();
    harness.wait_for_blank().await;

    // The announcement arrives before the loop is back in its select,
    // so this wait gives an unasked-for wake the room to happen.
    tokio::time::sleep(AUTO_OFF_SETTLE).await;
    assert!(
        !harness.panel_is_lit(),
        "nothing asked for the panel back, and the blank still did not hold"
    );

    harness.report_activity();

    harness.wait_for_wake().await;
}

#[tokio::test(start_paused = true)]
async fn a_ringing_alarm_refuses_a_display_off_request() {
    let mut harness = auto_off_harness(ALARM_RINGING).await;

    harness.request_display_off();

    assert!(
        !harness.blanked_after_settling().await,
        "the panel blanked under a ringing alarm, so the scene reset that \
         follows the announcement landed on the firing-alarm screen"
    );
    // A blank the loop must still deliver,
    // so the window above cannot have passed for want of a running loop.
    harness.stop_alarm();
    harness.request_display_off();
    harness.wait_for_blank().await;
}

#[tokio::test(start_paused = true)]
async fn activity_during_the_blank_still_brings_the_panel_back() {
    let mut harness = auto_off_harness(NO_ALARM).await;

    // Holding the panel lock pins the loop inside `blank_screen`,
    // where it reads no requests, so the wake lands in the window
    // a reader that consumed signals would have missed.
    let held = harness.panel.clone().lock_owned().await;
    harness.request_display_off();
    for _ in 0..YIELDS_TO_REACH_THE_PANEL {
        tokio::task::yield_now().await;
    }
    harness.report_activity();
    drop(held);

    // The blank has to have happened, or the wake landed before the loop
    // read the request and the window went untested.
    harness.wait_for_blank_announcement().await;
    harness.wait_for_wake().await;
}

#[tokio::test(start_paused = true)]
async fn activity_during_the_timers_blank_still_brings_the_panel_back() {
    let mut harness = auto_off_harness(NO_ALARM).await;
    harness
        .set_screen_off_timeout(Some(MIN_SCREEN_OFF_TIMEOUT_SECS))
        .await;
    harness
        .night_mode
        .toggle()
        .await
        .expect("BUG: toggling night mode on a fresh config must succeed");
    // The loop has to be counting down before the lock is taken,
    // or the lock pins it in the wake arm and the timer never arms.
    tokio::time::sleep(AUTO_OFF_SETTLE).await;

    // Holding the panel lock across the timeout pins the loop inside the
    // timer's own `blank_screen`, the blank that happens after the request
    // was read for this iteration.
    let held = harness.panel.clone().lock_owned().await;
    tokio::time::sleep(
        Duration::from_secs(u64::from(MIN_SCREEN_OFF_TIMEOUT_SECS)) + AUTO_OFF_SETTLE,
    )
    .await;
    harness.report_activity();
    drop(held);

    harness.wait_for_blank_announcement().await;
    harness.wait_for_wake().await;
}

#[tokio::test(start_paused = true)]
async fn a_hold_landing_as_the_alarm_stops_is_not_refused_against_the_stale_ring() {
    let mut harness = auto_off_harness(ALARM_RINGING).await;
    // The loop has to be in its select before the lock is taken,
    // or the lock pins its first pass instead of the one posed below.
    tokio::time::sleep(AUTO_OFF_SETTLE).await;

    // Holding the config lock pins the loop inside the timeout read
    // at the top of a pass, the one await between its samples.
    let held = harness.config.clone().write_owned().await;
    harness.report_activity();
    tokio::time::sleep(AUTO_OFF_SETTLE).await;
    harness.stop_alarm();
    harness.request_display_off();
    drop(held);

    harness.wait_for_blank().await;
}

#[tokio::test(start_paused = true)]
async fn a_refused_display_off_request_is_not_replayed_when_the_alarm_stops() {
    let mut harness = auto_off_harness(ALARM_RINGING).await;
    harness.request_display_off();
    assert!(
        !harness.blanked_after_settling().await,
        "the panel blanked under the ringing alarm, so the replay check below has nothing to prove"
    );

    harness.stop_alarm();

    assert!(
        !harness.blanked_after_settling().await,
        "the refused request was replayed the moment the alarm stopped"
    );
    // A blank the loop must still deliver,
    // so the window above cannot have passed for want of a running loop.
    harness.request_display_off();
    harness.wait_for_blank().await;
}

#[tokio::test(start_paused = true)]
async fn a_timeout_of_never_relights_the_timers_own_blank() {
    let harness = timer_blanked_harness().await;

    harness.set_screen_off_timeout(None).await;

    harness.wait_for_wake().await;
}

#[tokio::test(start_paused = true)]
async fn an_alarm_ending_re_arms_the_timer_instead_of_re_blanking() {
    let mut harness = timer_blanked_harness().await;
    harness.ring_alarm();
    harness.wait_for_wake().await;

    harness.stop_alarm();

    assert!(
        !harness.blanked_after_settling().await,
        "the panel went dark the moment the alarm stopped, without waiting out the timeout"
    );
    harness.wait_for_timer_blank().await;
}
