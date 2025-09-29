// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{
    collections::{HashMap, HashSet},
    fmt::{Display, Formatter},
    pin::Pin,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::anyhow;
use bmc_display::display_controller::DisplayController;
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
    },
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
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
    fn limit(&self) -> Option<u32> {
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
enum AlarmCmd {
    StopAll,
    Stop { id: AlarmId },
    Snooze,
}

#[derive(Clone, Debug)]
pub enum AlarmEvent {
    Stopped { id: AlarmId },
    Snoozed,
    Started,
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

    pub fn stop_all(&self) {
        let _ = self.tx_commands.send(AlarmCmd::StopAll);
    }

    pub fn stop_alarm(&self, id: AlarmId) {
        let _ = self.tx_commands.send(AlarmCmd::Stop { id });
    }

    pub fn snooze(&self) {
        let _ = self.tx_commands.send(AlarmCmd::Snooze);
    }

    pub fn send_event(&self, event: AlarmEvent) {
        let _ = self.tx_events.send(event);
    }

    fn subscribe_commands(&self) -> broadcast::Receiver<AlarmCmd> {
        self.tx_commands.subscribe()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<AlarmEvent> {
        self.tx_events.subscribe()
    }
}

#[derive(Debug, Clone)]
struct Alarm {
    data: AlarmData,
    sound_controller: SoundController,
    display_controller: DisplayController,
}

impl Alarm {
    const DEFAULT_LABEL: &str = "Alarm";

    fn new(
        data: AlarmData,
        sound_controller: SoundController,
        display_controller: DisplayController,
    ) -> Self {
        Self {
            data,
            sound_controller,
            display_controller,
        }
    }

    fn start(&self, token: CancellationToken, snoozed_count: u32) {
        let label = if self.data.name.is_empty() {
            Self::DEFAULT_LABEL.to_owned()
        } else {
            self.data.name.clone()
        };

        let show_snooze = self.data.snooze_options.as_ref().is_some_and(|options| {
            options
                .limit
                .limit()
                .is_some_and(|limit| snoozed_count < limit)
        });

        self.display_controller.set_alarm_data(label, show_snooze);

        self.display_controller.set_clock_alarm_screen(true);

        if let Some(sound) = self.data.sound.clone() {
            _ = tokio::spawn({
                let sound_controller = self.sound_controller.clone();
                let token = token.clone();
                let name = self.data.name.clone();
                async move {
                    while !token.is_cancelled() {
                        debug!("Alarm [{name}] playing sound");
                        if let Err(e) = sound_controller
                            .play_sound(sound.clone(), token.clone())
                            .await
                        {
                            warn!("Failed to play sound, err: {e}");
                            //NOTE: try to play sound in 5 secs. Resource can by busy playing other sounds
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                }
            });

            _ = tokio::spawn({
                let display_controller = self.display_controller.clone();
                async move {
                    let deadline =
                        tokio::time::sleep_until(Instant::now() + Duration::from_secs(60 * 10));
                    tokio::pin!(deadline);

                    tokio::select! {
                        () = token.cancelled() => {
                            debug!("Alarm cancelled before timeout");
                            display_controller.set_clock_alarm_screen(false);
                        }
                        () = &mut deadline => {
                            debug!("Alarm timed out after 10 minutes");
                            token.cancel();
                            display_controller.set_clock_alarm_screen(false);
                        }
                    }
                }
            });
        }
    }
}

#[derive(Clone, Debug)]
struct AlarmScheduler {
    scheduler: JobScheduler,
    active_alarms: Arc<Mutex<HashMap<AlarmId, Uuid>>>,
    alarm_bus: AlarmBus,
    timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
    display_controller: DisplayController,
}

impl AlarmScheduler {
    const SCHEDULER_SOURCE: &str = "Alarm";

    fn new(
        scheduler: JobScheduler,
        alarm_bus: AlarmBus,
        timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
        display_controller: DisplayController,
    ) -> Self {
        let alarm_scheduler = Self {
            scheduler,
            active_alarms: Arc::default(),
            alarm_bus,
            timezone_receiver,
            display_controller,
        };

        tokio::spawn({
            let mut self_ = alarm_scheduler.clone();
            async move {
                while let Ok(()) = self_.timezone_receiver.changed().await {
                    self_.recompute_next_alarm_time().await;
                }
            }
        });

        alarm_scheduler
    }

    async fn schedule(&self, alarm: Alarm) -> anyhow::Result<()> {
        let cron = alarm.data.cron()?;
        let name = alarm.data.name.clone();

        debug!(
            "Scheduling alarm [{name}] with cron [{cron:?}], sound: [{:?}]",
            alarm.data.sound
        );

        let task = {
            let alarm = alarm.clone();
            let alarm_bus = self.alarm_bus.clone();
            let self_ = self.clone();

            move || {
                let alarm = alarm.clone();
                let name = name.clone();
                let alarm_bus = alarm_bus.clone();
                let self_ = self_.clone();

                Box::pin(async move {
                    debug!("starting alarm: [{}]", name);
                    // NOTE: Stop all current running alarm(s). New alarm should "take over" display and sound output
                    alarm_bus.stop_all();

                    let mut rx = alarm_bus.subscribe_commands();

                    let mut token = CancellationToken::new();
                    let mut _drop_guard = token.clone().drop_guard();

                    let mut snooze_count = 0_u32;
                    alarm.start(token.clone(), snooze_count);

                    alarm_bus.send_event(AlarmEvent::Started);

                    self_.recompute_next_alarm_time().await;

                    loop {
                        match rx.recv().await {
                            Ok(msg) => match msg {
                                AlarmCmd::StopAll => {
                                    debug!("Alarm stop received. Cancelling alarm [{name}]");
                                    token.cancel();
                                    alarm_bus.send_event(AlarmEvent::Stopped {
                                        id: alarm.data.id.clone(),
                                    });
                                    break;
                                }
                                AlarmCmd::Stop { id } => {
                                    if alarm.data.id == id {
                                        debug!("Alarm stop received. Cancelling alarm [{name}]");
                                        token.cancel();
                                        alarm_bus.send_event(AlarmEvent::Stopped {
                                            id: alarm.data.id.clone(),
                                        });
                                        break;
                                    }
                                }
                                AlarmCmd::Snooze => {
                                    token.cancel();
                                    alarm_bus.send_event(AlarmEvent::Snoozed);

                                    debug!(
                                        "Alarm snooze received. Cancelling alarm [{name}], scheduling snooze"
                                    );

                                    if let Some(snooze) = alarm.data.snooze_options.as_ref() {
                                        let duration = snooze.duration.duration();

                                        token = CancellationToken::new();
                                        _drop_guard = token.clone().drop_guard();

                                        snooze_count += 1;

                                        tokio::spawn({
                                            let token = token.clone();
                                            let alarm = alarm.clone();
                                            let alarm_bus = alarm_bus.clone();
                                            async move {
                                                debug!("Snooze for {:?}...", duration);
                                                tokio::time::sleep(duration).await;
                                                debug!("Starting alarm after snooze");
                                                alarm.start(token, snooze_count);
                                                alarm_bus.send_event(AlarmEvent::Started);
                                            }
                                        });
                                    } else {
                                        break;
                                    }
                                }
                            },
                            Err(e) => {
                                warn!("Alarm received error: [{:?}]", e);

                                if !token.is_cancelled() {
                                    token.cancel();
                                }
                                break;
                            }
                        }
                    }
                }) as Pin<Box<dyn Future<Output = ()> + Send>>
            }
        };

        let id: Uuid = self
            .scheduler
            .schedule(
                Schedule::Cron(cron),
                Task::Async(Box::new(task)),
                JobConfig::new(Self::SCHEDULER_SOURCE),
            )
            .await?;

        self.active_alarms.lock().await.insert(alarm.data.id, id);

        self.recompute_next_alarm_time().await;

        Ok(())
    }

    async fn remove(&self, id: &AlarmId) -> anyhow::Result<()> {
        if let Some(ref job_id) = self.active_alarms.lock().await.remove(id) {
            self.alarm_bus.stop_alarm(id.clone());

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
            self.display_controller.set_next_alarm(None);
            return;
        };

        let timezone = self.timezone_receiver.borrow().clone();

        let maybe_next_alarm_dt = alarms
            .into_iter()
            .filter_map(|alarm| alarm.next_tick)
            .sorted()
            .next()
            .map(|next_tick| next_tick.with_timezone(timezone.chrono()).fixed_offset());

        self.display_controller.set_next_alarm(maybe_next_alarm_dt);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AlarmController {
    config_handle: Arc<RwLock<ConfigHandle>>,
    scheduler: AlarmScheduler,
    sound_controller: SoundController,
    display_controller: DisplayController,
}

impl AlarmController {
    pub(crate) async fn init(
        config_handle: Arc<RwLock<ConfigHandle>>,
        scheduler: JobScheduler,
        sound_controller: SoundController,
        display_controller: DisplayController,
        alarm_bus: AlarmBus,
        timezone_receiver: tokio::sync::watch::Receiver<Timezone>,
    ) -> anyhow::Result<Self> {
        let scheduler = AlarmScheduler::new(
            scheduler,
            alarm_bus.clone(),
            timezone_receiver,
            display_controller.clone(),
        );

        for alarm_data in config_handle
            .clone()
            .read()
            .await
            .alarms()
            .iter()
            .filter(|alarm| alarm.enabled)
        {
            let alarm = Alarm::new(
                alarm_data.clone(),
                sound_controller.clone(),
                display_controller.clone(),
            );

            scheduler.schedule(alarm).await?;
        }

        let controller = Self {
            config_handle,
            scheduler,
            sound_controller,
            display_controller,
        };

        let self_ = controller.clone();

        tokio::spawn(async move {
            let mut rx = alarm_bus.subscribe_events();

            while let Ok(event) = rx.recv().await {
                match event {
                    AlarmEvent::Stopped { id } => {
                        if let Some(alarm) =
                            self_.alarms().await.iter().find(|alarm| alarm.id == id)
                        {
                            if alarm.repeat.is_empty() {
                                _ = self_.set_enabled(id, false).await;
                            }
                        }
                        // TODO: Turn off LED strip
                    }
                    AlarmEvent::Snoozed | AlarmEvent::Started => {
                        // TODO: Turn off LED strip
                    }
                }
            }
        });

        Ok(controller)
    }

    pub(crate) async fn alarms(&self) -> Vec<AlarmData> {
        self.config_handle.read().await.alarms()
    }

    pub(crate) async fn add_alarm(&self, alarm_data: AlarmData) -> Result<(), AlarmError> {
        let mut config = self.config_handle.write().await;
        let mut temp_config = config.clone();

        if config
            .alarms()
            .iter()
            .any(|existing| existing.time == alarm_data.time)
        {
            return Err(AlarmError::DuplicateAlarm);
        }

        let alarm_id = alarm_data.id.clone();

        if alarm_data.enabled {
            let alarm = Alarm::new(
                alarm_data.clone(),
                self.sound_controller.clone(),
                self.display_controller.clone(),
            );

            self.scheduler.schedule(alarm).await.map_err(|e| {
                warn!("Failed to schedule alarm, err {e}");
                AlarmError::ScheduleAlarm
            })?;
        }

        temp_config.add_alarm(alarm_data);

        if let Err(e) = temp_config.save().await {
            warn!("Failed to store alarm, err {e}");

            _ = self
                .scheduler
                .remove(&alarm_id)
                .await
                .inspect_err(|e| warn!("{:?}", e));

            return Err(AlarmError::SyncToStorage);
        }
        *config = temp_config;

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
            return Err(AlarmError::NotFound);
        }

        if config
            .alarms()
            .iter()
            .any(|existing| existing.time == alarm_data.time && existing.id != alarm_id)
        {
            return Err(AlarmError::DuplicateAlarm);
        }

        self.scheduler.remove(&alarm_id).await.map_err(|e| {
            warn!("{:?}", e);
            AlarmError::RemoveAlarm
        })?;

        if alarm_data.enabled {
            let alarm = Alarm::new(
                alarm_data.clone(),
                self.sound_controller.clone(),
                self.display_controller.clone(),
            );

            self.scheduler.schedule(alarm).await.map_err(|e| {
                warn!("Failed to schedule alarm, err {e}");
                AlarmError::ScheduleAlarm
            })?;
        }

        temp_config.set_alarm(alarm_data);

        if let Err(e) = temp_config.save().await {
            warn!("Failed to store alarm, err {e}");

            _ = self
                .scheduler
                .remove(&alarm_id)
                .await
                .inspect_err(|e| warn!("{:?}", e));

            return Err(AlarmError::SyncToStorage);
        }

        *config = temp_config;

        Ok(())
    }

    pub(crate) async fn remove_alarm(&self, alarm_id: AlarmId) -> Result<(), AlarmError> {
        let mut config = self.config_handle.write().await;
        let mut temp_config = config.clone();

        if !config.alarms().iter().any(|alarm| alarm.id == alarm_id) {
            return Err(AlarmError::NotFound);
        }

        self.scheduler.remove(&alarm_id).await.map_err(|e| {
            warn!("{:?}", e);
            AlarmError::RemoveAlarm
        })?;

        temp_config.remove_alarm(&alarm_id);

        temp_config.save().await.map_err(|e| {
            warn!("Failed to store alarm, err {e}");
            AlarmError::SyncToStorage
        })?;

        *config = temp_config;

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
            .ok_or(AlarmError::NotFound)?;

        alarm_data.enabled = enabled;

        self.scheduler.remove(&alarm_data.id).await.map_err(|e| {
            warn!("{:?}", e);
            AlarmError::RemoveAlarm
        })?;

        if alarm_data.enabled {
            let alarm = Alarm::new(
                alarm_data.clone(),
                self.sound_controller.clone(),
                self.display_controller.clone(),
            );

            self.scheduler.schedule(alarm).await.map_err(|e| {
                warn!("Failed to schedule alarm, err {e}");
                AlarmError::ScheduleAlarm
            })?;
        }

        temp_config.set_alarm(alarm_data);

        if let Err(e) = temp_config.save().await {
            warn!("Failed to store alarm, err {e}");

            _ = self
                .scheduler
                .remove(&alarm_id)
                .await
                .inspect_err(|e| warn!("{:?}", e));

            return Err(AlarmError::SyncToStorage);
        }
        *config = temp_config;

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
            .expect("Failed to create cron from alarm data");

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
            .expect("Failed to create cron from alarm data");

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
            .expect("Failed to create cron from alarm data");

        let cron_string = cron.to_string();

        assert_eq!(String::from("0 58 14 * * 1,2,3,4,5,6,7"), cron_string);

        Ok(())
    }
}
