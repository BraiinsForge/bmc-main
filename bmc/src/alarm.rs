// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{
    collections::{HashMap, HashSet},
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
    pub(crate) repeat: HashSet<WeekDay>,
    pub(crate) sound: Option<Sounds>,
    pub(crate) snooze_options: Option<SnoozeOptions>,
}

impl AlarmData {
    pub(crate) fn new(
        enabled: bool,
        name: String,
        time: NaiveTime,
        repeat: HashSet<WeekDay>,
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
        repeat: HashSet<WeekDay>,
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

    fn weekdays_to_number_string(set: &HashSet<WeekDay>) -> String {
        let mut nums: Vec<String> = set.iter().map(|day| day.as_number_string()).collect();

        nums.sort();
        nums.join(",")
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
    #[expect(dead_code, reason = "consumed by future display-overlay channel")]
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
            SnoozeDuration::FiveMinutes => Duration::from_secs(5 * 60),
            SnoozeDuration::TenMinutes => Duration::from_secs(10 * 60),
            SnoozeDuration::FifteenMinutes => Duration::from_secs(15 * 60),
            SnoozeDuration::ThirtyMinutes => Duration::from_secs(30 * 60),
        }
    }
}

#[derive(Clone, Debug)]
pub enum AlarmCmd {
    StopAll,
    Stop { id: AlarmId },
    Snooze,
}

#[derive(Clone, Debug)]
pub enum AlarmEvent {
    Stopped {
        id: AlarmId,
    },
    Snoozed,
    Started {
        #[expect(dead_code, reason = "consumed by future display-overlay channel")]
        alarm: ActiveAlarm,
    },
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

    #[expect(dead_code, reason = "consumed by future display-overlay channel")]
    pub fn stop_all(&self) {
        if let Err(err) = self.tx_commands.send(AlarmCmd::StopAll) {
            warn!(error = %err, "Failed to send StopAll command, no active receivers");
        }
    }

    pub fn stop_alarm(&self, id: &AlarmId) {
        if let Err(err) = self.tx_commands.send(AlarmCmd::Stop { id: id.clone() }) {
            warn!(alarm_id = %id, error = %err, "Failed to send Stop command, no active receivers");
        }
    }

    #[expect(dead_code, reason = "consumed by future display-overlay channel")]
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

#[derive(Clone, Debug)]
struct AlarmScheduler {
    scheduler: JobScheduler,
    active_alarms: Arc<Mutex<HashMap<AlarmId, Uuid>>>,
    alarm_bus: AlarmBus,
    timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
    alarm_sender: mpsc::Sender<ActiveAlarm>,
    current_alarm: Arc<Mutex<Option<CurrentRunningAlarm>>>,
    sound_controller: SoundController,
}

impl AlarmScheduler {
    const SCHEDULER_SOURCE: &str = "Alarm";
    const ALARM_TIMEOUT: Duration = Duration::from_secs(60 * 10);

    fn init(
        scheduler: JobScheduler,
        alarm_bus: AlarmBus,
        timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
        sound_controller: SoundController,
    ) -> Self {
        let (alarm_sender, alarm_receiver) = mpsc::channel(1);

        let this = Self {
            scheduler,
            active_alarms: Arc::default(),
            alarm_bus,
            timezone_receiver,
            alarm_sender,
            current_alarm: Arc::default(),
            sound_controller,
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

                    //NOTE: Check if active_alarm is really enabled. It could happen that alarm is snoozed and then it is manually removed or disabled
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

                    // NOTE: Spawn task with playing alarm sound. Reason why this task isn't in the tokio::select! section below is that bmc-audio crate uses tokio::select!
                    // internally for playing sound and cancelling of the process. Having this part in tokio::select! could lead to errors when cancelling sound playing.
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
                                    // Clear the slot here so repeating alarms don't leak a
                                    // stale entry and the non-repeating set_enabled(false)
                                    // path can't emit a duplicate Stopped via the command
                                    // handler.
                                    let _ = current_alarm
                                        .lock()
                                        .await
                                        .take_if(|c| c.alarm.id == alarm_id);
                                    alarm_bus.send_event(AlarmEvent::Stopped { id: alarm_id });
                                }
                            }
                        }
                    });

                    self_.alarm_bus.send_event(AlarmEvent::Started {
                        alarm: active_alarm.clone(),
                    });

                    {
                        let mut current_alarm_guard = self_.current_alarm.lock().await;
                        *current_alarm_guard = Some(CurrentRunningAlarm::new(
                            active_alarm,
                            token,
                            sound_join_handle,
                            timeout_join_handle,
                        ));
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
                while let Ok(cmd) = rx.recv().await {
                    match cmd {
                        AlarmCmd::StopAll => {
                            info!("Received StopAll command");
                            self_.cancel_current_alarm().await;
                        }
                        AlarmCmd::Stop { id } => {
                            info!(alarm_id = %id, "Received Stop command");
                            if let Some(alarm) = self_
                                .current_alarm
                                .lock()
                                .await
                                .take_if(|current| current.alarm.id == id)
                            {
                                alarm.cancel().await;
                                self_.alarm_bus.send_event(AlarmEvent::Stopped { id });
                            }
                        }
                        AlarmCmd::Snooze => {
                            info!("Received Snooze command");
                            if let Some(active_alarm) = self_.current_alarm.lock().await.take() {
                                let mut alarm = active_alarm.alarm.clone();
                                active_alarm.cancel().await;
                                self_.alarm_bus.send_event(AlarmEvent::Snoozed);

                                if let Some(snooze) = alarm.snooze_options.as_ref() {
                                    let duration = snooze.duration.duration();

                                    tokio::spawn({
                                        let self_ = self_.clone();
                                        alarm.snooze_count += 1;
                                        let snooze_count = alarm.snooze_count;
                                        let alarm_id = alarm.id.clone();

                                        async move {
                                            info!(
                                                alarm_id = %alarm_id,
                                                snooze_count = snooze_count,
                                                snooze_duration_secs = duration.as_secs(),
                                                "Alarm snoozed, will retrigger after duration"
                                            );
                                            tokio::time::sleep(duration).await;
                                            if let Err(err) = self_.alarm_sender.send(alarm).await {
                                                warn!(error = %err, "Failed to trigger snoozed alarm");
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    async fn cancel_current_alarm(&self) {
        if let Some(running_alarm) = self.current_alarm.lock().await.take() {
            let alarm_data = running_alarm.alarm.data.clone();
            let alarm_id = &alarm_data.id;
            let alarm_name = &alarm_data.name;

            running_alarm.cancel().await;

            self.alarm_bus.send_event(AlarmEvent::Stopped {
                id: alarm_data.id.clone(),
            });

            info!(alarm_id = %alarm_id, alarm_name = %alarm_name, "Cancelled active alarm");
        }
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

        self.active_alarms
            .lock()
            .await
            .insert(alarm_data.id.clone(), schedule_id);

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
        if let Some(ref job_id) = self.active_alarms.lock().await.remove(id) {
            self.alarm_bus.stop_alarm(id);

            self.scheduler
                .cancel(job_id)
                .await
                .map_err(|e| anyhow!("Failed to remove scheduled alarm, err: {e}"))?;
        }

        self.recompute_next_alarm_time().await;
        Ok(())
    }

    async fn recompute_next_alarm_time(&self) {
        let Ok(alarms) = self.scheduler.jobs_by_source(Self::SCHEDULER_SOURCE).await else {
            error!("Failed to get scheduled alarms from scheduler");
            return;
        };

        let timezone = self.timezone_receiver.borrow().clone();

        let maybe_next_alarm_dt = alarms
            .into_iter()
            .filter_map(|alarm| alarm.next_tick)
            .sorted()
            .next()
            .map(|next_tick| next_tick.with_timezone(timezone.chrono()).fixed_offset());

        info!(next_alarm = ?maybe_next_alarm_dt, "Recomputed next alarm time");
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

            while let Ok(event) = rx.recv().await {
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
        let alarm_data = AlarmData::new(false, String::new(), time, HashSet::new(), None, None);

        let cron = alarm_data
            .cron()
            .expect("BUG: failed to create cron from alarm data");

        let cron_string = cron.to_string();

        assert_eq!(String::from("0 30 10 * * *"), cron_string);

        Ok(())
    }

    #[test]
    fn test_weekdays_to_num_string() {
        let cases: Vec<(HashSet<WeekDay>, &str)> = vec![
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
}
