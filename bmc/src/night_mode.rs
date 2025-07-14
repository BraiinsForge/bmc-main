// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;

use bmc_scheduler::BoxedTask;
use bmc_scheduler::JobScheduler;
use bmc_scheduler::jobs::to_boxed;
use bmc_scheduler::scheduler::JobConfig;
use bmc_scheduler::scheduler::Schedule;
use bmc_scheduler::scheduler::Task;
use bmc_shared_time::time::Timezone;
use chrono::{Local, NaiveTime};
use tokio::sync::{RwLock, watch};
use tracing::debug;

use crate::config::ConfigHandle;
use crate::config::NightModeConfig as NightModeConfiguration;

#[derive(Debug, Clone)]
pub(crate) struct NightModeConfig {
    pub(crate) from: NaiveTime,
    pub(crate) to: NaiveTime,
    pub(crate) enabled: bool,
}

impl From<NightModeConfiguration> for NightModeConfig {
    fn from(value: NightModeConfiguration) -> Self {
        Self {
            enabled: value.enabled,
            from: value.from,
            to: value.to,
        }
    }
}

#[derive(Clone)]
pub(crate) struct NightModeController {
    config_handle: Arc<RwLock<ConfigHandle>>,
    scheduler: JobScheduler,
    timezone_receiver: watch::Receiver<Timezone>,
    activate_night_mode_task: Arc<BoxedTask>,
    deactivate_night_mode_task: Arc<BoxedTask>,
}

impl NightModeController {
    const NIGHT_MODE_SCHEDULER_SOURCE: &str = "NightMode";

    pub(crate) fn new(
        config_handle: Arc<RwLock<ConfigHandle>>,
        scheduler: JobScheduler,
        timezone_receiver: watch::Receiver<Timezone>,
        activate_night_mode_task: BoxedTask,
        deactivate_night_mode_task: BoxedTask,
    ) -> Self {
        Self {
            config_handle,
            scheduler,
            timezone_receiver,
            activate_night_mode_task: Arc::new(activate_night_mode_task),
            deactivate_night_mode_task: Arc::new(deactivate_night_mode_task),
        }
    }

    pub(crate) async fn init(&self, night_mode: NightModeConfig) -> anyhow::Result<()> {
        if night_mode.enabled {
            let _ = self.enable_disable_night_mode_service(night_mode).await;
        }

        Ok(())
    }

    pub(crate) async fn night_mode_config(&self) -> NightModeConfiguration {
        self.config_handle.read().await.night_mode()
    }

    pub(crate) async fn set_night_mode_enabled(
        &self,
        enabled: bool,
    ) -> anyhow::Result<NightModeConfig> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_enabled(enabled);

        let config: NightModeConfig = config_handle.night_mode().into();
        self.enable_disable_night_mode_service(config.clone())
            .await?;

        config_handle.sync_to_storage().await?;

        Ok(config)
    }

    pub(crate) async fn set_night_mode_interval(
        &self,
        from: NaiveTime,
        to: NaiveTime,
    ) -> anyhow::Result<NightModeConfig> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_interval(from, to);

        let config: NightModeConfig = config_handle.night_mode().into();
        self.enable_disable_night_mode_service(config.clone())
            .await?;

        config_handle.sync_to_storage().await?;

        Ok(config)
    }

    pub(crate) async fn set_night_mode_brightness(
        &self,
        value_pct: u8,
    ) -> anyhow::Result<NightModeConfig> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_brightness(value_pct);

        let config: NightModeConfig = config_handle.night_mode().into();

        config_handle.sync_to_storage().await?;

        Ok(config)
    }

    async fn enable_disable_night_mode_service(
        &self,
        config: NightModeConfig,
    ) -> anyhow::Result<()> {
        self.cancel_scheduled_night_mode().await;

        if !config.enabled {
            return Ok(());
        }

        debug!("schedule nightmode jobs");

        let from_cron = bmc_scheduler::cron::from_naive_time(config.from)?;
        let to_cron = bmc_scheduler::cron::from_naive_time(config.to)?;

        self.scheduler
            .schedule(
                Schedule::Cron(from_cron),
                Task::Async(to_boxed(self.activate_night_mode_task.clone())),
                JobConfig::new(Self::NIGHT_MODE_SCHEDULER_SOURCE.to_owned()),
            )
            .await?;

        self.scheduler
            .schedule(
                Schedule::Cron(to_cron),
                Task::Async(to_boxed(self.deactivate_night_mode_task.clone())),
                JobConfig::new(Self::NIGHT_MODE_SCHEDULER_SOURCE.to_owned()),
            )
            .await?;

        Ok(())
    }

    async fn cancel_scheduled_night_mode(&self) {
        debug!("cancel scheduled nightmode jobs");

        self.scheduler
            .cancel_jobs(Self::NIGHT_MODE_SCHEDULER_SOURCE.to_owned())
            .await;
    }

    pub(crate) fn is_night_mode(&self, night_mode: &NightModeConfig) -> bool {
        let timezone: chrono_tz::Tz = (&self.timezone_receiver.borrow().clone()).into();
        let now = Local::now().with_timezone(&timezone).time();

        night_mode.enabled && Self::is_time_in_range(night_mode.from, night_mode.to, now)
    }

    /// Checks whether `now` (in local time) is in the [from, to) range.
    /// Handles ranges that cross midnight.
    fn is_time_in_range(from: NaiveTime, to: NaiveTime, now: NaiveTime) -> bool {
        if from <= to {
            now >= from && now < to
        } else {
            now >= from || now < to
        }
    }
}

impl std::fmt::Debug for NightModeController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NightModeController")
            .field("config_handle", &self.config_handle)
            .field("scheduler", &self.scheduler)
            .field("timezone_receiver", &self.timezone_receiver)
            .field("activate_night_mode_task", &"<task>")
            .field("deactivate_night_mode_task", &"<task>")
            .finish()
    }
}
