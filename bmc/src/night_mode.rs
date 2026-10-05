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

use std::sync::{Arc, Mutex};

use bmc_shared_time::time::Timezone;
use chrono::NaiveTime;
use tokio::sync::{RwLock, watch};
use tracing::info;

use crate::config::{ConfigHandle, NightModeConfig};
use crate::daily_window::{DailyWindow, DailyWindowWatch};

#[cfg(test)]
mod tests;

/// A manual toggle against the schedule.
/// It lasts while the schedule disagrees with it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum NightModeOverride {
    None,
    ForceActive,
    ForceInactive,
}

impl NightModeOverride {
    /// Returns the effective night-mode state and the override still in force.
    fn settle(self, enabled: bool, scheduled: bool) -> (bool, Self) {
        match self {
            Self::ForceActive if enabled && !scheduled => (true, self),
            Self::ForceInactive if enabled && scheduled => (false, self),
            Self::None | Self::ForceActive | Self::ForceInactive => (scheduled, Self::None),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Reevaluation {
    KeepOverride,
    SetOverride(NightModeOverride),
}

#[derive(Clone)]
pub(crate) struct NightModeController {
    config_handle: Arc<RwLock<ConfigHandle>>,
    window: DailyWindowWatch,
    scheduled: watch::Receiver<bool>,
    is_active_sender: watch::Sender<bool>,
    override_state: Arc<Mutex<NightModeOverride>>,
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
        let (is_active_sender, _) = watch::channel(false);

        let this = Self {
            config_handle,
            scheduled: window.subscribe(),
            window,
            is_active_sender,
            override_state: Arc::new(Mutex::new(NightModeOverride::None)),
        };
        this.refresh("boot").await;
        tokio::spawn(this.clone().follow_schedule());

        info!(
            enabled = night_mode.enabled,
            is_active = *this.is_active_sender.borrow(),
            "Night mode controller initialized"
        );

        this
    }

    async fn follow_schedule(self) {
        let mut scheduled = self.scheduled.clone();
        while scheduled.changed().await.is_ok() {
            self.refresh("schedule").await;
        }
    }

    async fn refresh(&self, trigger: &'static str) {
        self.publish(trigger, Reevaluation::KeepOverride).await;
    }

    /// Settles the override against the schedule and publishes the result.
    async fn publish(&self, trigger: &'static str, reevaluation: Reevaluation) {
        // Held until the state is published,
        // so a schedule edit cannot be overwritten by a result computed from the old schedule.
        let config_handle = self.config_handle.read().await;
        let enabled = config_handle.night_mode().enabled;

        let mut override_state = self
            .override_state
            .lock()
            .expect("BUG: night mode override lock poisoned");
        // Read under the lock, so a publish racing a schedule edge cannot settle against the old level.
        let scheduled = *self.scheduled.borrow();
        let candidate = match reevaluation {
            Reevaluation::KeepOverride => *override_state,
            Reevaluation::SetOverride(new_override) => new_override,
        };
        let (is_active, remaining) = candidate.settle(enabled, scheduled);
        *override_state = remaining;
        let was_active = self.is_active_sender.send_replace(is_active);
        drop(override_state);
        drop(config_handle);

        if was_active != is_active {
            info!(
                trigger,
                is_active,
                override_state = ?remaining,
                "Night mode state changed"
            );
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.is_active_sender.subscribe()
    }

    pub(crate) async fn config(&self) -> NightModeConfig {
        self.config_handle.read().await.night_mode()
    }

    fn calculate_is_active(&self) -> bool {
        let override_state = *self
            .override_state
            .lock()
            .expect("BUG: night mode override lock poisoned");
        match override_state {
            NightModeOverride::ForceActive => true,
            NightModeOverride::ForceInactive => false,
            NightModeOverride::None => *self.scheduled.borrow(),
        }
    }

    pub(crate) async fn set_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_enabled(enabled);
        self.window.set(window_of(&config_handle.night_mode()));
        config_handle.save().await?;
        drop(config_handle);

        self.publish(
            "enabled change",
            Reevaluation::SetOverride(NightModeOverride::None),
        )
        .await;

        info!(enabled = enabled, "Night mode enabled state updated");

        Ok(())
    }

    pub(crate) async fn set_interval(&self, from: NaiveTime, to: NaiveTime) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_night_mode_interval(from, to);
        self.window.set(window_of(&config_handle.night_mode()));
        config_handle.save().await?;
        drop(config_handle);

        self.publish(
            "interval change",
            Reevaluation::SetOverride(NightModeOverride::None),
        )
        .await;

        info!(
            from = %from,
            to = %to,
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
        let is_currently_active = self.calculate_is_active();
        let now_in_scheduled_range = *self.scheduled.borrow();

        let new_override = match (config.enabled, is_currently_active, now_in_scheduled_range) {
            // Case 1: Config disabled, turning ON
            (false, _, _) => {
                // Enable config + force active
                self.set_enabled(true).await?;
                NightModeOverride::ForceActive
            }

            // Case 2: Currently active during scheduled hours, turning OFF
            (true, true, true) => NightModeOverride::ForceInactive,

            // Case 3: Was force active outside hours, turning OFF
            // Case 4: Currently inactive during scheduled hours (was forced off), turning ON
            (true, true, false) | (true, false, true) => NightModeOverride::None,

            // Case 5: Outside hours and inactive, turning ON
            (true, false, false) => NightModeOverride::ForceActive,
        };

        self.publish("toggle", Reevaluation::SetOverride(new_override))
            .await;

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
