// Copyright (C) 2025  Braiins Systems s.r.o.

// TODO: display refactor
#![allow(dead_code)]

pub(crate) const MIN_BRIGHTNESS_PCT: u8 = 10;

use std::fmt::Debug;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use tracing::info;

use crate::config::ConfigHandle;

/// Trait for controlling display backlight hardware.
pub trait DisplayBacklightDriver: Sync + Send + Clone + Debug + 'static {
    fn init(&mut self) -> anyhow::Result<()>;

    fn change_state(&self, enabled: bool) -> anyhow::Result<()>;

    fn state(&self) -> anyhow::Result<bool>;

    fn toggle_state(&mut self) -> anyhow::Result<()> {
        self.state().and_then(|state| self.change_state(!state))
    }

    fn turn_on(&self) -> anyhow::Result<()> {
        self.change_state(true)
    }

    fn turn_off(&self) -> anyhow::Result<()> {
        self.change_state(false)
    }

    fn brightness(&self) -> anyhow::Result<u8>;

    fn max_brightness(&self) -> u8;

    fn set_brightness(&self, value: u8) -> anyhow::Result<()>;

    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::integer_division)]
    fn pct_to_brightness(&self, percent: u8) -> u8 {
        ((u16::from(percent) * u16::from(self.max_brightness())) / 100) as u8
    }

    fn set_brightness_pct(&self, percent: u8) -> anyhow::Result<()> {
        self.set_brightness(self.pct_to_brightness(percent))
    }
}

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
