// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;

use bmc_scheduler::JobScheduler;
use bmc_scheduler::scheduler::JobConfig;
use bmc_scheduler::scheduler::Schedule;
use bmc_scheduler::scheduler::Task;
use bmc_shared_time::time::Timezone;
use chrono::NaiveTime;
use tokio::sync::{RwLock, watch};
use tracing::debug;

use crate::config::{ConfigHandle, NightModeConfig};

#[derive(Clone)]
pub(crate) struct NightModeController {
    config_handle: Arc<RwLock<ConfigHandle>>,
    scheduler: JobScheduler,
    timezone_receiver: watch::Receiver<Timezone>,
    is_active_sender: watch::Sender<bool>,
}

impl NightModeController {
    const NIGHT_MODE_SCHEDULER_SOURCE: &'static str = "NightMode";

    pub(crate) async fn init(
        config_handle: Arc<RwLock<ConfigHandle>>,
        scheduler: JobScheduler,
        timezone_receiver: watch::Receiver<Timezone>,
    ) -> anyhow::Result<Self> {
        let night_mode = config_handle.read().await.night_mode();
        let timezone = timezone_receiver.borrow().clone();
        let is_active = night_mode.is_active(&timezone);
        let (is_active_sender, _) = watch::channel(is_active);

        tokio::spawn(Self::update_is_active_on_timezone_change(
            config_handle.clone(),
            timezone_receiver.clone(),
            is_active_sender.clone(),
        ));

        let this = Self {
            config_handle,
            scheduler,
            timezone_receiver,
            is_active_sender,
        };

        this.schedule_jobs(&night_mode).await?;

        Ok(this)
    }

    async fn update_is_active_on_timezone_change(
        config_handle: Arc<RwLock<ConfigHandle>>,
        mut timezone_receiver: watch::Receiver<Timezone>,
        is_active_sender: watch::Sender<bool>,
    ) {
        while let Ok(()) = timezone_receiver.changed().await {
            let night_mode = config_handle.read().await.night_mode();
            let timezone = timezone_receiver.borrow_and_update().clone();
            let is_active = night_mode.is_active(&timezone);
            is_active_sender.send_replace(is_active);
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.is_active_sender.subscribe()
    }

    pub(crate) async fn config(&self) -> NightModeConfig {
        self.config_handle.read().await.night_mode()
    }

    pub(crate) async fn set_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_enabled(enabled);
        config_handle.save().await?;

        let night_mode = config_handle.night_mode();
        drop(config_handle);

        self.schedule_jobs(&night_mode).await?;

        let timezone = self.timezone_receiver.borrow().clone();
        let is_active = night_mode.is_active(&timezone);
        self.is_active_sender.send_replace(is_active);

        Ok(())
    }

    pub(crate) async fn set_interval(&self, from: NaiveTime, to: NaiveTime) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_interval(from, to);
        config_handle.save().await?;

        let night_mode = config_handle.night_mode();
        drop(config_handle);

        self.schedule_jobs(&night_mode).await?;

        let timezone = self.timezone_receiver.borrow().clone();
        let is_active = night_mode.is_active(&timezone);
        self.is_active_sender.send_replace(is_active);

        Ok(())
    }

    pub(crate) async fn set_brightness(&self, value_pct: u8) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_brightness(value_pct);
        config_handle.save().await?;

        Ok(())
    }

    pub(crate) async fn set_sound_volume(&self, sound_volume_pct: u8) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_sound_volume(sound_volume_pct);
        config_handle.save().await?;

        Ok(())
    }

    async fn schedule_jobs(&self, night_mode: &NightModeConfig) -> anyhow::Result<()> {
        debug!("cancel scheduled nightmode jobs");

        self.scheduler
            .cancel_jobs(Self::NIGHT_MODE_SCHEDULER_SOURCE.to_owned())
            .await;

        if !night_mode.enabled {
            return Ok(());
        }

        debug!("schedule nightmode jobs");

        let from_cron = bmc_scheduler::cron::from_naive_time(night_mode.from)?;
        let to_cron = bmc_scheduler::cron::from_naive_time(night_mode.to)?;

        self.scheduler
            .schedule(
                Schedule::Cron(from_cron),
                Task::Async({
                    let is_active_sender = self.is_active_sender.clone();
                    Box::new(move || {
                        let is_active_sender = is_active_sender.clone();
                        Box::pin(async move {
                            is_active_sender.send_replace(true);
                        })
                    })
                }),
                JobConfig::new(Self::NIGHT_MODE_SCHEDULER_SOURCE.to_owned()),
            )
            .await?;

        self.scheduler
            .schedule(
                Schedule::Cron(to_cron),
                Task::Async({
                    let is_active_sender = self.is_active_sender.clone();
                    Box::new(move || {
                        let is_active_sender = is_active_sender.clone();
                        Box::pin(async move {
                            is_active_sender.send_replace(false);
                        })
                    })
                }),
                JobConfig::new(Self::NIGHT_MODE_SCHEDULER_SOURCE.to_owned()),
            )
            .await?;

        Ok(())
    }
}

impl std::fmt::Debug for NightModeController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NightModeController").finish()
    }
}
