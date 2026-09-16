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

use super::super::*;
use super::AlarmFixture;

const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// A whole `AlarmController` over a tempdir config and crontab.
/// Its alarms carry no sound, so nothing reaches `bmc-audio`.
struct AlarmHarness {
    fixture: AlarmFixture,
    controller: AlarmController,
}

impl AlarmHarness {
    async fn new() -> Self {
        Self::build(AlarmFixture::new(bmc_platform::Product::Bmc100).await).await
    }

    async fn with_unwritable_crontab() -> Self {
        Self::build(AlarmFixture::with_unwritable_crontab(bmc_platform::Product::Bmc100).await)
            .await
    }

    async fn build(fixture: AlarmFixture) -> Self {
        let controller = fixture.supported_controller().await;

        Self {
            fixture,
            controller,
        }
    }

    /// Schedule an enabled alarm at `hour:minute`.
    async fn add(&self, hour: u32, minute: u32) -> AlarmData {
        self.add_with_sound(hour, minute, None).await
    }

    /// Like `add`, with a sound. The harness has no sound files and no madplay,
    /// so the sound task fails at once and sits in `play_until_cancelled`'s retry sleep,
    /// which a dismiss or snooze has to wait out before it can join the task.
    async fn add_with_sound(&self, hour: u32, minute: u32, sound: Option<Sounds>) -> AlarmData {
        let alarm = AlarmData::new(
            true,
            "regression".to_owned(),
            NaiveTime::from_hms_opt(hour, minute, 0).expect("BUG: valid test time"),
            BTreeSet::new(),
            sound,
            Some(SnoozeOptions {
                limit: SnoozeLimit::Forever,
                // Long enough that it cannot re-fire mid-test.
                duration: SnoozeDuration::ThirtyMinutes,
            }),
        );
        self.controller
            .add_alarm(alarm.clone())
            .await
            .expect("BUG: add_alarm must succeed");
        alarm
    }

    /// Ring a scheduled alarm the way its cron job does, returning once it
    /// holds the current-alarm slot.
    async fn ring(&self, alarm: &AlarmData) {
        self.controller
            .scheduler
            .alarm_sender
            .send(alarm.clone().into())
            .await
            .expect("BUG: the alarm handler must be running");
        self.settle("the alarm to ring", || async { self.ringing().await })
            .await;
    }

    /// Snooze the ringing alarm, returning once the snooze is registered.
    async fn snooze(&self, id: &AlarmId) {
        self.fixture.bus.snooze();
        self.settle("the snooze to register", || async {
            self.controller
                .scheduler
                .pending_snoozes
                .lock()
                .await
                .contains_key(id)
        })
        .await;
    }

    /// Reads `pending_snoozes` under a timeout:
    /// the deadlock these tests guard against parks the command handler
    /// on that lock for good, so a bare read would hang the run
    /// instead of failing it.
    async fn snooze_pending(&self) -> bool {
        tokio::time::timeout(SETTLE_TIMEOUT, async {
            !self
                .controller
                .scheduler
                .pending_snoozes
                .lock()
                .await
                .is_empty()
        })
        .await
        .expect("BUG: reading the pending snoozes must not hang")
    }

    async fn ringing(&self) -> bool {
        self.controller
            .scheduler
            .current_alarm
            .lock()
            .await
            .is_some()
    }

    /// Drain `events` up to and including `Started` for `id`, in arrival order.
    async fn events_until_started(
        &self,
        events: &mut broadcast::Receiver<AlarmEvent>,
        id: &AlarmId,
    ) -> Vec<AlarmEvent> {
        let mut seen = Vec::new();
        let announced = tokio::time::timeout(SETTLE_TIMEOUT, async {
            loop {
                let event = events
                    .recv()
                    .await
                    .expect("BUG: the alarm bus must outlive the test");
                let is_started = matches!(&event, AlarmEvent::Started { alarm } if alarm.id == *id);
                seen.push(event);
                if is_started {
                    break;
                }
            }
        })
        .await;
        assert!(announced.is_ok(), "timed out waiting for Started of {id}");
        seen
    }

    async fn settle<F, Fut>(&self, what: &str, condition: F)
    where
        F: Fn() -> Fut,
        Fut: Future<Output = bool>,
    {
        self.settle_within(SETTLE_TIMEOUT, what, condition).await;
    }

    async fn settle_within<F, Fut>(&self, timeout: Duration, what: &str, condition: F)
    where
        F: Fn() -> Fut,
        Fut: Future<Output = bool>,
    {
        let settled = tokio::time::timeout(timeout, async {
            while !condition().await {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        })
        .await;
        assert!(settled.is_ok(), "timed out waiting for {what}");
    }
}

/// Regression: the `Stop` arm dropped the alarm's pending snooze
/// and then recomputed the next alarm while still holding `pending_snoozes`.
/// That parked the handler on its own lock for good,
/// leaving nothing able to dismiss or snooze an alarm again.
#[tokio::test(flavor = "multi_thread")]
async fn stop_command_leaves_the_handler_serving() {
    let harness = AlarmHarness::new().await;
    // Both are scheduled before the Stop below: a wedged handler also
    // blocks `add_alarm`, which would hang this test rather than fail it.
    let snoozed = harness.add(7, 30).await;
    let other = harness.add(8, 30).await;

    harness.ring(&snoozed).await;
    harness.snooze(&snoozed.id).await;
    harness.fixture.bus.stop_alarm(&snoozed.id);

    harness.ring(&other).await;
    harness.fixture.bus.stop_current();
    harness
        .settle("the handler to dismiss the ringing alarm", || async {
            !harness.ringing().await
        })
        .await;
}

/// A dismiss must send `Stopped` before it joins the sound task:
/// sent after the join, an alarm firing meanwhile announces `Started` first
/// and every listener takes the late `Stopped` as its own.
/// The merge base kept the order only by holding the guard across the join,
/// which is the timeout deadlock; the guard may go, the order may not.
#[tokio::test(flavor = "multi_thread")]
async fn stopped_is_announced_before_the_next_started() {
    let harness = AlarmHarness::new().await;
    let dismissed = harness
        .add_with_sound(7, 30, Some(Sounds::Confirmation))
        .await;
    let next = harness.add(8, 30).await;
    let mut events = harness.fixture.bus.subscribe_events();

    harness.ring(&dismissed).await;
    harness.fixture.bus.stop_current();
    harness
        .settle("the dismissed alarm to leave the slot", || async {
            !harness.ringing().await
        })
        .await;
    harness.ring(&next).await;

    let seen = harness.events_until_started(&mut events, &next.id).await;
    assert!(
        seen.iter()
            .any(|event| matches!(event, AlarmEvent::Stopped { id } if *id == dismissed.id)),
        "Stopped for the dismissed alarm must precede Started for the next one, saw {seen:?}"
    );
}

/// Regression: a Snooze queued ahead of the `Stop` that `remove()` sends
/// registers its pending entry only once the ringing alarm is joined,
/// so `remove()`'s own drop can run first and find nothing.
/// The `Stop` arm runs after that Snooze and is the drop that sees the entry.
#[tokio::test(flavor = "multi_thread")]
async fn stop_drops_a_snooze_registered_after_removal() {
    let harness = AlarmHarness::new().await;
    let removed = harness
        .add_with_sound(7, 30, Some(Sounds::Confirmation))
        .await;
    let other = harness.add(8, 30).await;

    harness.ring(&removed).await;
    harness.fixture.bus.snooze();
    harness
        .settle("the snooze to take the slot", || async {
            !harness.ringing().await
        })
        .await;
    harness
        .controller
        .remove_alarm(removed.id.clone())
        .await
        .expect("BUG: remove_alarm must succeed");

    // Queued behind the `Stop`, so once this snooze is registered the `Stop` has run.
    harness.ring(&other).await;
    harness.fixture.bus.snooze();
    harness
        .settle_within(
            crate::sound::SLEEP_DURATION + SETTLE_TIMEOUT,
            "the other alarm's snooze to register",
            || async {
                harness
                    .controller
                    .scheduler
                    .pending_snoozes
                    .lock()
                    .await
                    .contains_key(&other.id)
            },
        )
        .await;

    assert!(
        !harness
            .controller
            .scheduler
            .pending_snoozes
            .lock()
            .await
            .contains_key(&removed.id),
        "a snooze registered after its alarm was removed must be dropped by the Stop"
    );
}

/// Regression: a snooze emitted `Snoozed` only after joining the sound task,
/// so an alarm firing during that join announced `Started` first
/// and every listener took the late `Snoozed` as its own.
#[tokio::test(flavor = "multi_thread")]
async fn snoozed_is_announced_before_the_next_started() {
    let harness = AlarmHarness::new().await;
    let snoozed = harness
        .add_with_sound(7, 30, Some(Sounds::Confirmation))
        .await;
    let next = harness.add(8, 30).await;
    let mut events = harness.fixture.bus.subscribe_events();

    harness.ring(&snoozed).await;
    harness.fixture.bus.snooze();
    harness
        .settle("the snoozed alarm to leave the slot", || async {
            !harness.ringing().await
        })
        .await;
    harness.ring(&next).await;

    let seen = harness.events_until_started(&mut events, &next.id).await;
    assert!(
        seen.iter()
            .any(|event| matches!(event, AlarmEvent::Snoozed)),
        "Snoozed must precede Started for the next alarm, saw {seen:?}"
    );
}

/// Regression: deleting a snoozed alarm hung, so the alarm
/// stayed in the config and on the widget.
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_snoozed_alarm_leaves_nothing_behind() {
    let harness = AlarmHarness::new().await;
    let alarm = harness.add(7, 30).await;
    harness.ring(&alarm).await;
    harness.snooze(&alarm.id).await;
    let mut next_alarm = harness.controller.subscribe_next_alarm();

    let removed = tokio::time::timeout(SETTLE_TIMEOUT, harness.controller.remove_alarm(alarm.id))
        .await
        .expect("BUG: remove_alarm must not hang");
    removed.expect("BUG: remove_alarm must succeed");

    assert!(
        harness
            .fixture
            .config_handle
            .read()
            .await
            .alarms()
            .is_empty(),
        "a removed alarm must not survive in the config"
    );
    assert!(
        !harness.snooze_pending().await,
        "a removed alarm must not leave a snooze able to re-fire it"
    );
    assert!(
        next_alarm.borrow_and_update().is_none(),
        "a removed alarm must not stay on the widget as the next alarm"
    );
}

/// A scheduler cancel that fails still leaves the alarm out of `active_alarms`,
/// so its snooze could only show on the widget and then find nothing to ring.
/// `remove` reports the failure but cleans up first.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_cancel_still_drops_the_snooze() {
    let harness = AlarmHarness::with_unwritable_crontab().await;
    let alarm = harness.add(7, 30).await;
    harness.ring(&alarm).await;
    harness.snooze(&alarm.id).await;
    let mut next_alarm = harness.controller.subscribe_next_alarm();

    let removed = tokio::time::timeout(SETTLE_TIMEOUT, harness.controller.remove_alarm(alarm.id))
        .await
        .expect("BUG: remove_alarm must not hang");
    assert!(
        matches!(removed, Err(AlarmError::RemoveAlarm)),
        "an unwritable crontab must surface as a removal failure, got {removed:?}"
    );

    assert!(
        !harness.snooze_pending().await,
        "a failed cancel must still drop the snooze"
    );
    assert!(
        next_alarm.borrow_and_update().is_none(),
        "a failed cancel must still clear the widget's next alarm"
    );
}
