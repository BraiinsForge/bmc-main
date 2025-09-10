// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_scheduler::JobScheduler;
use bmc_shared_time::time::Timezone;
use chrono::NaiveTime;
use tokio::sync::{Mutex, RwLock, watch};
use tracing::{debug, warn};

use crate::{
    backlight::DisplayBacklightController,
    config::{ConfigHandle, NightModeConfig},
    night_mode::NightModeController,
};

#[derive(Debug, Clone)]

pub(crate) struct DisplaySettings {
    pub(crate) brightness_pct: u8,
    pub(crate) night_mode_config: NightModeConfig,
}

#[derive(Clone, Debug)]
pub(crate) struct SystemManager<T: DisplayBacklightDriver> {
    brightness_pct: Arc<AtomicU8>,
    brightness_night_mode_pct: Arc<AtomicU8>,
    timezone_receiver: watch::Receiver<Timezone>,
    night_mode_controller: NightModeController,
    backlight_controller: DisplayBacklightController<T>,
}

impl<T: DisplayBacklightDriver> SystemManager<T> {
    pub(crate) fn new(
        config_handle: Arc<RwLock<ConfigHandle>>,
        brightness: u8,
        brightness_night_mode: u8,
        timezone_receiver: watch::Receiver<Timezone>,
        backlight_driver: Arc<Mutex<T>>,
        scheduler: JobScheduler,
    ) -> Self {
        let backlight_controller =
            DisplayBacklightController::new(config_handle.clone(), backlight_driver);

        let brightness_pct = Arc::new(AtomicU8::new(brightness));

        let brightness_night_mode_pct = Arc::new(AtomicU8::new(brightness_night_mode));

        let activate_night_mode = {
            let backlight_controller = backlight_controller.clone();
            let brightness_night_mode_pct = brightness_night_mode_pct.clone();
            move || {
                Self::night_mode_task(
                    backlight_controller.clone(),
                    brightness_night_mode_pct.clone(),
                )
            }
        };

        let deactivate_night_mode = {
            let backlight_controller = backlight_controller.clone();
            let brightness_pct = brightness_pct.clone();
            move || Self::night_mode_task(backlight_controller.clone(), brightness_pct.clone())
        };

        let night_mode_controller = NightModeController::new(
            config_handle,
            scheduler,
            timezone_receiver.clone(),
            Box::new(activate_night_mode),
            Box::new(deactivate_night_mode),
        );

        Self {
            brightness_pct,
            brightness_night_mode_pct,
            timezone_receiver,
            night_mode_controller,
            backlight_controller,
        }
    }

    pub(crate) async fn init(&self) -> anyhow::Result<()> {
        let config = self.night_mode_controller.night_mode_config().await;
        self.set_current_brightness(config.clone()).await?;
        self.night_mode_controller.init(config).await?;

        tokio::spawn({
            let self_clone = self.clone();
            async move {
                self_clone
                    .run_timezone_listener(self_clone.timezone_receiver.clone())
                    .await;
            }
        });

        Ok(())
    }

    async fn run_timezone_listener(&self, mut receiver: watch::Receiver<Timezone>) {
        while let Ok(()) = receiver.changed().await {
            let night_mode = self.night_mode_controller.night_mode_config().await;

            if let Err(e) = self.set_current_brightness(night_mode).await {
                warn!("Failed to set brightness on timezone change. Err: {e}");
            }
        }
    }

    fn night_mode_task(
        backlight_controller: DisplayBacklightController<T>,
        brightness: Arc<AtomicU8>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let value = brightness.load(Ordering::Acquire);
            debug!("Executing night mode task, set brightness to {value}");
            let _ = backlight_controller.set_display_brightness(value).await;
        })
    }

    async fn set_current_brightness(&self, config: NightModeConfig) -> anyhow::Result<()> {
        let value = if self.night_mode_controller.is_night_mode(&config) {
            self.brightness_night_mode_pct.load(Ordering::Acquire)
        } else {
            self.brightness_pct.load(Ordering::Acquire)
        };

        self.backlight_controller
            .set_display_brightness(value)
            .await?;

        Ok(())
    }

    pub(crate) async fn set_night_mode_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let config = self
            .night_mode_controller
            .set_night_mode_enabled(enabled)
            .await?;

        self.set_current_brightness(config).await
    }

    pub(crate) async fn set_night_mode_interval(
        &self,
        from: NaiveTime,
        to: NaiveTime,
    ) -> anyhow::Result<()> {
        let config = self
            .night_mode_controller
            .set_night_mode_interval(from, to)
            .await?;

        self.set_current_brightness(config).await
    }

    pub(crate) async fn set_night_mode_brightness(&self, value_pct: u8) -> anyhow::Result<()> {
        let config = self
            .night_mode_controller
            .set_night_mode_brightness(value_pct)
            .await?;

        self.brightness_night_mode_pct
            .store(value_pct, Ordering::Release);

        self.set_current_brightness(config).await
    }

    pub(crate) async fn display_settings(&self) -> DisplaySettings {
        let brightness_pct = self.backlight_controller.brightness().await;
        let night_mode_config = self.night_mode_controller.night_mode_config().await;

        DisplaySettings {
            brightness_pct,
            night_mode_config,
        }
    }

    pub(crate) async fn set_brightness(&self, value_pct: u8) -> anyhow::Result<()> {
        self.backlight_controller
            .set_config_brightness(value_pct)
            .await?;

        let config = self.night_mode_controller.night_mode_config().await;

        self.brightness_pct.store(value_pct, Ordering::Release);

        self.set_current_brightness(config).await
    }
}
