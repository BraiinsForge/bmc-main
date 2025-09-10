// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;

use crate::{
    backlight::DisplayBacklightController,
    config::{ConfigHandle, NightModeConfig},
    night_mode::NightModeController,
};
use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_scheduler::JobScheduler;
use bmc_shared_time::time::Timezone;
use chrono::NaiveTime;
use tokio::sync::{Mutex, Notify, RwLock, watch};
use tracing::warn;

#[derive(Debug, Clone)]

pub(crate) struct DisplaySettings {
    pub(crate) brightness_pct: u8,
    pub(crate) night_mode_config: NightModeConfig,
}

#[derive(Clone, Debug)]
pub(crate) struct SystemManager<T: DisplayBacklightDriver> {
    night_mode_controller: NightModeController,
    backlight_controller: DisplayBacklightController<T>,
    brightness_modified: Arc<Notify>,
}

impl<T: DisplayBacklightDriver> SystemManager<T> {
    pub(crate) async fn init(
        config_handle: Arc<RwLock<ConfigHandle>>,
        timezone_receiver: watch::Receiver<Timezone>,
        backlight_driver: Arc<Mutex<T>>,
        scheduler: JobScheduler,
    ) -> anyhow::Result<Self> {
        let backlight_controller =
            DisplayBacklightController::new(config_handle.clone(), backlight_driver);

        let night_mode_controller =
            NightModeController::init(config_handle, scheduler, timezone_receiver).await?;

        let brightness_modified = Arc::new(Notify::new());

        tokio::spawn(Self::set_current_brightness(
            backlight_controller.clone(),
            night_mode_controller.clone(),
            brightness_modified.clone(),
        ));

        Ok(Self {
            night_mode_controller,
            backlight_controller,
            brightness_modified,
        })
    }

    async fn set_current_brightness(
        backlight_controller: DisplayBacklightController<T>,
        night_mode_controller: NightModeController,
        brightness_modified: Arc<Notify>,
    ) {
        let mut night_mode_receiver = night_mode_controller.subscribe();
        loop {
            let night_mode_is_active = *night_mode_receiver.borrow_and_update();

            let brightness = if night_mode_is_active {
                night_mode_controller.config().await.brightness_pct
            } else {
                backlight_controller.brightness().await
            };

            if let Err(err) = backlight_controller
                .set_display_brightness(brightness)
                .await
            {
                warn!(?err, "Failed to set display brightness");
            }

            tokio::select! {
                biased;
                result = night_mode_receiver.changed() => {
                    if result.is_err() {
                        break;
                    }
                },
                () = brightness_modified.notified() => {},
            }
        }
    }

    pub(crate) async fn set_night_mode_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        self.night_mode_controller.set_enabled(enabled).await
    }

    pub(crate) async fn set_night_mode_interval(
        &self,
        from: NaiveTime,
        to: NaiveTime,
    ) -> anyhow::Result<()> {
        self.night_mode_controller.set_interval(from, to).await
    }

    pub(crate) async fn set_night_mode_brightness(&self, value_pct: u8) -> anyhow::Result<()> {
        self.night_mode_controller.set_brightness(value_pct).await?;
        self.brightness_modified.notify_waiters();

        Ok(())
    }

    pub(crate) async fn set_brightness(&self, value_pct: u8) -> anyhow::Result<()> {
        self.backlight_controller
            .set_config_brightness(value_pct)
            .await?;
        self.brightness_modified.notify_waiters();

        Ok(())
    }

    pub(crate) async fn display_settings(&self) -> DisplaySettings {
        let brightness_pct = self.backlight_controller.brightness().await;
        let night_mode_config = self.night_mode_controller.config().await;

        DisplaySettings {
            brightness_pct,
            night_mode_config,
        }
    }
}
