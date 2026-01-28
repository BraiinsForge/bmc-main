// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;

use bmc_scheduler::JobScheduler;
use bmc_scheduler::scheduler::JobConfig;
use bmc_scheduler::scheduler::Schedule;
use bmc_scheduler::scheduler::Task;
use bmc_shared_time::time::Timezone;
use chrono::NaiveTime;
use tokio::sync::{RwLock, watch};
use tracing::{debug, error, info};

use crate::config::{ConfigHandle, NightModeConfig};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum NightModeOverride {
    None,          // Follow schedule
    ForceActive,   // User turned on outside hours - turn off at scheduled 'to'
    ForceInactive, // User turned off during hours - turn on at next scheduled 'from'
}

#[derive(Clone)]
pub(crate) struct NightModeController {
    config_handle: Arc<RwLock<ConfigHandle>>,
    scheduler: JobScheduler,
    timezone_receiver: watch::Receiver<Timezone>,
    is_active_sender: watch::Sender<bool>,
    override_state: Arc<RwLock<NightModeOverride>>,
}

impl NightModeController {
    const NIGHT_MODE_SCHEDULER_SOURCE: &'static str = "NightMode";

    pub(crate) async fn init(
        config_handle: Arc<RwLock<ConfigHandle>>,
        scheduler: JobScheduler,
        timezone_receiver: watch::Receiver<Timezone>,
    ) -> Self {
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
            override_state: Arc::new(RwLock::new(NightModeOverride::None)),
        };

        if let Err(err) = this.schedule_jobs(&night_mode).await {
            error!(
                error = %err,
                enabled = night_mode.enabled,
                from = %night_mode.from,
                to = %night_mode.to,
                "Failed to schedule night mode jobs"
            );
        } else {
            info!(
                enabled = night_mode.enabled,
                is_active = is_active,
                "Night mode controller initialized"
            );
        }

        this
    }

    async fn update_is_active_on_timezone_change(
        config_handle: Arc<RwLock<ConfigHandle>>,
        mut timezone_receiver: watch::Receiver<Timezone>,
        is_active_sender: watch::Sender<bool>,
    ) {
        loop {
            match timezone_receiver.changed().await {
                Ok(()) => {
                    let night_mode = config_handle.read().await.night_mode();
                    let timezone = timezone_receiver.borrow_and_update().clone();
                    let is_active = night_mode.is_active(&timezone);
                    let previous_state = is_active_sender.send_replace(is_active);

                    if previous_state != is_active {
                        info!(
                            timezone = %timezone,
                            is_active = is_active,
                            "Night mode state updated due to timezone change"
                        );
                    }
                }
                Err(err) => {
                    info!(error = %err, "Timezone receiver closed, stopping night mode update loop");
                    break;
                }
            }
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.is_active_sender.subscribe()
    }

    pub(crate) async fn config(&self) -> NightModeConfig {
        self.config_handle.read().await.night_mode()
    }

    pub(crate) async fn override_state(&self) -> NightModeOverride {
        *self.override_state.read().await
    }

    async fn calculate_is_active(&self) -> bool {
        let config = self.config().await;
        let override_state = self.override_state().await;
        let timezone = self.timezone_receiver.borrow().clone();

        match override_state {
            NightModeOverride::ForceActive => true,
            NightModeOverride::ForceInactive => false,
            NightModeOverride::None => config.is_active(&timezone),
        }
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

        info!(
            enabled = enabled,
            is_active = is_active,
            "Night mode enabled state updated"
        );

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

        info!(
            from = %from,
            to = %to,
            is_active = is_active,
            "Night mode interval updated"
        );

        Ok(())
    }

    pub(crate) async fn set_brightness(&self, value_pct: u8) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_brightness(value_pct);
        config_handle.save().await?;

        info!(brightness_pct = value_pct, "Night mode brightness updated");

        Ok(())
    }

    pub(crate) async fn set_sound_volume(&self, sound_volume_pct: u8) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_sound_volume(sound_volume_pct);
        config_handle.save().await?;

        info!(
            volume_pct = sound_volume_pct,
            "Night mode sound volume updated"
        );

        Ok(())
    }

    pub(crate) async fn set_led_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_led_enabled(enabled);
        config_handle.save().await?;

        info!(led_enabled = enabled, "Night mode LED enabled updated");

        Ok(())
    }

    pub(crate) async fn toggle(&self) -> anyhow::Result<()> {
        let config = self.config().await;
        let is_currently_active = self.calculate_is_active().await;
        let timezone = self.timezone_receiver.borrow().clone();
        let now = chrono::Local::now().with_timezone(timezone.chrono()).time();
        let now_in_scheduled_range = NightModeConfig::is_time_in_range(config.from, config.to, now);

        match (config.enabled, is_currently_active, now_in_scheduled_range) {
            // Case 1: Config disabled, turning ON
            (false, _, _) => {
                // Enable config + force active
                self.set_enabled(true).await?;
                *self.override_state.write().await = NightModeOverride::ForceActive;
            }

            // Case 2: Currently active during scheduled hours, turning OFF
            (true, true, true) => {
                // Force inactive during scheduled hours
                // The scheduled 'from' job will clear this override
                *self.override_state.write().await = NightModeOverride::ForceInactive;
            }

            // Case 3: Was force active outside hours, turning OFF
            (true, true, false) => {
                // Clear force active override
                *self.override_state.write().await = NightModeOverride::None;
            }

            // Case 4: Currently inactive during scheduled hours (was forced off), turning ON
            (true, false, true) => {
                // Clear force inactive override
                *self.override_state.write().await = NightModeOverride::None;
            }

            // Case 5: Outside hours and inactive, turning ON
            (true, false, false) => {
                // Force active outside hours
                // The scheduled 'to' job will clear this override
                *self.override_state.write().await = NightModeOverride::ForceActive;
            }
        }

        // Recalculate and update is_active
        let new_is_active = self.calculate_is_active().await;
        self.is_active_sender.send_replace(new_is_active);

        Ok(())
    }

    async fn schedule_jobs(&self, night_mode: &NightModeConfig) -> anyhow::Result<()> {
        debug!("Cancelling scheduled night mode jobs");

        self.scheduler
            .cancel_jobs(Self::NIGHT_MODE_SCHEDULER_SOURCE.to_owned())
            .await;

        if !night_mode.enabled {
            debug!("Night mode disabled, no jobs scheduled");
            return Ok(());
        }

        debug!(from = %night_mode.from, to = %night_mode.to, "Scheduling night mode jobs");

        let from_cron = bmc_scheduler::cron::from_naive_time(night_mode.from)?;
        let to_cron = bmc_scheduler::cron::from_naive_time(night_mode.to)?;

        self.scheduler
            .schedule(
                Schedule::Cron(from_cron),
                Task::Async({
                    let is_active_sender = self.is_active_sender.clone();
                    let from_time = night_mode.from;
                    let override_state = self.override_state.clone();
                    Box::new(move || {
                        let is_active_sender = is_active_sender.clone();
                        let from_time = from_time;
                        let override_state = override_state.clone();
                        Box::pin(async move {
                            // Clear override when scheduled start time hits
                            *override_state.write().await = NightModeOverride::None;
                            is_active_sender.send_replace(true);
                            info!(time = %from_time, "Night mode activated by schedule");
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
                    let to_time = night_mode.to;
                    let override_state = self.override_state.clone();
                    Box::new(move || {
                        let is_active_sender = is_active_sender.clone();
                        let to_time = to_time;
                        let override_state = override_state.clone();
                        Box::pin(async move {
                            // Clear override when scheduled end time hits
                            *override_state.write().await = NightModeOverride::None;
                            is_active_sender.send_replace(false);
                            info!(time = %to_time, "Night mode deactivated by schedule");
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
