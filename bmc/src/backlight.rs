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

pub(crate) const MIN_BRIGHTNESS_PCT: u8 = 10;

pub use bmc_platform::backlight::DisplayBacklightDriver;

use std::sync::Arc;

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

    /// Whether the panel is showing the user anything.
    ///
    /// Delegates to [`DisplayBacklightDriver::is_visible`], the same predicate
    /// the compositor's [`bmc_platform::backlight::ScreenVisibility`] port
    /// answers with, so the two ends cannot disagree about a dark panel.
    pub(crate) async fn is_visible(&self) -> anyhow::Result<bool> {
        self.backlight_driver.lock().await.is_visible()
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
