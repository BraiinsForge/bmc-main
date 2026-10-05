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

use std::sync::Arc;

use bmc_shared_time::time::Timezone;
use chrono::NaiveTime;
use tokio::sync::{RwLock, watch};
use tracing::info;

use crate::config::{ConfigHandle, NightModeConfig};
use crate::daily_window::{DailyWindow, DailyWindowWatch};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum NightModeOverride {
    None,          // Follow schedule
    ForceActive,   // User turned on outside hours - turn off at scheduled 'to'
    ForceInactive, // User turned off during hours - turn on at next scheduled 'from'
}

#[derive(Clone)]
pub(crate) struct NightModeController {
    config_handle: Arc<RwLock<ConfigHandle>>,
    window: DailyWindowWatch,
    scheduled: watch::Receiver<bool>,
    is_active_sender: watch::Sender<bool>,
    override_state: Arc<RwLock<NightModeOverride>>,
}

impl NightModeController {
    pub(crate) async fn init(
        config_handle: Arc<RwLock<ConfigHandle>>,
        timezone_receiver: watch::Receiver<Timezone>,
        clock_steps: watch::Receiver<u64>,
    ) -> Self {
        let window = DailyWindowWatch::start(timezone_receiver, clock_steps);
        Self::with_window(config_handle, window).await
    }

    async fn with_window(
        config_handle: Arc<RwLock<ConfigHandle>>,
        window: DailyWindowWatch,
    ) -> Self {
        let night_mode = config_handle.read().await.night_mode();
        window.set(window_of(&night_mode));
        let scheduled = window.subscribe();
        let is_active = *scheduled.borrow();
        let (is_active_sender, _) = watch::channel(is_active);

        let this = Self {
            config_handle,
            window,
            scheduled,
            is_active_sender,
            override_state: Arc::new(RwLock::new(NightModeOverride::None)),
        };
        tokio::spawn(this.clone().follow_schedule());

        info!(
            enabled = night_mode.enabled,
            is_active = is_active,
            "Night mode controller initialized"
        );

        this
    }

    /// Ends any manual override whenever the schedule turns night mode on or off.
    async fn follow_schedule(self) {
        let mut scheduled = self.scheduled.clone();
        while scheduled.changed().await.is_ok() {
            let is_active = *scheduled.borrow_and_update();
            *self.override_state.write().await = NightModeOverride::None;
            self.is_active_sender.send_replace(is_active);
            info!(is_active, "Night mode state changed by schedule");
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
        match self.override_state().await {
            NightModeOverride::ForceActive => true,
            NightModeOverride::ForceInactive => false,
            NightModeOverride::None => *self.scheduled.borrow(),
        }
    }

    pub(crate) async fn set_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_enabled(enabled);
        config_handle.save().await?;

        let night_mode = config_handle.night_mode();
        drop(config_handle);

        self.window.set(window_of(&night_mode));
        let is_active = *self.scheduled.borrow();
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

        self.window.set(window_of(&night_mode));
        let is_active = *self.scheduled.borrow();
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

    pub(crate) async fn set_screen_off_timeout(&self, timeout: Option<u32>) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_screen_off_timeout(timeout);
        config_handle.save().await?;

        info!(
            timeout_secs = ?timeout,
            "Night mode screen off timeout updated"
        );

        Ok(())
    }

    pub(crate) async fn toggle(&self) -> anyhow::Result<()> {
        let config = self.config().await;
        let is_currently_active = self.calculate_is_active().await;
        let now_in_scheduled_range = *self.scheduled.borrow();

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
}

fn window_of(night_mode: &NightModeConfig) -> Option<DailyWindow> {
    night_mode.enabled.then_some(DailyWindow {
        from: night_mode.from,
        to: night_mode.to,
    })
}

impl std::fmt::Debug for NightModeController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NightModeController").finish()
    }
}
