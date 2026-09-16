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

use std::{
    collections::{BTreeSet, HashMap},
    fmt::{Display, Formatter},
    ops::{Deref, DerefMut},
    pin::Pin,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::anyhow;
use bmc_scheduler::{
    Cron, JobScheduler,
    scheduler::{JobConfig, Schedule, Task},
};
use bmc_shared_time::time::Timezone;
use bmc_shared_time::time::WeekDay;
use bmc_widget_protocol::NextAlarm;
use chrono::{NaiveTime, Timelike};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    sync::{
        Mutex, RwLock,
        broadcast::{self},
        mpsc,
    },
    task::JoinHandle,
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    config::ConfigHandle,
    sound::{SoundController, Sounds},
};

const CHANNEL_CAPACITY: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct AlarmId(String);

impl AlarmId {
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Display for AlarmId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AlarmId {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            Err(anyhow!("Empty string"))
        } else {
            Ok(Self(value.to_owned()))
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct AlarmData {
    pub(crate) id: AlarmId,
    pub(crate) enabled: bool,
    pub(crate) name: String,
    pub(crate) time: NaiveTime,
    /// Ordered, so writing the config back yields the same bytes.
    /// A hash set's order varies per process.
    pub(crate) repeat: BTreeSet<WeekDay>,
    pub(crate) sound: Option<Sounds>,
    pub(crate) snooze_options: Option<SnoozeOptions>,
}

impl AlarmData {
    pub(crate) fn new(
        enabled: bool,
        name: String,
        time: NaiveTime,
        repeat: BTreeSet<WeekDay>,
        sound: Option<Sounds>,
        snooze_options: Option<SnoozeOptions>,
    ) -> Self {
        Self {
            id: AlarmId::generate(),
            enabled,
            name,
            time,
            repeat,
            sound,
            snooze_options,
        }
    }

    pub(crate) fn new_with_id(
        id: AlarmId,
        enabled: bool,
        name: String,
        time: NaiveTime,
        repeat: BTreeSet<WeekDay>,
        sound: Option<Sounds>,
        snooze_options: Option<SnoozeOptions>,
    ) -> Self {
        Self {
            id,
            enabled,
            name,
            time,
            repeat,
            sound,
            snooze_options,
        }
    }

    fn weekdays_to_number_string(set: &BTreeSet<WeekDay>) -> String {
        set.iter()
            .copied()
            .map(WeekDay::as_number_string)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn cron(&self) -> anyhow::Result<Cron> {
        let minute = self.time.minute().to_string();
        let hour = self.time.hour().to_string();

        let days = if self.repeat.is_empty() {
            "*".to_owned()
        } else {
            Self::weekdays_to_number_string(&self.repeat)
        };
        Ok(Cron::from_str(&format!("0 {minute} {hour} * * {days}"))?)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct SnoozeOptions {
    pub(crate) limit: SnoozeLimit,
    pub(crate) duration: SnoozeDuration,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) enum SnoozeLimit {
    Forever,
    Three,
    Five,
}

impl SnoozeLimit {
    pub(crate) fn limit(&self) -> Option<u32> {
        match self {
            SnoozeLimit::Forever => None,
            SnoozeLimit::Three => Some(3),
            SnoozeLimit::Five => Some(5),
        }
    }
}

#[expect(clippy::enum_variant_names)]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) enum SnoozeDuration {
    FiveMinutes,
    TenMinutes,
    FifteenMinutes,
    ThirtyMinutes,
}

impl SnoozeDuration {
    fn duration(&self) -> Duration {
        match self {
            SnoozeDuration::FiveMinutes => Duration::from_mins(5),
            SnoozeDuration::TenMinutes => Duration::from_mins(10),
            SnoozeDuration::FifteenMinutes => Duration::from_mins(15),
            SnoozeDuration::ThirtyMinutes => Duration::from_mins(30),
        }
    }
}

#[derive(Clone, Debug)]
pub enum AlarmCmd {
    StopCurrent,
    Stop { id: AlarmId },
    Snooze,
}

#[derive(Clone, Debug)]
pub enum AlarmEvent {
    Stopped { id: AlarmId },
    Snoozed,
    Started { alarm: ActiveAlarm },
}

#[derive(Clone, Debug)]
pub struct AlarmBus {
    tx_commands: broadcast::Sender<AlarmCmd>,
    tx_events: broadcast::Sender<AlarmEvent>,
}

impl AlarmBus {
    pub fn new() -> Self {
        let (tx_commands, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_events, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            tx_commands,
            tx_events,
        }
    }

    pub fn stop_current(&self) {
        if let Err(err) = self.tx_commands.send(AlarmCmd::StopCurrent) {
            warn!(error = %err, "Failed to send StopCurrent command, no active receivers");
        }
    }

    /// Dismiss `id` if it is the alarm currently ringing.
    pub fn stop_alarm(&self, id: &AlarmId) {
        if let Err(err) = self.tx_commands.send(AlarmCmd::Stop { id: id.clone() }) {
            warn!(alarm_id = %id, error = %err, "Failed to send Stop command, no active receivers");
        }
    }

    pub fn snooze(&self) {
        if let Err(err) = self.tx_commands.send(AlarmCmd::Snooze) {
            warn!(error = %err, "Failed to send Snooze command, no active receivers");
        }
    }

    pub fn send_event(&self, event: AlarmEvent) {
        if let Err(err) = self.tx_events.send(event) {
            warn!(error = %err, "Failed to send alarm event, no active receivers");
        }
    }

    pub fn subscribe_commands(&self) -> broadcast::Receiver<AlarmCmd> {
        self.tx_commands.subscribe()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<AlarmEvent> {
        self.tx_events.subscribe()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ActiveAlarm {
    pub(crate) data: AlarmData,
    pub(crate) snooze_count: u32,
}

impl Deref for ActiveAlarm {
    type Target = AlarmData;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl DerefMut for ActiveAlarm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl AsRef<AlarmData> for ActiveAlarm {
    fn as_ref(&self) -> &AlarmData {
        self
    }
}

impl AsMut<AlarmData> for ActiveAlarm {
    fn as_mut(&mut self) -> &mut AlarmData {
        self
    }
}

impl From<AlarmData> for ActiveAlarm {
    fn from(value: AlarmData) -> Self {
        Self {
            data: value,
            snooze_count: 0,
        }
    }
}

impl ActiveAlarm {
    /// Whether this firing may still be snoozed: snooze is configured,
    /// and the per-firing count is below the limit (`Forever` has none).
    pub(crate) fn snooze_allowed(&self) -> bool {
        self.snooze_options
            .as_ref()
            .is_some_and(|opts| opts.limit.limit().is_none_or(|max| self.snooze_count < max))
    }
}

#[derive(Debug)]
pub(crate) struct CurrentRunningAlarm {
    pub(crate) alarm: ActiveAlarm,
    pub(crate) cancellation_token: CancellationToken,
    pub(crate) sound_join_handle: Option<JoinHandle<()>>,
    pub(crate) timeout_join_handle: JoinHandle<()>,
}

impl CurrentRunningAlarm {
    fn new(
        alarm: ActiveAlarm,
        cancellation_token: CancellationToken,
        sound_join_handle: Option<JoinHandle<()>>,
        timeout_join_handle: JoinHandle<()>,
    ) -> Self {
        Self {
            alarm,
            cancellation_token,
            sound_join_handle,
            timeout_join_handle,
        }
    }

    async fn cancel(self) {
        self.cancellation_token.cancel();
        if let Some(handle) = self.sound_join_handle
            && let Err(err) = handle.await
        {
            warn!(alarm_id = %self.alarm.id, error = %err, "Sound task panicked during cancellation");
        }
        if let Err(err) = self.timeout_join_handle.await {
            warn!(alarm_id = %self.alarm.id, error = %err, "Timeout task panicked during cancellation");
        }
    }
}

/// Per-scheduled-alarm bookkeeping: cron job id + display name.
#[derive(Clone, Debug)]
struct ScheduledAlarm {
    job_id: Uuid,
    name: String,
}

/// Outstanding snooze waiting to re-fire. `cancel`
/// halts the spawned sleep task on stop / removal
/// so the alarm doesn't re-fire after the user stops it.
#[derive(Debug)]
struct PendingSnooze {
    cancel: CancellationToken,
    fire_at_utc_ms: i64,
    name: String,
}

#[derive(Clone, Debug)]
struct AlarmScheduler {
    scheduler: JobScheduler,
    active_alarms: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>>,
    alarm_bus: AlarmBus,
    timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
    alarm_sender: mpsc::Sender<ActiveAlarm>,
    current_alarm: Arc<Mutex<Option<CurrentRunningAlarm>>>,
    sound_controller: SoundController,
    next_alarm_sender: tokio::sync::watch::Sender<Option<NextAlarm>>,
    /// Snoozes that haven't fired yet, keyed by alarm id.
    pending_snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>>,
}

impl AlarmScheduler {
    const SCHEDULER_SOURCE: &str = "Alarm";
    const ALARM_TIMEOUT: Duration = Duration::from_mins(10);

    fn init(
        scheduler: JobScheduler,
        alarm_bus: AlarmBus,
        timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
        sound_controller: SoundController,
    ) -> Self {
        let (alarm_sender, alarm_receiver) = mpsc::channel(1);
        let (next_alarm_sender, _) = tokio::sync::watch::channel(None);

        let this = Self {
            scheduler,
            active_alarms: Arc::default(),
            alarm_bus,
            timezone_receiver,
            alarm_sender,
            current_alarm: Arc::default(),
            sound_controller,
            next_alarm_sender,
            pending_snoozes: Arc::default(),
        };

        this.spawn_timezone_listener();
        this.spawn_alarm_handle(alarm_receiver);
        this.spawn_command_handle();

        this
    }

    fn spawn_timezone_listener(&self) {
        tokio::spawn({
            let mut self_ = self.clone();
            async move {
                while let Ok(()) = self_.timezone_receiver.changed().await {
                    self_.recompute_next_alarm_time().await;
                }
            }
        });
    }

    fn spawn_alarm_handle(&self, mut alarm_receiver: mpsc::Receiver<ActiveAlarm>) {
        tokio::spawn({
            let self_ = self.clone();
            async move {
                while let Some(active_alarm) = alarm_receiver.recv().await {
                    let alarm_id = &active_alarm.id;
                    let alarm_name = &active_alarm.name;
                    let alarm_time = active_alarm.time;
                    let snooze_count = active_alarm.snooze_count;
                    info!(
                        alarm_id = %alarm_id,
                        alarm_name = %alarm_name,
                        alarm_time = %alarm_time,
                        snooze_count = snooze_count,
                        "Starting alarm"
                    );

                    // A firing already queued here outruns a removal or disable:
                    // a snooze re-fire clears its pending entry before sending,
                    // so cancelling the snooze no longer stops it.
                    if !self_
                        .active_alarms
                        .lock()
                        .await
                        .contains_key(&active_alarm.id)
                    {
                        continue;
                    }

                    self_.cancel_current_alarm().await;

                    let token = CancellationToken::new();

                    // NOTE: Spawn task with playing alarm sound.
                    // Reason why this task isn't in the tokio::select! section below is
                    // that bmc-audio crate uses tokio::select! internally for playing sound
                    // and cancelling of the process. Having this part in tokio::select!
                    // could lead to errors when cancelling sound playing.
                    let sound_join_handle = if let Some(sound) = active_alarm.data.sound.clone() {
                        Some(tokio::spawn({
                            let sound_controller = self_.sound_controller.clone();
                            let token = token.clone();

                            async move { sound_controller.play_until_cancelled(sound, token).await }
                        }))
                    } else {
                        None
                    };

                    // NOTE: Spawn timeout for the alarm
                    let timeout_join_handle = tokio::spawn({
                        let alarm_id = active_alarm.id.clone();
                        let alarm_bus = self_.alarm_bus.clone();
                        let token = token.clone();
                        let current_alarm = self_.current_alarm.clone();

                        async move {
                            let deadline =
                                tokio::time::sleep_until(Instant::now() + Self::ALARM_TIMEOUT);
                            tokio::pin!(deadline);

                            tokio::select! {
                                () = token.cancelled() => {
                                    info!(alarm_id = %alarm_id, "Alarm cancelled before timeout");
                                }
                                () = &mut deadline => {
                                    info!(alarm_id = %alarm_id, timeout_minutes = 10, "Alarm timed out, auto-stopping");
                                    token.cancel();
                                    // A dismiss landing this same instant may already hold the alarm;
                                    // then it owns the `Stopped`, and this branch has nothing to announce.
                                    let mut slot = current_alarm.lock().await;
                                    if slot.take_if(|c| c.alarm.id == alarm_id).is_some() {
                                        alarm_bus.send_event(AlarmEvent::Stopped { id: alarm_id });
                                    }
                                }
                            }
                        }
                    });

                    // Store the slot before announcing the alarm, and emit
                    // while still holding the lock: a dismiss/snooze racing in
                    // after `Started` must find the slot populated (or it
                    // silently no-ops), and must not be able to take it before
                    // the emit (listeners would see Stopped before Started).
                    {
                        let mut current_alarm_guard = self_.current_alarm.lock().await;
                        *current_alarm_guard = Some(CurrentRunningAlarm::new(
                            active_alarm.clone(),
                            token,
                            sound_join_handle,
                            timeout_join_handle,
                        ));
                        self_.alarm_bus.send_event(AlarmEvent::Started {
                            alarm: active_alarm,
                        });
                    }

                    self_.recompute_next_alarm_time().await;
                }
            }
        });
    }

    fn spawn_command_handle(&self) {
        tokio::spawn({
            let self_ = self.clone();
            let mut rx = self_.alarm_bus.subscribe_commands();

            async move {
                loop {
                    let cmd = match rx.recv().await {
                        Ok(cmd) => cmd,
                        // Lagging only drops stale commands; the handler must
                        // survive, or nothing can ever stop/snooze an alarm
                        // again (a burst of taps overflows the 8-slot buffer).
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(skipped = n, "alarm command receiver lagged");
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    };
                    match cmd {
                        AlarmCmd::StopCurrent => {
                            info!("Received StopCurrent command");
                            self_.cancel_current_alarm().await;
                        }
                        AlarmCmd::Stop { id } => {
                            info!(alarm_id = %id, "Received Stop command");
                            // `remove()` drops the pending snooze as well, but it may run too early:
                            // a Snooze queued before this command registers its entry only after the join.
                            // This arm runs after that Snooze, so it is the drop that sees the entry.
                            let pending = self_.pending_snoozes.lock().await.remove(&id);
                            if let Some(pending) = pending {
                                pending.cancel.cancel();
                                self_.recompute_next_alarm_time().await;
                            }
                            self_
                                .stop_current_if(|current| current.alarm.id == id)
                                .await;
                        }
                        AlarmCmd::Snooze => self_.handle_snooze_command().await,
                    }
                }
            }
        });
    }

    /// If snooze limit is not reached yet, stop the running alarm
    /// and register a cancellable pending snooze that re-fires through
    /// `alarm_sender`. Re-snoozing the same alarm cancels the prior pending entry.
    async fn handle_snooze_command(&self) {
        info!("Received Snooze command");
        let active_alarm = {
            let mut current_alarm = self.current_alarm.lock().await;
            let Some(active_alarm) = current_alarm.as_ref() else {
                return;
            };

            // Snooze must be both enabled and under its limit. Checking here --
            // before anything is taken, cancelled, or a `Snoozed` event is sent
            // -- leaves the alarm ringing (only "Stop" applies) when snooze is
            // not on offer, so a stale or spurious snooze request cannot silence
            // a no-snooze alarm with nothing left to re-fire.
            if !active_alarm.alarm.snooze_allowed() {
                return;
            }

            let active_alarm = current_alarm
                .take()
                .expect("BUG: alarm present, checked above");
            // Under the guard for the same reason `stop_current_if` sends `Stopped` there.
            self.alarm_bus.send_event(AlarmEvent::Snoozed);
            active_alarm
        };

        let mut alarm = active_alarm.alarm.clone();
        active_alarm.cancel().await;

        let snooze = alarm
            .snooze_options
            .as_ref()
            .expect("BUG: snooze_allowed guarantees snooze_options is Some");
        let duration = snooze.duration.duration();
        let fire_at_utc_ms = chrono::Utc::now().timestamp_millis().saturating_add(
            i64::try_from(duration.as_millis()).expect("BUG: snooze duration must fit in i64 ms"),
        );
        let cancel = CancellationToken::new();
        let alarm_id = alarm.id.clone();
        alarm.snooze_count += 1;
        let snooze_count = alarm.snooze_count;

        let stale = self.pending_snoozes.lock().await.insert(
            alarm_id.clone(),
            PendingSnooze {
                cancel: cancel.clone(),
                fire_at_utc_ms,
                name: alarm.name.clone(),
            },
        );
        if let Some(stale) = stale {
            stale.cancel.cancel();
        }
        self.recompute_next_alarm_time().await;

        let self_ = self.clone();
        let alarm_id_log = alarm_id.clone();
        tokio::spawn(async move {
            info!(
                alarm_id = %alarm_id_log,
                snooze_count = snooze_count,
                snooze_duration_secs = duration.as_secs(),
                "Alarm snoozed, will retrigger after duration"
            );
            tokio::select! {
                () = tokio::time::sleep(duration) => {
                    self_.pending_snoozes.lock().await.remove(&alarm_id);
                    self_.recompute_next_alarm_time().await;
                    if let Err(err) = self_.alarm_sender.send(alarm).await {
                        warn!(error = %err, "Failed to trigger snoozed alarm");
                    }
                }
                () = cancel.cancelled() => {
                    info!(
                        alarm_id = %alarm_id_log,
                        "Snoozed alarm cancelled before retrigger"
                    );
                }
            }
        });
    }

    async fn cancel_current_alarm(&self) {
        if let Some(alarm) = self.stop_current_if(|_| true).await {
            info!(alarm_id = %alarm.id, alarm_name = %alarm.name, "Cancelled active alarm");
        }
    }

    /// Stop the ringing alarm if `matches` accepts it.
    ///
    /// `Stopped` is sent under the guard that empties the slot,
    /// `Started` under the one that fills it, so the lock orders them:
    /// a firing that lands mid-stop never announces itself first.
    /// Listeners match `Stopped` without the id,
    /// so the reverse order would dismiss the new alarm while it rings.
    /// The joins run with the guard gone: the timeout task's deadline branch takes this lock.
    async fn stop_current_if(
        &self,
        matches: impl FnOnce(&CurrentRunningAlarm) -> bool,
    ) -> Option<ActiveAlarm> {
        let stopped = {
            let mut slot = self.current_alarm.lock().await;
            let stopped = slot.take_if(|current| matches(current));
            if let Some(stopped) = &stopped {
                self.alarm_bus.send_event(AlarmEvent::Stopped {
                    id: stopped.alarm.id.clone(),
                });
            }
            stopped
        }?;

        let alarm = stopped.alarm.clone();
        stopped.cancel().await;
        Some(alarm)
    }

    async fn schedule(&self, alarm_data: AlarmData) -> anyhow::Result<()> {
        let cron = alarm_data.cron()?;
        let sender = self.alarm_sender.clone();

        let task = {
            let sender = sender.clone();
            let alarm_data = alarm_data.clone();
            move || {
                let sender = sender.clone();
                let alarm_data = alarm_data.clone();
                Box::pin(async move {
                    if let Err(err) = sender.send(alarm_data.into()).await {
                        warn!(error = %err, "Failed to trigger scheduled alarm");
                    }
                }) as Pin<Box<dyn Future<Output = ()> + Send>>
            }
        };

        let schedule_id = self
            .scheduler
            .schedule(
                Schedule::Cron(cron),
                Task::Async(Box::new(task)),
                JobConfig::new(Self::SCHEDULER_SOURCE),
            )
            .await?;

        self.active_alarms.lock().await.insert(
            alarm_data.id.clone(),
            ScheduledAlarm {
                job_id: schedule_id,
                name: alarm_data.name.clone(),
            },
        );

        info!(
            alarm_id = %alarm_data.id,
            alarm_name = %alarm_data.name,
            alarm_time = %alarm_data.time,
            "Alarm scheduled successfully"
        );

        self.recompute_next_alarm_time().await;

        Ok(())
    }

    async fn remove(&self, id: &AlarmId) -> anyhow::Result<()> {
        // Guard dropped first: the cancel below touches the crontab,
        // and every firing takes this lock.
        let scheduled = self.active_alarms.lock().await.remove(id);
        let cancel_result = if let Some(scheduled) = scheduled {
            self.alarm_bus.stop_alarm(id);

            self.scheduler
                .cancel(&scheduled.job_id)
                .await
                .map_err(|e| anyhow!("Failed to remove scheduled alarm, err: {e}"))
        } else {
            Ok(())
        };

        // A surviving snooze would show the removed alarm on the widget as next;
        // its re-fire is caught by the `active_alarms` check on every firing.
        // Runs even on a failed cancel: the alarm has already left `active_alarms`.
        if let Some(pending) = self.pending_snoozes.lock().await.remove(id) {
            pending.cancel.cancel();
        }

        self.recompute_next_alarm_time().await;
        cancel_result
    }

    async fn recompute_next_alarm_time(&self) {
        recompute_and_broadcast_next_alarm(
            &self.scheduler,
            Self::SCHEDULER_SOURCE,
            &self.active_alarms,
            &self.pending_snoozes,
            &self.next_alarm_sender,
        )
        .await;
    }
}

/// Resolve the soonest pending alarm across the scheduler
/// and any in-flight snoozes, then broadcast it on `next_alarm_sender`.
///
/// Extracted from `AlarmScheduler::recompute_next_alarm_time`
/// so the behavior is unit-testable without standing up
/// the full controller.
async fn recompute_and_broadcast_next_alarm(
    scheduler: &JobScheduler,
    scheduler_source: &str,
    active_alarms: &Mutex<HashMap<AlarmId, ScheduledAlarm>>,
    pending_snoozes: &Mutex<HashMap<AlarmId, PendingSnooze>>,
    next_alarm_sender: &tokio::sync::watch::Sender<Option<NextAlarm>>,
) {
    // Scheduler-side lookup failures (transport error, empty list, or race window
    // where `jobs_by_source` reports a job_id that `active_alarms` no longer holds)
    // all collapse to "no scheduler candidate". The snooze merge below still runs,
    // so a pending snooze is never masked by a scheduler-side miss.
    let scheduler_next = match scheduler.jobs_by_source(scheduler_source).await {
        Err(e) => {
            error!(?e, "Failed to get scheduled alarms from scheduler");
            None
        }
        Ok(alarms) => {
            let next = alarms
                .into_iter()
                .filter_map(|alarm| alarm.next_tick.map(|t| (t, alarm.job_id)))
                .sorted_by_key(|(t, _)| *t)
                .next();
            if let Some((next_tick, job_id)) = next {
                // Linear scan over the small configured alarm set.
                let name = active_alarms
                    .lock()
                    .await
                    .values()
                    .find(|s| s.job_id == job_id)
                    .map(|s| s.name.clone());
                if let Some(name) = name {
                    Some(NextAlarm {
                        fire_at_utc_ms: next_tick.timestamp_millis(),
                        name,
                    })
                } else {
                    warn!(
                        ?job_id,
                        "Scheduler reports alarm not present in active_alarms; \
                         dropping scheduler-side candidate and broadcasting any snooze",
                    );
                    None
                }
            } else {
                None
            }
        }
    };

    // Snoozes are off-scheduler;
    // merge them in so the broadcast reflects
    // whichever firing is actually soonest.
    let snooze_candidates: Vec<NextAlarm> = pending_snoozes
        .lock()
        .await
        .values()
        .map(|s| NextAlarm {
            fire_at_utc_ms: s.fire_at_utc_ms,
            name: s.name.clone(),
        })
        .collect();
    let next_alarm = pick_soonest_next_alarm(scheduler_next, snooze_candidates);

    info!(next_alarm = ?next_alarm, "Recomputed next alarm time");
    // Fired on every scheduler tick, even on no changes to the next alarm.
    next_alarm_sender.send_if_modified(|current| {
        if *current == next_alarm {
            false
        } else {
            *current = next_alarm;
            true
        }
    });
}

/// Soonest firing across scheduler-derived candidate + pending
/// snoozes. Pure for testability; ties broken by `min_by_key` order.
fn pick_soonest_next_alarm(
    scheduler_next: Option<NextAlarm>,
    pending_snoozes: Vec<NextAlarm>,
) -> Option<NextAlarm> {
    pending_snoozes
        .into_iter()
        .chain(scheduler_next)
        .min_by_key(|n| n.fire_at_utc_ms)
}

#[derive(Debug)]
pub(crate) enum AlarmBackend {
    Supported(AlarmController),
    Unsupported {
        // Held only to keep the channel open; closing it signals shutdown.
        next_alarm_sender: tokio::sync::watch::Sender<Option<NextAlarm>>,
    },
}

impl AlarmBackend {
    pub(crate) async fn init(
        alarm_supported: bool,
        config_handle: Arc<RwLock<ConfigHandle>>,
        scheduler: JobScheduler,
        sound_controller: SoundController,
        alarm_bus: AlarmBus,
        timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
    ) -> Self {
        if alarm_supported {
            Self::Supported(
                AlarmController::init(
                    config_handle,
                    scheduler,
                    sound_controller,
                    alarm_bus,
                    timezone_receiver,
                )
                .await,
            )
        } else {
            let (next_alarm_sender, _) = tokio::sync::watch::channel(None);
            Self::Unsupported { next_alarm_sender }
        }
    }

    pub(crate) fn controller(&self) -> Option<AlarmController> {
        match self {
            Self::Supported(controller) => Some(controller.clone()),
            Self::Unsupported { .. } => None,
        }
    }

    pub(crate) fn subscribe_next_alarm(&self) -> tokio::sync::watch::Receiver<Option<NextAlarm>> {
        match self {
            Self::Supported(controller) => controller.subscribe_next_alarm(),
            Self::Unsupported { next_alarm_sender } => next_alarm_sender.subscribe(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AlarmController {
    config_handle: Arc<RwLock<ConfigHandle>>,
    scheduler: AlarmScheduler,
}

impl AlarmController {
    pub(crate) async fn init(
        config_handle: Arc<RwLock<ConfigHandle>>,
        scheduler: JobScheduler,
        sound_controller: SoundController,
        alarm_bus: AlarmBus,
        timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
    ) -> Self {
        let scheduler = AlarmScheduler::init(
            scheduler,
            alarm_bus.clone(),
            timezone_receiver,
            sound_controller,
        );

        for alarm_data in config_handle
            .clone()
            .read()
            .await
            .alarms()
            .into_iter()
            .filter(|alarm| alarm.enabled)
        {
            let alarm_id = alarm_data.id.clone();
            if let Err(err) = scheduler.schedule(alarm_data).await {
                error!(%alarm_id, ?err, "Failed to schedule alarm job");
            }
        }

        let controller = Self {
            config_handle,
            scheduler,
        };

        let self_ = controller.clone();

        tokio::spawn(async move {
            let mut rx = alarm_bus.subscribe_events();

            loop {
                let event = match rx.recv().await {
                    Ok(event) => event,
                    // Skipping stale events must not kill the subscriber; only
                    // a closed bus ends it.
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "alarm event receiver lagged");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if let AlarmEvent::Stopped { id } = event {
                    // NOTE: Alarm that is not repeatable needs to be deactivated after it was triggered
                    if let Some(alarm) = self_.alarms().await.iter().find(|alarm| alarm.id == id)
                        && alarm.repeat.is_empty()
                        && let Err(err) = self_.set_enabled(id.clone(), false).await
                    {
                        warn!(alarm_id = %id, error = ?err, "Failed to disable non-repeating alarm after it was triggered");
                    }
                }
            }
        });

        controller
    }

    /// Subscribe to the soonest-to-fire alarm watch channel.
    pub(crate) fn subscribe_next_alarm(&self) -> tokio::sync::watch::Receiver<Option<NextAlarm>> {
        self.scheduler.next_alarm_sender.subscribe()
    }

    pub(crate) async fn alarms(&self) -> Vec<AlarmData> {
        self.config_handle.read().await.alarms()
    }

    pub(crate) async fn add_alarm(&self, alarm_data: AlarmData) -> Result<(), AlarmError> {
        let mut config = self.config_handle.write().await;
        let mut temp_config = config.clone();

        if let Some(existing) = config
            .alarms()
            .iter()
            .find(|existing| existing.time == alarm_data.time)
        {
            warn!(
                alarm_id = %alarm_data.id,
                time = %alarm_data.time,
                existing_alarm_id = %existing.id,
                "Failed to add alarm: An alarm with the same time already exists"
            );
            return Err(AlarmError::DuplicateAlarm);
        }

        let alarm_id = alarm_data.id.clone();

        if alarm_data.enabled {
            self.scheduler
                .schedule(alarm_data.clone())
                .await
                .map_err(|err| {
                    error!(alarm_id = %alarm_id, error = ?err, "Failed to schedule alarm");
                    AlarmError::ScheduleAlarm
                })?;
        }

        temp_config.add_alarm(alarm_data);

        if let Err(err) = temp_config.save().await {
            error!(alarm_id = %alarm_id, error = ?err, "Failed to save alarm configuration");

            _ = self
                .scheduler
                .remove(&alarm_id)
                .await
                .inspect_err(|err| error!(alarm_id = %alarm_id, error = ?err, "Failed to remove alarm from scheduler during rollback"));

            return Err(AlarmError::SyncToStorage);
        }
        *config = temp_config;

        info!(alarm_id = %alarm_id, "Alarm created successfully");

        Ok(())
    }

    pub(crate) async fn set_alarm(&self, alarm_data: AlarmData) -> Result<(), AlarmError> {
        let mut config = self.config_handle.write().await;
        let mut temp_config = config.clone();

        let alarm_id = alarm_data.id.clone();

        if !config
            .alarms()
            .iter()
            .any(|existing| existing.id == alarm_id)
        {
            warn!(alarm_id = %alarm_id, "Failed to update alarm: Alarm not found");
            return Err(AlarmError::NotFound);
        }

        if let Some(existing) = config
            .alarms()
            .iter()
            .find(|existing| existing.time == alarm_data.time && existing.id != alarm_id)
        {
            warn!(
                alarm_id = %alarm_id,
                time = %alarm_data.time,
                existing_alarm_id = %existing.id,
                "Failed to update alarm: An alarm with the same time already exists"
            );
            return Err(AlarmError::DuplicateAlarm);
        }

        self.scheduler.remove(&alarm_id).await.map_err(|err| {
            warn!(alarm_id = %alarm_id, error = ?err, "Failed to remove alarm from scheduler");
            AlarmError::RemoveAlarm
        })?;

        if alarm_data.enabled {
            self.scheduler
                .schedule(alarm_data.clone())
                .await
                .map_err(|err| {
                    warn!(alarm_id = %alarm_id, error = ?err, "Failed to schedule updated alarm");
                    AlarmError::ScheduleAlarm
                })?;
        }

        temp_config.set_alarm(alarm_data);

        if let Err(err) = temp_config.save().await {
            warn!(alarm_id = %alarm_id, error = ?err, "Failed to save updated alarm configuration");

            _ = self
                .scheduler
                .remove(&alarm_id)
                .await
                .inspect_err(|err| warn!(alarm_id = %alarm_id, error = ?err, "Failed to remove alarm from scheduler during rollback"));

            return Err(AlarmError::SyncToStorage);
        }

        *config = temp_config;

        info!(alarm_id = %alarm_id, "Alarm updated successfully");

        Ok(())
    }

    pub(crate) async fn remove_alarm(&self, alarm_id: AlarmId) -> Result<(), AlarmError> {
        let mut config = self.config_handle.write().await;
        let mut temp_config = config.clone();

        if !config.alarms().iter().any(|alarm| alarm.id == alarm_id) {
            warn!(alarm_id = %alarm_id, "Failed to remove alarm: Alarm not found");
            return Err(AlarmError::NotFound);
        }

        self.scheduler.remove(&alarm_id).await.map_err(|err| {
            warn!(alarm_id = %alarm_id, error = ?err, "Failed to remove alarm from scheduler");
            AlarmError::RemoveAlarm
        })?;

        temp_config.remove_alarm(&alarm_id);

        temp_config.save().await.map_err(|err| {
            warn!(alarm_id = %alarm_id, error = ?err, "Failed to save configuration after removing alarm");
            AlarmError::SyncToStorage
        })?;

        *config = temp_config;

        info!(alarm_id = %alarm_id, "Alarm removed successfully");

        Ok(())
    }

    pub(crate) async fn set_enabled(
        &self,
        alarm_id: AlarmId,
        enabled: bool,
    ) -> Result<(), AlarmError> {
        let mut config = self.config_handle.write().await;
        let mut temp_config = config.clone();

        let mut alarm_data = config
            .alarms()
            .into_iter()
            .find(|alarm| alarm.id == alarm_id)
            .ok_or_else(|| {
                warn!(alarm_id = %alarm_id, "Failed to set alarm enabled state: Alarm not found");
                AlarmError::NotFound
            })?;

        alarm_data.enabled = enabled;

        self.scheduler.remove(&alarm_data.id).await.map_err(|err| {
            warn!(alarm_id = %alarm_id, error = ?err, "Failed to remove alarm from scheduler");
            AlarmError::RemoveAlarm
        })?;

        if alarm_data.enabled {
            self.scheduler
                .schedule(alarm_data.clone())
                .await
                .map_err(|err| {
                    warn!(alarm_id = %alarm_id, error = ?err, "Failed to schedule alarm after enabling");
                    AlarmError::ScheduleAlarm
                })?;
        }

        temp_config.set_alarm(alarm_data);

        if let Err(err) = temp_config.save().await {
            warn!(alarm_id = %alarm_id, error = ?err, enabled = enabled, "Failed to save alarm enabled state");

            _ = self
                .scheduler
                .remove(&alarm_id)
                .await
                .inspect_err(|err| warn!(alarm_id = %alarm_id, error = ?err, "Failed to remove alarm from scheduler during rollback"));

            return Err(AlarmError::SyncToStorage);
        }
        *config = temp_config;

        if enabled {
            info!(alarm_id = %alarm_id, "Alarm enabled successfully");
        } else {
            info!(alarm_id = %alarm_id, "Alarm disabled successfully");
        }

        Ok(())
    }
}

#[derive(Error, Debug)]
pub(crate) enum AlarmError {
    #[error("An alarm with the same time already exists.")]
    DuplicateAlarm,
    #[error("Failed to save configuration.")]
    SyncToStorage,
    #[error("Alarm not found")]
    NotFound,
    #[error("Failed to schedule alarm")]
    ScheduleAlarm,
    #[error("Failed to remove alarm")]
    RemoveAlarm,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BackendFixture {
        _temp: tempfile::TempDir,
        config_path: std::path::PathBuf,
        expected_config: Vec<u8>,
        config_handle: Arc<RwLock<ConfigHandle>>,
        scheduler: JobScheduler,
    }

    async fn backend_fixture() -> BackendFixture {
        let temp = tempfile::tempdir().expect("BUG: create alarm backend test directory");
        let config_path = temp.path().join("bmc-config.json");
        let (mut config_handle, _) = ConfigHandle::init(
            config_path.clone(),
            50,
            50,
            50,
            50,
            bmc_platform::Product::Bmm101,
        )
        .await;
        config_handle.add_alarm(AlarmData::new(
            true,
            "persisted".to_owned(),
            NaiveTime::from_hms_opt(7, 30, 0).expect("BUG: valid alarm test time"),
            BTreeSet::new(),
            None,
            None,
        ));
        config_handle
            .save()
            .await
            .expect("BUG: persist alarm backend test config");
        let expected_config = tokio::fs::read(&config_path)
            .await
            .expect("BUG: read alarm backend test config");
        let config_handle = Arc::new(RwLock::new(config_handle));
        let (_timezone_sender, timezone_receiver) =
            tokio::sync::watch::channel(Timezone::default());
        let scheduler = JobScheduler::init(
            timezone_receiver,
            Some(temp.path().join("scheduler-crontab")),
        )
        .await;

        BackendFixture {
            _temp: temp,
            config_path,
            expected_config,
            config_handle,
            scheduler,
        }
    }

    async fn init_backend(fixture: &BackendFixture, alarm_supported: bool) -> AlarmBackend {
        let (_timezone_sender, timezone_receiver) =
            tokio::sync::watch::channel(Timezone::default());
        AlarmBackend::init(
            alarm_supported,
            fixture.config_handle.clone(),
            fixture.scheduler.clone(),
            SoundController::new(
                fixture.config_handle.clone(),
                fixture
                    .config_path
                    .parent()
                    .expect("BUG: test config has a parent")
                    .join("sounds"),
            ),
            AlarmBus::new(),
            timezone_receiver,
        )
        .await
    }

    #[tokio::test]
    async fn unsupported_backend_preserves_enabled_alarm_without_scheduling_it() {
        let fixture = backend_fixture().await;
        let backend = init_backend(&fixture, false).await;
        let next_alarm_receiver = backend.subscribe_next_alarm();

        assert!(backend.controller().is_none());
        assert!(next_alarm_receiver.borrow().is_none());
        assert!(
            next_alarm_receiver.has_changed().is_ok(),
            "unsupported alarm channel must remain open"
        );
        assert!(
            fixture
                .scheduler
                .jobs_by_source(AlarmScheduler::SCHEDULER_SOURCE)
                .await
                .expect("BUG: list alarm jobs")
                .is_empty()
        );
        let alarms = fixture.config_handle.read().await.alarms();
        assert_eq!(alarms.len(), 1);
        assert!(alarms[0].enabled);
        assert_eq!(
            tokio::fs::read(&fixture.config_path)
                .await
                .expect("BUG: reread alarm backend test config"),
            fixture.expected_config
        );
    }

    #[tokio::test]
    async fn supported_backend_schedules_enabled_persisted_alarm() {
        let fixture = backend_fixture().await;
        let backend = init_backend(&fixture, true).await;

        assert!(backend.controller().is_some());
        assert_eq!(
            fixture
                .scheduler
                .jobs_by_source(AlarmScheduler::SCHEDULER_SOURCE)
                .await
                .expect("BUG: list alarm jobs")
                .len(),
            1
        );
        assert!(backend.subscribe_next_alarm().borrow().is_some());
    }

    fn active_alarm(snooze_options: Option<SnoozeOptions>, snooze_count: u32) -> ActiveAlarm {
        let data = AlarmData::new(
            true,
            "test".to_owned(),
            NaiveTime::from_hms_opt(7, 30, 0).expect("BUG: valid test time"),
            BTreeSet::new(),
            None,
            snooze_options,
        );
        ActiveAlarm { data, snooze_count }
    }

    fn snooze(limit: SnoozeLimit) -> SnoozeOptions {
        SnoozeOptions {
            limit,
            duration: SnoozeDuration::FiveMinutes,
        }
    }

    #[test]
    fn snooze_not_allowed_without_snooze_options() {
        // Regression: a snooze request on a no-snooze alarm must be rejected up
        // front so it is not cancelled with nothing to re-fire.
        assert!(!active_alarm(None, 0).snooze_allowed());
    }

    #[test]
    fn snooze_allowed_forever_ignores_count() {
        assert!(active_alarm(Some(snooze(SnoozeLimit::Forever)), 0).snooze_allowed());
        assert!(active_alarm(Some(snooze(SnoozeLimit::Forever)), 99).snooze_allowed());
    }

    #[test]
    fn snooze_allowed_until_limit_reached() {
        assert!(active_alarm(Some(snooze(SnoozeLimit::Three)), 0).snooze_allowed());
        assert!(active_alarm(Some(snooze(SnoozeLimit::Three)), 2).snooze_allowed());
        assert!(!active_alarm(Some(snooze(SnoozeLimit::Three)), 3).snooze_allowed());
        assert!(!active_alarm(Some(snooze(SnoozeLimit::Three)), 4).snooze_allowed());
    }

    const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(5);
    const BRIGHTNESS_AND_VOLUME_PCT: u8 = 50;

    /// A whole `AlarmController` over a tempdir config and crontab.
    /// Its alarms carry no sound, so nothing reaches `bmc-audio`.
    struct AlarmHarness {
        _tmp: tempfile::TempDir,
        _timezone: tokio::sync::watch::Sender<Timezone>,
        config: Arc<RwLock<ConfigHandle>>,
        bus: AlarmBus,
        controller: AlarmController,
    }

    impl AlarmHarness {
        async fn new() -> Self {
            Self::build(false).await
        }

        /// A harness whose crontab directory is a regular file,
        /// so the scheduler cannot create its crontab and `cancel` fails.
        /// Do not instead make the crontab path a directory:
        /// `Crontab::read_from_stream` discards read errors,
        /// and reading a directory yields EISDIR forever, so the scheduler spins.
        async fn with_unwritable_crontab() -> Self {
            Self::build(true).await
        }

        async fn build(break_crontab: bool) -> Self {
            let tmp = tempfile::tempdir().expect("BUG: tempdir creation must succeed in tests");
            let (config_handle, _) = ConfigHandle::init(
                tmp.path().join("bmc-config.json"),
                BRIGHTNESS_AND_VOLUME_PCT,
                BRIGHTNESS_AND_VOLUME_PCT,
                BRIGHTNESS_AND_VOLUME_PCT,
                BRIGHTNESS_AND_VOLUME_PCT,
                bmc_platform::Product::Bmc100,
            )
            .await;
            let config = Arc::new(RwLock::new(config_handle));
            let (timezone, timezone_receiver) = tokio::sync::watch::channel(Timezone::default());
            // Without a path of its own the scheduler writes /etc/crontabs/root,
            // which a test process may not open.
            let crontab = tmp.path().join("crontabs").join("root");
            if break_crontab {
                std::fs::write(
                    crontab
                        .parent()
                        .expect("BUG: the crontab path must have a parent"),
                    "",
                )
                .expect("BUG: writing the crontab directory stand-in must succeed in tests");
            }
            let scheduler = JobScheduler::init(timezone_receiver.clone(), Some(crontab)).await;
            let sound = SoundController::new(config.clone(), tmp.path().to_path_buf());
            let bus = AlarmBus::new();
            let controller = AlarmController::init(
                config.clone(),
                scheduler,
                sound,
                bus.clone(),
                timezone_receiver,
            )
            .await;

            Self {
                _tmp: tmp,
                _timezone: timezone,
                config,
                bus,
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
            self.bus.snooze();
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
                    let is_started =
                        matches!(&event, AlarmEvent::Started { alarm } if alarm.id == *id);
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
        harness.bus.stop_alarm(&snoozed.id);

        harness.ring(&other).await;
        harness.bus.stop_current();
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
        let mut events = harness.bus.subscribe_events();

        harness.ring(&dismissed).await;
        harness.bus.stop_current();
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
        harness.bus.snooze();
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
        harness.bus.snooze();
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
        let mut events = harness.bus.subscribe_events();

        harness.ring(&snoozed).await;
        harness.bus.snooze();
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

        let removed =
            tokio::time::timeout(SETTLE_TIMEOUT, harness.controller.remove_alarm(alarm.id))
                .await
                .expect("BUG: remove_alarm must not hang");
        removed.expect("BUG: remove_alarm must succeed");

        assert!(
            harness.config.read().await.alarms().is_empty(),
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

        let removed =
            tokio::time::timeout(SETTLE_TIMEOUT, harness.controller.remove_alarm(alarm.id))
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

    #[test]
    fn test_weekday_to_num_string() {
        let cases = [
            (WeekDay::Monday, "1".to_owned()),
            (WeekDay::Tuesday, "2".to_owned()),
            (WeekDay::Wednesday, "3".to_owned()),
            (WeekDay::Thursday, "4".to_owned()),
            (WeekDay::Friday, "5".to_owned()),
            (WeekDay::Saturday, "6".to_owned()),
            (WeekDay::Sunday, "7".to_owned()),
        ];

        for (weekday, expected) in cases {
            let num_string = weekday.as_number_string();
            assert_eq!(num_string, expected);
        }
    }

    #[test]
    fn test_weekday_to_string() {
        let cases = [
            (WeekDay::Monday, "Monday".to_owned()),
            (WeekDay::Tuesday, "Tuesday".to_owned()),
            (WeekDay::Wednesday, "Wednesday".to_owned()),
            (WeekDay::Thursday, "Thursday".to_owned()),
            (WeekDay::Friday, "Friday".to_owned()),
            (WeekDay::Saturday, "Saturday".to_owned()),
            (WeekDay::Sunday, "Sunday".to_owned()),
        ];

        for (weekday, expected) in cases {
            let value = weekday.to_string();
            assert_eq!(value, expected);
        }
    }

    #[test]
    fn test_parse_simple_alarm_data_to_cron() -> anyhow::Result<()> {
        let time = NaiveTime::parse_from_str("10:30", "%H:%M")?;
        let alarm_data = AlarmData::new(false, String::new(), time, BTreeSet::new(), None, None);

        let cron = alarm_data
            .cron()
            .expect("BUG: failed to create cron from alarm data");

        let cron_string = cron.to_string();

        assert_eq!(String::from("0 30 10 * * *"), cron_string);

        Ok(())
    }

    #[test]
    fn test_weekdays_to_num_string() {
        let cases: Vec<(BTreeSet<WeekDay>, &str)> = vec![
            (
                [WeekDay::Monday, WeekDay::Tuesday, WeekDay::Wednesday].into(),
                "1,2,3",
            ),
            (
                [WeekDay::Thursday, WeekDay::Monday, WeekDay::Sunday].into(),
                "1,4,7",
            ),
            ([].into(), ""),
        ];

        for (weekdays, expected) in cases {
            let num_string = AlarmData::weekdays_to_number_string(&weekdays);

            assert_eq!(&num_string, expected);
        }
    }

    #[test]
    fn test_parse_alarm_data_with_repeat_to_cron() -> anyhow::Result<()> {
        let time = NaiveTime::parse_from_str("14:58", "%H:%M")?;
        let repeat = [WeekDay::Monday, WeekDay::Tuesday, WeekDay::Wednesday].into();
        let alarm_data = AlarmData::new(false, String::new(), time, repeat, None, None);

        let cron = alarm_data
            .cron()
            .expect("BUG: failed to create cron from alarm data");

        let cron_string = cron.to_string();

        assert_eq!(String::from("0 58 14 * * 1,2,3"), cron_string);

        Ok(())
    }

    #[test]
    fn test_parse_alarm_data_with_all_days_to_cron() -> anyhow::Result<()> {
        let time = NaiveTime::parse_from_str("14:58", "%H:%M")?;
        let repeat = [
            WeekDay::Monday,
            WeekDay::Tuesday,
            WeekDay::Wednesday,
            WeekDay::Thursday,
            WeekDay::Friday,
            WeekDay::Saturday,
            WeekDay::Sunday,
        ]
        .into();

        let alarm_data = AlarmData::new(false, String::new(), time, repeat, None, None);

        let cron = alarm_data
            .cron()
            .expect("BUG: failed to create cron from alarm data");

        let cron_string = cron.to_string();

        assert_eq!(String::from("0 58 14 * * 1,2,3,4,5,6,7"), cron_string);

        Ok(())
    }

    fn na(fire: i64, name: &str) -> NextAlarm {
        NextAlarm {
            fire_at_utc_ms: fire,
            name: name.to_owned(),
        }
    }

    #[test]
    fn pick_soonest_returns_none_when_no_candidates() {
        assert_eq!(pick_soonest_next_alarm(None, vec![]), None);
    }

    #[test]
    fn pick_soonest_returns_scheduler_alone_when_no_snoozes() {
        let sched = na(1000, "morning");
        assert_eq!(
            pick_soonest_next_alarm(Some(sched.clone()), vec![]),
            Some(sched)
        );
    }

    #[test]
    fn pick_soonest_returns_snooze_alone_when_no_scheduler() {
        let snooze = na(500, "snoozed");
        assert_eq!(
            pick_soonest_next_alarm(None, vec![snooze.clone()]),
            Some(snooze)
        );
    }

    #[test]
    fn pick_soonest_prefers_snooze_when_it_fires_first() {
        let sched = na(1000, "morning cron");
        let snooze = na(400, "snoozed");
        assert_eq!(
            pick_soonest_next_alarm(Some(sched), vec![snooze.clone()]),
            Some(snooze)
        );
    }

    #[test]
    fn pick_soonest_prefers_scheduler_when_it_fires_first() {
        // A scheduled alarm dated before any pending snooze
        // keeps the broadcast pointing at the cron entry.
        //
        // Should not happen with current UX (snooze << next cron),
        // but the merge rule has to be order-independent.
        let sched = na(100, "morning cron");
        let snooze = na(900, "snoozed");
        assert_eq!(
            pick_soonest_next_alarm(Some(sched.clone()), vec![snooze]),
            Some(sched)
        );
    }

    #[test]
    fn pick_soonest_picks_earliest_among_multiple_snoozes() {
        let sched = na(2000, "cron");
        let snoozes = vec![na(1500, "later snooze"), na(800, "earlier snooze")];
        assert_eq!(
            pick_soonest_next_alarm(Some(sched), snoozes),
            Some(na(800, "earlier snooze"))
        );
    }

    // Tests that pin the full contract of `recompute_and_broadcast_next_alarm`
    // — across empty / present scheduler entries, race-window lookup misses,
    // and the snooze merge — so the next refactor can't silently drop a branch.
    //
    // NOTE: the `Err` arm of `JobScheduler::jobs_by_source` (which broadcasts `None`)
    // isn't covered here. `JobScheduler` is a concrete struct whose only failure mode
    // is internal lock poisoning, unreachable from its public API.
    //
    // Exercising that branch at function level would require trait-abstracting
    // the scheduler — left as follow-up.
    mod recompute {
        use std::str::FromStr;

        use bmc_scheduler::{
            Cron, JobScheduler,
            scheduler::{JobConfig, Schedule, Task},
        };
        use chrono::{DateTime, Utc};
        use tempfile::TempDir;

        use super::*;

        async fn make_scheduler() -> (JobScheduler, TempDir) {
            let temp_dir = TempDir::new().expect("BUG: failed to create tempdir");
            let (_tz_tx, tz_rx) = tokio::sync::watch::channel(Timezone::default());
            let scheduler = JobScheduler::init(tz_rx, Some(temp_dir.path().join("crontab"))).await;
            (scheduler, temp_dir)
        }

        async fn schedule_alarm_job(
            scheduler: &JobScheduler,
            name: &str,
        ) -> (AlarmId, ScheduledAlarm, DateTime<Utc>) {
            let cron = Cron::from_str("0 0 12 * * *").expect("BUG: failed to parse cron");
            let task: bmc_scheduler::BoxedTask = Box::new(|| Box::pin(async {}));
            let job_id = scheduler
                .schedule(
                    Schedule::Cron(cron),
                    Task::Async(task),
                    JobConfig::new(AlarmScheduler::SCHEDULER_SOURCE),
                )
                .await
                .expect("BUG: failed to schedule cron job");
            let next_tick = scheduler
                .jobs_by_source(AlarmScheduler::SCHEDULER_SOURCE)
                .await
                .expect("BUG: failed to fetch jobs by source")
                .into_iter()
                .find(|j| j.job_id == job_id)
                .and_then(|j| j.next_tick)
                .expect("BUG: scheduler left next_tick unset on the freshly scheduled job");
            (
                AlarmId::generate(),
                ScheduledAlarm {
                    job_id,
                    name: name.to_owned(),
                },
                next_tick,
            )
        }

        fn pending_snooze(fire_at_utc_ms: i64, name: &str) -> PendingSnooze {
            PendingSnooze {
                cancel: CancellationToken::new(),
                fire_at_utc_ms,
                name: name.to_owned(),
            }
        }

        async fn run(
            scheduler: &JobScheduler,
            active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>>,
            snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>>,
        ) -> Option<NextAlarm> {
            let (tx, rx) = tokio::sync::watch::channel::<Option<NextAlarm>>(None);
            recompute_and_broadcast_next_alarm(
                scheduler,
                AlarmScheduler::SCHEDULER_SOURCE,
                &active,
                &snoozes,
                &tx,
            )
            .await;
            rx.borrow().clone()
        }

        #[tokio::test]
        async fn empty_scheduler_and_no_snoozes_broadcasts_none() {
            let (scheduler, _temp_dir) = make_scheduler().await;
            let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
            let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
            assert_eq!(run(&scheduler, active, snoozes).await, None);
        }

        #[tokio::test]
        async fn empty_scheduler_with_snooze_broadcasts_snooze() {
            let (scheduler, _temp_dir) = make_scheduler().await;
            let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
            let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
            snoozes
                .lock()
                .await
                .insert(AlarmId::generate(), pending_snooze(42, "snooze"));
            assert_eq!(
                run(&scheduler, active, snoozes).await,
                Some(NextAlarm {
                    fire_at_utc_ms: 42,
                    name: "snooze".to_owned(),
                }),
            );
        }

        #[tokio::test]
        async fn scheduled_alarm_resolves_to_its_next_tick() {
            let (scheduler, _temp_dir) = make_scheduler().await;
            let (alarm_id, scheduled, tick) = schedule_alarm_job(&scheduler, "morning").await;
            let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
            active.lock().await.insert(alarm_id, scheduled);
            let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
            assert_eq!(
                run(&scheduler, active, snoozes).await,
                Some(NextAlarm {
                    fire_at_utc_ms: tick.timestamp_millis(),
                    name: "morning".to_owned(),
                }),
            );
        }

        #[tokio::test]
        async fn scheduler_wins_when_it_fires_before_snooze() {
            let (scheduler, _temp_dir) = make_scheduler().await;
            let (alarm_id, scheduled, tick) = schedule_alarm_job(&scheduler, "morning").await;
            let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
            active.lock().await.insert(alarm_id, scheduled);
            let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
            let later = tick.timestamp_millis() + 1_000_000;
            snoozes
                .lock()
                .await
                .insert(AlarmId::generate(), pending_snooze(later, "late snooze"));
            assert_eq!(
                run(&scheduler, active, snoozes).await,
                Some(NextAlarm {
                    fire_at_utc_ms: tick.timestamp_millis(),
                    name: "morning".to_owned(),
                }),
            );
        }

        #[tokio::test]
        async fn snooze_wins_when_it_fires_before_scheduler() {
            let (scheduler, _temp_dir) = make_scheduler().await;
            let (alarm_id, scheduled, tick) = schedule_alarm_job(&scheduler, "morning").await;
            let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
            active.lock().await.insert(alarm_id, scheduled);
            let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
            let earlier = tick.timestamp_millis() - 1_000_000;
            snoozes
                .lock()
                .await
                .insert(AlarmId::generate(), pending_snooze(earlier, "early snooze"));
            assert_eq!(
                run(&scheduler, active, snoozes).await,
                Some(NextAlarm {
                    fire_at_utc_ms: earlier,
                    name: "early snooze".to_owned(),
                }),
            );
        }

        // Race window between `jobs_by_source` and `active_alarms` lookup:
        // scheduler still reports a job_id whose entry has just been removed from `active_alarms`.
        // A pending snooze must still drive the broadcast — silently dropping it masks the snooze on every widget.
        #[tokio::test]
        async fn race_miss_with_snooze_broadcasts_snooze() {
            let (scheduler, _temp_dir) = make_scheduler().await;
            // Schedule a job so `jobs_by_source` returns one,
            // but don't insert it into `active_alarms` — the lookup misses.
            let _ = schedule_alarm_job(&scheduler, "ignored").await;
            let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
            let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
            snoozes
                .lock()
                .await
                .insert(AlarmId::generate(), pending_snooze(42, "snooze"));
            assert_eq!(
                run(&scheduler, active, snoozes).await,
                Some(NextAlarm {
                    fire_at_utc_ms: 42,
                    name: "snooze".to_owned(),
                }),
            );
        }

        // Race window with no snooze: the scheduler reports a job
        // whose `active_alarms` entry has just been removed.
        // With nothing else to offer,
        // the broadcast must resolve to "no alarm".
        #[tokio::test]
        async fn race_miss_without_snooze_broadcasts_none() {
            let (scheduler, _temp_dir) = make_scheduler().await;
            let _ = schedule_alarm_job(&scheduler, "ignored").await;
            let active: Arc<Mutex<HashMap<AlarmId, ScheduledAlarm>>> = Arc::default();
            let snoozes: Arc<Mutex<HashMap<AlarmId, PendingSnooze>>> = Arc::default();
            assert_eq!(run(&scheduler, active, snoozes).await, None);
        }
    }
}
