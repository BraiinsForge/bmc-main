// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;
use std::time::Duration;

use crate::{
    backlight::DisplayBacklightController,
    bootloader_config::BootloaderConfig,
    config::{ConfigHandle, NightModeConfig},
    led::LedState,
    manager::BmcManager,
    night_mode::NightModeController,
    sound::SoundController,
};
use bmc_display::display_controller::DisplayController;
use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_scheduler::JobScheduler;
use bmc_shared_time::time::Timezone;
use chrono::{NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use tokio::sync::{Mutex, Notify, RwLock, watch};
use tracing::{info, warn};

#[derive(Debug, Clone)]

pub(crate) struct DisplaySettings {
    pub(crate) brightness_pct: u8,
    pub(crate) night_mode_config: NightModeConfig,
}

#[derive(Debug)]
pub(crate) struct SoundSettings {
    pub(crate) volume: u8,
    pub(crate) volume_night_mode: u8,
}

#[derive(Debug)]
pub(crate) struct LedSettings {
    pub(crate) led_enabled: bool,
    pub(crate) led_enabled_night_mode: bool,
}

const BOOTLOADER_SYNC_INTERVAL: Duration = Duration::from_secs(60 * 60); // 1 hour
const BOOTLOADER_SYNC_DEBOUNCE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub(crate) struct SystemManager<T: DisplayBacklightDriver> {
    night_mode_controller: NightModeController,
    backlight_controller: DisplayBacklightController<T>,
    brightness_modified: Arc<Notify>,
    sound_controller: SoundController,
    sound_volume_modified: Arc<Notify>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    led_state_modified: Arc<Notify>,
}

impl<T: DisplayBacklightDriver> SystemManager<T> {
    pub(crate) async fn init<M: BmcManager>(
        config_handle: Arc<RwLock<ConfigHandle>>,
        timezone_receiver: watch::Receiver<Timezone>,
        backlight_driver: Arc<Mutex<T>>,
        scheduler: JobScheduler,
        display_controller: DisplayController,
        sound_controller: SoundController,
        led_state_sender: watch::Sender<LedState>,
        manager: Arc<M>,
    ) -> Self {
        let backlight_controller =
            DisplayBacklightController::new(config_handle.clone(), backlight_driver.clone());

        let night_mode_controller =
            NightModeController::init(config_handle.clone(), scheduler, timezone_receiver.clone())
                .await;

        let brightness_modified = Arc::new(Notify::new());

        tokio::spawn(Self::set_current_brightness(
            backlight_controller.clone(),
            night_mode_controller.clone(),
            brightness_modified.clone(),
            display_controller.clone(),
        ));

        tokio::spawn(Self::set_night_mode_flag_in_slint(
            display_controller.clone(),
            night_mode_controller.clone(),
        ));

        let sound_volume_modified = Arc::new(Notify::new());

        tokio::spawn(Self::set_current_sound_volume(
            sound_controller.clone(),
            night_mode_controller.clone(),
            sound_volume_modified.clone(),
            display_controller.clone(),
        ));

        let led_state_modified = Arc::new(Notify::new());

        tokio::spawn(Self::set_current_led_state(
            config_handle.clone(),
            night_mode_controller.clone(),
            led_state_sender.clone(),
            led_state_modified.clone(),
        ));

        tokio::spawn(Self::sync_bootloader_config_task(
            config_handle.clone(),
            timezone_receiver.clone(),
            backlight_driver,
            manager,
        ));

        Self {
            night_mode_controller,
            backlight_controller,
            brightness_modified,
            sound_controller,
            sound_volume_modified,
            config_handle,
            led_state_modified,
        }
    }

    async fn set_current_brightness(
        backlight_controller: DisplayBacklightController<T>,
        night_mode_controller: NightModeController,
        brightness_modified: Arc<Notify>,
        display_controller: DisplayController,
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
                warn!(
                    error = %err,
                    brightness = brightness,
                    night_mode_active = night_mode_is_active,
                    "Failed to set display brightness"
                );
            }

            // Update the BrightnessAdapter in Slint UI
            display_controller.set_brightness(brightness);

            tokio::select! {
                biased;
                result = night_mode_receiver.changed() => {
                    if let Err(err) = result {
                        info!(error = %err, "Night mode receiver closed, stopping brightness update loop");
                        break;
                    }
                },
                () = brightness_modified.notified() => {},
            }
        }
    }

    async fn set_current_sound_volume(
        sound_controller: SoundController,
        night_mode_controller: NightModeController,
        sound_volume_modified: Arc<Notify>,
        display_controller: DisplayController,
    ) {
        let mut night_mode_receiver = night_mode_controller.subscribe();
        loop {
            let night_mode_is_active = *night_mode_receiver.borrow_and_update();

            let sound_volume = if night_mode_is_active {
                night_mode_controller.config().await.sound_volume_pct
            } else {
                sound_controller.sound_volume().await
            };

            if let Err(err) = sound_controller.set_audio_sound_volume(sound_volume).await {
                warn!(
                    error = %err,
                    volume = sound_volume,
                    night_mode_active = night_mode_is_active,
                    "Failed to set audio sound volume"
                );
            }

            // Update the SoundAdapter in Slint
            display_controller.set_sound_volume(sound_volume);

            tokio::select! {
                biased;
                result = night_mode_receiver.changed() => {
                    if let Err(err) = result {
                        info!(error = %err, "Night mode receiver closed, stopping sound volume update loop");
                        break;
                    }
                },
                () = sound_volume_modified.notified() => {},
            }
        }
    }

    async fn set_current_led_state(
        config_handle: Arc<RwLock<ConfigHandle>>,
        night_mode_controller: NightModeController,
        led_state_sender: watch::Sender<LedState>,
        led_state_modified: Arc<Notify>,
    ) {
        let mut night_mode_receiver = night_mode_controller.subscribe();
        loop {
            let night_mode_is_active = *night_mode_receiver.borrow_and_update();

            let led_enabled = if night_mode_is_active {
                night_mode_controller.config().await.led_enabled
            } else {
                config_handle.read().await.led_enabled()
            };

            if let Err(err) = led_state_sender.send(LedState::from(led_enabled)) {
                warn!(
                    error = %err,
                    led_enabled = led_enabled,
                    night_mode_active = night_mode_is_active,
                    "Failed to send LED state"
                );
            }

            tokio::select! {
                biased;
                result = night_mode_receiver.changed() => {
                    if let Err(err) = result {
                        info!(error = %err, "Night mode receiver closed, stopping LED state update loop");
                        break;
                    }
                },
                () = led_state_modified.notified() => {},
            }
        }
    }

    async fn set_night_mode_flag_in_slint(
        display_controller: DisplayController,
        night_mode_controller: NightModeController,
    ) {
        let mut night_mode_receiver = night_mode_controller.subscribe();
        loop {
            let night_mode_is_active = *night_mode_receiver.borrow_and_update();

            if night_mode_is_active {
                display_controller.reset_cycler();
            }
            display_controller.set_night_mode(night_mode_is_active);

            if let Err(err) = night_mode_receiver.changed().await {
                info!(error = %err, "Night mode receiver closed, stopping display update loop");
                break;
            }
        }
    }

    pub(crate) fn subscribe_night_mode(&self) -> watch::Receiver<bool> {
        self.night_mode_controller.subscribe()
    }

    pub(crate) async fn night_mode_config(&self) -> crate::config::NightModeConfig {
        self.night_mode_controller.config().await
    }

    pub(crate) async fn toggle_night_mode(&self) -> anyhow::Result<()> {
        self.night_mode_controller.toggle().await
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

    pub(crate) fn is_night_mode_active(&self) -> bool {
        *self.night_mode_controller.subscribe().borrow()
    }

    pub(crate) async fn display_settings(&self) -> DisplaySettings {
        let brightness_pct = self.backlight_controller.brightness().await;
        let night_mode_config = self.night_mode_controller.config().await;

        DisplaySettings {
            brightness_pct,
            night_mode_config,
        }
    }

    pub(crate) async fn sound_settings(&self) -> SoundSettings {
        let volume = self.sound_controller.sound_volume().await;
        let volume_night_mode = self.night_mode_controller.config().await.sound_volume_pct;

        SoundSettings {
            volume,
            volume_night_mode,
        }
    }

    pub(crate) async fn set_sound_volume(&self, value: u8) -> anyhow::Result<()> {
        self.sound_controller.set_config_sound_volume(value).await?;
        self.sound_volume_modified.notify_waiters();

        Ok(())
    }

    pub(crate) async fn set_sound_volume_night_mode(&self, value: u8) -> anyhow::Result<()> {
        self.night_mode_controller.set_sound_volume(value).await?;
        self.sound_volume_modified.notify_waiters();

        Ok(())
    }

    pub(crate) async fn led_settings(&self) -> LedSettings {
        let led_enabled = self.config_handle.read().await.led_enabled();
        let led_enabled_night_mode = self.night_mode_controller.config().await.led_enabled;

        LedSettings {
            led_enabled,
            led_enabled_night_mode,
        }
    }

    pub(crate) async fn set_led_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        {
            let mut config_handle = self.config_handle.write().await;
            config_handle.set_led_enabled(enabled);
            config_handle.save().await?;
        }
        self.led_state_modified.notify_waiters();

        Ok(())
    }

    pub(crate) async fn set_led_enabled_night_mode(&self, enabled: bool) -> anyhow::Result<()> {
        self.night_mode_controller.set_led_enabled(enabled).await?;
        self.led_state_modified.notify_waiters();

        Ok(())
    }

    /// Convert local NaiveTime to UTC minutes since midnight.
    ///
    /// Uses today's date to determine the correct UTC offset (accounting for DST).
    fn local_time_to_utc_minutes(local_time: NaiveTime, timezone: &Timezone) -> u16 {
        // Use today's date to create a proper DateTime with timezone info
        let today = Utc::now().date_naive();

        // Create a local datetime and convert to UTC
        let local_datetime = NaiveDate::and_time(&today, local_time);
        let utc_datetime = timezone
            .chrono()
            .from_local_datetime(&local_datetime)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|| {
                // Handle ambiguous/invalid times (DST transitions)
                timezone
                    .chrono()
                    .from_local_datetime(&local_datetime)
                    .earliest()
                    .or_else(|| {
                        timezone
                            .chrono()
                            .from_local_datetime(&local_datetime)
                            .latest()
                    })
                    .unwrap_or_else(|| {
                        // Final fallback: interpret naive time as UTC, then convert to timezone
                        Utc.from_utc_datetime(&local_datetime)
                            .with_timezone(timezone.chrono())
                    })
                    .with_timezone(&Utc)
            });

        let utc_time = utc_datetime.time();
        let hours = utc_time.hour() as u16;
        let minutes = utc_time.minute() as u16;

        hours * 60 + minutes
    }

    /// Calculate BootloaderConfig from current config and timezone.
    async fn calculate_bootloader_config(
        config_handle: &RwLock<ConfigHandle>,
        timezone: &Timezone,
        backlight_driver: &Mutex<T>,
    ) -> BootloaderConfig {
        let config = config_handle.read().await;
        let night_mode = config.night_mode();
        let brightness_pct = config.brightness_pct();
        let led_enabled = config.led_enabled();
        drop(config);

        let driver = backlight_driver.lock().await;
        let screen_day = driver.pct_to_brightness(brightness_pct);
        drop(driver);

        let (night_from_utc_minutes, night_to_utc_minutes, led_night, screen_night) =
            if night_mode.enabled {
                let driver = backlight_driver.lock().await;
                let screen_night = driver.pct_to_brightness(night_mode.brightness_pct);
                drop(driver);

                (
                    Some(Self::local_time_to_utc_minutes(night_mode.from, timezone)),
                    Some(Self::local_time_to_utc_minutes(night_mode.to, timezone)),
                    Some(night_mode.led_enabled),
                    Some(screen_night),
                )
            } else {
                (None, None, None, None)
            };

        BootloaderConfig {
            night_from_utc_minutes,
            night_to_utc_minutes,
            led_day: led_enabled,
            led_night,
            screen_day,
            screen_night,
        }
    }

    /// Background task that periodically syncs bootloader configuration.
    async fn sync_bootloader_config_task<M: BmcManager>(
        config_handle: Arc<RwLock<ConfigHandle>>,
        timezone_receiver: watch::Receiver<Timezone>,
        backlight_driver: Arc<Mutex<T>>,
        manager: Arc<M>,
    ) {
        let mut interval = tokio::time::interval(BOOTLOADER_SYNC_INTERVAL);
        let mut timezone_receiver = timezone_receiver.clone();

        // Subscribe to config change notifications
        let mut night_mode_schedule_rx = config_handle
            .read()
            .await
            .subscribe_night_mode_schedule_change();
        let mut led_settings_rx = config_handle.read().await.subscribe_led_settings_change();
        let mut brightness_settings_rx = config_handle
            .read()
            .await
            .subscribe_brightness_settings_change();

        let debounce = tokio::time::sleep(BOOTLOADER_SYNC_DEBOUNCE);
        tokio::pin!(debounce);
        let mut pending_sync = false;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    pending_sync = true;
                    debounce.as_mut().reset(tokio::time::Instant::now());
                },
                Ok(()) = timezone_receiver.changed() => {
                    pending_sync = true;
                    debounce.as_mut().reset(tokio::time::Instant::now() + BOOTLOADER_SYNC_DEBOUNCE);
                },
                Ok(()) = night_mode_schedule_rx.recv() => {
                    pending_sync = true;
                    debounce.as_mut().reset(tokio::time::Instant::now() + BOOTLOADER_SYNC_DEBOUNCE);
                },
                Ok(()) = led_settings_rx.recv() => {
                    pending_sync = true;
                    debounce.as_mut().reset(tokio::time::Instant::now() + BOOTLOADER_SYNC_DEBOUNCE);
                },
                Ok(()) = brightness_settings_rx.recv() => {
                    pending_sync = true;
                    debounce.as_mut().reset(tokio::time::Instant::now() + BOOTLOADER_SYNC_DEBOUNCE);
                },
                () = &mut debounce, if pending_sync => {
                    pending_sync = false;

                    let timezone = timezone_receiver.borrow().clone();
                    let bootloader_config =
                        Self::calculate_bootloader_config(&config_handle, &timezone, &backlight_driver)
                            .await;

                    if let Err(err) = manager.sync_boot_environment(&bootloader_config).await {
                        warn!(error = %err, "Failed to sync bootloader configuration");
                    }
                },
            }
        }
    }
}
