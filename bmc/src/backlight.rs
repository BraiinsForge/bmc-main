// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;

use bmc_display::display_driver::DisplayBacklightDriver;
use tokio::sync::{Mutex, RwLock};
use tracing::info;

use crate::config::ConfigHandle;

#[derive(Clone, Debug)]
pub(crate) struct DisplayBacklightController<T: DisplayBacklightDriver> {
    config_handle: Arc<RwLock<ConfigHandle>>,
    backlight_driver: Arc<Mutex<T>>,
}

impl<T: DisplayBacklightDriver> DisplayBacklightController<T> {
    pub(crate) fn new(
        config_handle: Arc<RwLock<ConfigHandle>>,
        backlight_driver: Arc<Mutex<T>>,
    ) -> Self {
        Self {
            config_handle,
            backlight_driver,
        }
    }

    pub(crate) async fn brightness(&self) -> u8 {
        self.config_handle.read().await.brightness_pct()
    }

    pub(crate) async fn set_config_brightness(&self, value_pct: u8) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_brightness(value_pct);

        config_handle.save().await?;

        info!(
            brightness_pct = value_pct,
            "Display brightness configuration updated"
        );

        Ok(())
    }

    pub(crate) async fn set_display_brightness(&self, value_pct: u8) -> anyhow::Result<()> {
        self.backlight_driver
            .lock()
            .await
            .set_brightness_pct(value_pct)?;

        info!(
            brightness_pct = value_pct,
            "Display backlight brightness applied"
        );

        Ok(())
    }

    pub(crate) async fn is_on(&self) -> anyhow::Result<bool> {
        self.backlight_driver.lock().await.state()
    }

    pub(crate) async fn turn_on(&self) -> anyhow::Result<()> {
        self.backlight_driver.lock().await.turn_on()?;
        info!("Display backlight turned on");
        Ok(())
    }

    pub(crate) async fn turn_off(&self) -> anyhow::Result<()> {
        self.backlight_driver.lock().await.turn_off()?;
        info!("Display backlight turned off");
        Ok(())
    }
}
