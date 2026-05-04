// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;
use std::time::Duration;

use crate::backlight::DisplayBacklightDriver;
use crate::{
    backlight::DisplayBacklightController,
    bootloader_config::BootloaderConfig,
    config::{ConfigHandle, NightModeConfig},
    led::LedState,
    manager::BmcManager,
    night_mode::NightModeController,
    sound::SoundController,
};
use bmc_scheduler::JobScheduler;
use bmc_shared_time::time::Timezone;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};
use tokio::sync::{Mutex, Notify, RwLock, broadcast, watch};
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
const MIN_SCREEN_OFF_TIMEOUT_SECS: u32 = 5;

#[derive(Clone, Debug)]
pub(crate) struct SystemManager<T: DisplayBacklightDriver> {
    night_mode_controller: NightModeController,
    backlight_controller: DisplayBacklightController<T>,
    brightness_modified: Arc<Notify>,
    sound_controller: SoundController,
    sound_volume_modified: Arc<Notify>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    led_state_modified: Arc<Notify>,
    screen_activity: Arc<Notify>,
    screen_woken_tx: broadcast::Sender<()>,
}

impl<T: DisplayBacklightDriver> SystemManager<T> {
    #[expect(clippy::too_many_arguments)]
    pub(crate) async fn init<M: BmcManager>(
        config_handle: Arc<RwLock<ConfigHandle>>,
        timezone_receiver: watch::Receiver<Timezone>,
        backlight_driver: Arc<Mutex<T>>,
        scheduler: JobScheduler,
        sound_controller: SoundController,
        led_state_sender: watch::Sender<LedState>,
        manager: Arc<M>,
        screen_activity: Arc<Notify>,
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
        ));

        let sound_volume_modified = Arc::new(Notify::new());

        tokio::spawn(Self::set_current_sound_volume(
            sound_controller.clone(),
            night_mode_controller.clone(),
            sound_volume_modified.clone(),
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

        let timeout_changed = config_handle
            .read()
            .await
            .subscribe_screen_off_timeout_change();
        let (screen_woken_tx, _screen_woken_rx) = broadcast::channel(8);

        tokio::spawn(Self::run_screen_auto_off(
            backlight_controller.clone(),
            night_mode_controller.clone(),
            brightness_modified.clone(),
            screen_activity.clone(),
            timeout_changed,
            screen_woken_tx.clone(),
        ));

        Self {
            night_mode_controller,
            backlight_controller,
            brightness_modified,
            sound_controller,
            sound_volume_modified,
            config_handle,
            led_state_modified,
            screen_activity,
            screen_woken_tx,
        }
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
                warn!(
                    error = %err,
                    brightness = brightness,
                    night_mode_active = night_mode_is_active,
                    "Failed to set display brightness"
                );
            }

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

    /// Query whether the hardware backlight is currently off.
    /// Falls back to `false` (assume on) if the driver query fails.
    async fn is_backlight_off(backlight_controller: &DisplayBacklightController<T>) -> bool {
        match backlight_controller.is_on().await {
            Ok(on) => !on,
            Err(err) => {
                warn!(error = %err, "Failed to query backlight state, assuming on");
                false
            }
        }
    }

    async fn run_screen_auto_off(
        backlight_controller: DisplayBacklightController<T>,
        night_mode_controller: NightModeController,
        brightness_modified: Arc<Notify>,
        screen_activity: Arc<Notify>,
        mut timeout_changed: broadcast::Receiver<Option<u32>>,
        screen_woken_tx: broadcast::Sender<()>,
    ) {
        let mut night_mode_receiver = night_mode_controller.subscribe();

        loop {
            let night_mode_active = *night_mode_receiver.borrow_and_update();

            // If night mode is not active, ensure screen is on and wait
            if !night_mode_active {
                if Self::is_backlight_off(&backlight_controller).await {
                    Self::wake_screen(
                        &backlight_controller,
                        &brightness_modified,
                        &screen_woken_tx,
                    )
                    .await;
                }

                if night_mode_receiver.changed().await.is_err() {
                    break;
                }
                continue;
            }

            let config = night_mode_controller.config().await;
            let timeout_secs = config.screen_off_timeout_secs.unwrap_or(0);

            // No timeout configured — ensure screen is on and wait for state changes
            if timeout_secs == 0 {
                if Self::is_backlight_off(&backlight_controller).await {
                    Self::wake_screen(
                        &backlight_controller,
                        &brightness_modified,
                        &screen_woken_tx,
                    )
                    .await;
                }

                tokio::select! {
                    biased;
                    result = night_mode_receiver.changed() => {
                        if result.is_err() { break; }
                    },
                    () = screen_activity.notified() => {},
                    Ok(_) = timeout_changed.recv() => {},
                }
                continue;
            }

            let clamped = timeout_secs.max(MIN_SCREEN_OFF_TIMEOUT_SECS);
            let timeout = std::time::Duration::from_secs(u64::from(clamped));

            tokio::select! {
                biased;
                result = night_mode_receiver.changed() => {
                    if result.is_err() { break; }
                    // Night mode state changed — loop back to handle it
                },
                () = screen_activity.notified() => {
                    // User activity detected — wake screen if off
                    if Self::is_backlight_off(&backlight_controller).await {
                        Self::wake_screen(&backlight_controller, &brightness_modified, &screen_woken_tx).await;
                        info!("Screen woken by user activity");
                    }
                    // Timer restarts on next loop iteration
                },
                Ok(_) = timeout_changed.recv() => {
                    // Config changed — loop back to re-read it
                },
                () = tokio::time::sleep(timeout) => {
                    // Timeout expired — turn off screen.
                    // Sequence: brightness→0, then power off pin
                    // to avoid a visible flash from the kernel backlight driver.
                    if !Self::is_backlight_off(&backlight_controller).await {
                        if let Err(err) = backlight_controller.set_display_brightness(0).await {
                            warn!(error = %err, "Failed to zero brightness for auto-off");
                        }
                        if let Err(err) = backlight_controller.turn_off().await {
                            warn!(error = %err, "Failed to turn off backlight for auto-off");
                        }
                        info!(timeout_secs = clamped, "Screen auto-off activated");
                    }
                    // Loop back — will sleep again or wake on activity/config change
                },
            }
        }
    }

    /// Wake the screen from auto-off: power on backlight and restore brightness.
    async fn wake_screen(
        backlight_controller: &DisplayBacklightController<T>,
        brightness_modified: &Arc<Notify>,
        screen_woken_tx: &broadcast::Sender<()>,
    ) {
        // Sequence: power on → restore brightness
        // (reverse of turn-off to avoid flash)
        if let Err(err) = backlight_controller.turn_on().await {
            warn!(error = %err, "Failed to turn on backlight on wake");
        }
        brightness_modified.notify_waiters();
        let _ = screen_woken_tx.send(());
    }

    #[expect(dead_code, reason = "reserved for the display-overlay channel")]
    pub(crate) fn notify_screen_activity(&self) {
        self.screen_activity.notify_waiters();
    }

    pub(crate) fn subscribe_screen_woken(&self) -> broadcast::Receiver<()> {
        self.screen_woken_tx.subscribe()
    }

    pub(crate) async fn set_night_mode_screen_off_timeout(
        &self,
        timeout: Option<u32>,
    ) -> anyhow::Result<()> {
        self.night_mode_controller
            .set_screen_off_timeout(timeout)
            .await
    }

    #[expect(dead_code, reason = "reserved for the display-overlay channel")]
    pub(crate) fn subscribe_night_mode(&self) -> watch::Receiver<bool> {
        self.night_mode_controller.subscribe()
    }

    #[expect(dead_code, reason = "reserved for the display-overlay channel")]
    pub(crate) async fn night_mode_config(&self) -> crate::config::NightModeConfig {
        self.night_mode_controller.config().await
    }

    #[expect(dead_code, reason = "reserved for the display-overlay channel")]
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

    #[expect(dead_code, reason = "reserved for the display-overlay channel")]
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
    /// Uses the current time to determine the correct UTC offset (accounting for DST).
    /// If the local_time has already passed today, uses tomorrow's date instead,
    /// but only if it would result in a different (smaller) UTC value - this ensures
    /// U-Boot interprets the time correctly.
    fn local_time_to_utc_minutes(local_time: NaiveTime, timezone: &Timezone) -> u16 {
        Self::local_time_to_utc_minutes_at(local_time, timezone, Utc::now().naive_utc())
    }

    fn local_time_to_utc_minutes_at(
        local_time: NaiveTime,
        timezone: &Timezone,
        now_utc: NaiveDateTime,
    ) -> u16 {
        // Convert current UTC time to local time in the given timezone
        let now_local = timezone.chrono().from_utc_datetime(&now_utc).naive_local();
        let today = now_local.date();

        // Check if the target time has already passed today
        let use_tomorrow = local_time < now_local.time();

        let target_date = if use_tomorrow {
            today + chrono::Duration::days(1)
        } else {
            today
        };

        // Calculate UTC minutes for the target date
        let utc_minutes_target =
            Self::convert_local_time_to_utc_minutes(local_time, timezone, target_date);

        // If using tomorrow, verify it makes sense for U-Boot:
        // We can only use tomorrow's value if the current UTC time has already passed
        // tomorrow's target UTC time. Otherwise, U-Boot might misinterpret the time.
        if use_tomorrow {
            let utc_minutes_today =
                Self::convert_local_time_to_utc_minutes(local_time, timezone, today);
            let now_utc_minutes = now_utc.time().hour() * 60 + now_utc.time().minute();

            // Use tomorrow's value only if current UTC time > tomorrow's target UTC time
            // This ensures U-Boot won't think we're still before the target time
            if now_utc_minutes > u32::from(utc_minutes_target) {
                return utc_minutes_target;
            }
            return utc_minutes_today;
        }

        utc_minutes_target
    }

    /// Convert a local time on a specific date to UTC minutes since midnight.
    #[expect(clippy::cast_possible_truncation)]
    fn convert_local_time_to_utc_minutes(
        local_time: NaiveTime,
        timezone: &Timezone,
        date: NaiveDate,
    ) -> u16 {
        use chrono::FixedOffset;
        use chrono_tz::OffsetComponents;

        let local_datetime = NaiveDate::and_time(&date, local_time);
        let tz = timezone.chrono();
        let mapping = tz.from_local_datetime(&local_datetime);

        // Try unambiguous conversion first
        let utc_datetime = if let Some(dt) = mapping.single() {
            dt.with_timezone(&Utc)
        } else if let Some(dt) = mapping.earliest().or_else(|| mapping.latest()) {
            // Ambiguous time (DST fall-back): use earliest or latest
            dt.with_timezone(&Utc)
        } else {
            // Gap time (DST spring-forward): use base (standard) offset
            let tz_offset = tz.offset_from_utc_datetime(&local_datetime);
            let base_offset_secs = tz_offset.base_utc_offset().num_seconds() as i32;
            FixedOffset::east_opt(base_offset_secs)
                .and_then(|offset| offset.from_local_datetime(&local_datetime).single())
                .map_or_else(
                    || Utc.from_utc_datetime(&local_datetime),
                    |dt| dt.with_timezone(&Utc),
                )
        };

        let utc_time = utc_datetime.time();
        // Hours (0-23) and minutes (0-59) always fit in u16
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backlight::DisplayBacklightDriver;
    use std::str::FromStr;

    #[derive(Debug, Clone)]
    struct DummyBacklightDriver;

    impl DisplayBacklightDriver for DummyBacklightDriver {
        fn init(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn change_state(&self, _enabled: bool) -> anyhow::Result<()> {
            Ok(())
        }
        fn state(&self) -> anyhow::Result<bool> {
            Ok(true)
        }
        fn brightness(&self) -> anyhow::Result<u8> {
            Ok(100)
        }
        fn max_brightness(&self) -> u8 {
            255
        }
        fn set_brightness(&self, _value: u8) -> anyhow::Result<()> {
            Ok(())
        }
    }

    type TestSystemManager = SystemManager<DummyBacklightDriver>;

    /// Helper to create a UTC NaiveDateTime from local time in Prague
    fn prague_local_to_utc(date: NaiveDate, time: NaiveTime, timezone: &Timezone) -> NaiveDateTime {
        let local_dt = NaiveDate::and_time(&date, time);
        timezone
            .chrono()
            .from_local_datetime(&local_dt)
            .single()
            .map_or(local_dt, |dt| dt.naive_utc()) // fallback for gap times
    }

    #[tokio::test]
    async fn test_local_time_to_utc_minutes_prague_midnight() {
        // Europe/Prague on 2026-02-02 is UTC+1 (standard time, no DST)
        let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
        let date = NaiveDate::from_ymd_opt(2026, 2, 2).expect("BUG: invalid date");
        // Current time is 00:00 local (23:00 UTC previous day)
        let now_utc = prague_local_to_utc(
            date,
            NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time"),
            &timezone,
        );
        let local_time = NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time");

        let utc_minutes =
            TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

        // Midnight in Prague (UTC+1) = 23:00 previous day in UTC = 23*60 = 1380 minutes
        assert_eq!(utc_minutes, 23 * 60);
    }

    #[tokio::test]
    async fn test_local_time_to_utc_minutes_prague_late_evening() {
        let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
        let date = NaiveDate::from_ymd_opt(2026, 2, 2).expect("BUG: invalid date");
        // Current time is 00:00 local so 23:45 hasn't passed
        let now_utc = prague_local_to_utc(
            date,
            NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time"),
            &timezone,
        );
        let local_time = NaiveTime::from_hms_opt(23, 45, 0).expect("BUG: invalid time");

        let utc_minutes =
            TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

        // 23:45 in Prague (UTC+1) = 22:45 UTC = 22*60 + 45 = 1365 minutes
        assert_eq!(utc_minutes, 22 * 60 + 45);
    }

    #[tokio::test]
    async fn test_local_time_to_utc_minutes_prague_summer_midnight() {
        // Europe/Prague on 2026-07-02 is UTC+2 (DST)
        let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
        let date = NaiveDate::from_ymd_opt(2026, 7, 2).expect("BUG: invalid date");
        let now_utc = prague_local_to_utc(
            date,
            NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time"),
            &timezone,
        );
        let local_time = NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time");

        let utc_minutes =
            TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

        // Midnight in Prague (UTC+2) = 22:00 previous day in UTC = 22*60 = 1320 minutes
        assert_eq!(utc_minutes, 22 * 60);
    }

    #[tokio::test]
    async fn test_local_time_to_utc_minutes_prague_summer_late_evening() {
        let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
        let date = NaiveDate::from_ymd_opt(2026, 7, 2).expect("BUG: invalid date");
        let now_utc = prague_local_to_utc(
            date,
            NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time"),
            &timezone,
        );
        let local_time = NaiveTime::from_hms_opt(23, 45, 0).expect("BUG: invalid time");

        let utc_minutes =
            TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

        // 23:45 in Prague (UTC+2) = 21:45 UTC = 21*60 + 45 = 1305 minutes
        assert_eq!(utc_minutes, 21 * 60 + 45);
    }

    #[tokio::test]
    async fn test_local_time_to_utc_minutes_prague_dst_spring_nonexistent_time() {
        // On 2026-03-29, clocks jump from 02:00 to 03:00 (CET -> CEST)
        // 02:30 doesn't exist that day - it's in the "gap"
        let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
        let date = NaiveDate::from_ymd_opt(2026, 3, 29).expect("BUG: invalid date");
        // Use 00:00 local which is still valid (before the gap)
        let now_utc = prague_local_to_utc(
            date,
            NaiveTime::from_hms_opt(0, 0, 0).expect("BUG: invalid time"),
            &timezone,
        );
        let local_time = NaiveTime::from_hms_opt(2, 30, 0).expect("BUG: invalid time");

        let utc_minutes =
            TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

        // Since 02:30 doesn't exist, we fall back to using the base (standard) offset.
        // Prague's base offset is UTC+1 (CET).
        // 02:30 with UTC+1 = 01:30 UTC = 1*60 + 30 = 90 minutes
        assert_eq!(utc_minutes, 60 + 30);
    }

    #[tokio::test]
    async fn test_local_time_to_utc_minutes_prague_dst_spring_same_day() {
        // Scenario: It's Saturday 2026-03-28 at 06:00 local (before DST switch).
        // Night mode ends at 06:30. Since 06:30 hasn't passed yet today,
        // we should use today's offset (UTC+1).
        let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
        let date = NaiveDate::from_ymd_opt(2026, 3, 28).expect("BUG: invalid date");
        // 06:00 local on 2026-03-28 = 05:00 UTC (Prague is UTC+1)
        let now_utc = prague_local_to_utc(
            date,
            NaiveTime::from_hms_opt(6, 0, 0).expect("BUG: invalid time"),
            &timezone,
        );
        let local_time = NaiveTime::from_hms_opt(6, 30, 0).expect("BUG: invalid time");

        let utc_minutes =
            TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

        // 06:30 hasn't passed yet (it's 06:00 now), so use today (2026-03-28).
        // On 2026-03-28, Prague is UTC+1 (CET, before DST switch).
        // 06:30 CET = 05:30 UTC = 5*60 + 30 = 330 minutes
        assert_eq!(utc_minutes, 5 * 60 + 30);
    }

    #[tokio::test]
    async fn test_local_time_to_utc_minutes_prague_dst_spring_next_day() {
        // Scenario: It's Saturday 2026-03-28 at 07:00 local (before DST switch).
        // Night mode ends at 06:30. Since 06:30 has already passed today,
        // the next 06:30 will be on Sunday 2026-03-29, after the DST switch.
        // So 06:30 on Sunday should use UTC+2, not UTC+1.
        let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
        let date = NaiveDate::from_ymd_opt(2026, 3, 28).expect("BUG: invalid date");
        // 07:00 local on 2026-03-28 = 06:00 UTC (Prague is still UTC+1)
        let now_utc = prague_local_to_utc(
            date,
            NaiveTime::from_hms_opt(7, 0, 0).expect("BUG: invalid time"),
            &timezone,
        );
        let local_time = NaiveTime::from_hms_opt(6, 30, 0).expect("BUG: invalid time");

        let utc_minutes =
            TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

        // 06:30 has already passed today (it's 07:00 now), so use tomorrow (2026-03-29).
        // On 2026-03-29, Prague is UTC+2 (CEST after DST switch).
        // 06:30 CEST = 04:30 UTC = 4*60 + 30 = 270 minutes
        assert_eq!(utc_minutes, 4 * 60 + 30);
    }

    #[tokio::test]
    async fn test_local_time_to_utc_minutes_prague_dst_fall_back_use_today() {
        // Scenario: It's Saturday 2026-10-24 at 07:00 local (still DST, UTC+2).
        // Night mode ends at 06:30. Since 06:30 has passed today,
        // normally we'd use tomorrow (Sunday 2026-10-25, after fall-back, UTC+1).
        //
        // However, using tomorrow would cause a problem:
        // - Today 06:30 CEST = 04:30 UTC (270 min)
        // - Tomorrow 06:30 CET = 05:30 UTC (330 min)
        //
        // If we returned 330 min and U-Boot checks now (05:00 UTC = 300 min), it would think
        // night mode is still on (300 < 330). So we must use today's value.
        let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
        let date = NaiveDate::from_ymd_opt(2026, 10, 24).expect("BUG: invalid date");
        // 07:00 local on 2026-10-24 = 05:00 UTC (Prague is still UTC+2)
        let now_utc = prague_local_to_utc(
            date,
            NaiveTime::from_hms_opt(7, 0, 0).expect("BUG: invalid time"),
            &timezone,
        );
        let local_time = NaiveTime::from_hms_opt(6, 30, 0).expect("BUG: invalid time");

        let utc_minutes =
            TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

        // Even though 06:30 has passed, we use today's value to avoid U-Boot misinterpretation.
        // 06:30 CEST = 04:30 UTC = 4*60 + 30 = 270 minutes
        assert_eq!(utc_minutes, 4 * 60 + 30);
    }

    #[tokio::test]
    async fn test_local_time_to_utc_minutes_prague_dst_fall_back_use_tomorrow() {
        // Scenario: It's Saturday 2026-10-24 at 08:00 local (still DST, UTC+2).
        // Night mode ends at 06:30. Since 06:30 has passed today,
        // we should use tomorrow (Sunday 2026-10-25, after fall-back, UTC+1).
        //
        // Values:
        // - Today 06:30 CEST = 04:30 UTC (270 min)
        // - Tomorrow 06:30 CET = 05:30 UTC (330 min)
        // - Now 08:00 CEST = 06:00 UTC (360 min)
        //
        // Since now (360 min) > tomorrow's value (330 min), it's safe to use tomorrow.
        // U-Boot will correctly see that night mode has ended.
        let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
        let date = NaiveDate::from_ymd_opt(2026, 10, 24).expect("BUG: invalid date");
        // 08:00 local on 2026-10-24 = 06:00 UTC (Prague is still UTC+2)
        let now_utc = prague_local_to_utc(
            date,
            NaiveTime::from_hms_opt(8, 0, 0).expect("BUG: invalid time"),
            &timezone,
        );
        let local_time = NaiveTime::from_hms_opt(6, 30, 0).expect("BUG: invalid time");

        let utc_minutes =
            TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

        // Now it's safe to use tomorrow's value.
        // 06:30 CET = 05:30 UTC = 5*60 + 30 = 330 minutes
        assert_eq!(utc_minutes, 5 * 60 + 30);
    }

    #[tokio::test]
    async fn test_local_time_to_utc_minutes_prague_dst_fall_back_from_time() {
        // Scenario: It's Saturday 2026-10-24 at 23:00 local (still DST, UTC+2).
        // Night mode starts at 22:30. Since 22:30 has passed today,
        // normally we'd consider tomorrow.
        //
        // Values:
        // - Today 22:30 CEST = 20:30 UTC (1230 min)
        // - Tomorrow 22:30 CET = 21:30 UTC (1290 min)
        // - Now 23:00 CEST = 21:00 UTC (1260 min)
        //
        // Since now (1230 min) < tomorrow's value (1290 min), we use today's value.
        // This is correct: at 21:00 UTC, night mode should be ON (past 20:30 UTC).
        // If we used 1290, U-Boot would think night mode hasn't started yet.
        let timezone = Timezone::from_str("Europe/Prague").expect("BUG: invalid timezone");
        let date = NaiveDate::from_ymd_opt(2026, 10, 24).expect("BUG: invalid date");
        // 23:00 local on 2026-10-24 = 21:00 UTC (Prague is still UTC+2)
        let now_utc = prague_local_to_utc(
            date,
            NaiveTime::from_hms_opt(23, 0, 0).expect("BUG: invalid time"),
            &timezone,
        );
        let local_time = NaiveTime::from_hms_opt(22, 30, 0).expect("BUG: invalid time");

        let utc_minutes =
            TestSystemManager::local_time_to_utc_minutes_at(local_time, &timezone, now_utc);

        // Use today's value to ensure U-Boot correctly sees night mode as ON.
        // 22:30 CEST = 20:30 UTC = 20*60 + 30 = 1230 minutes
        assert_eq!(utc_minutes, 20 * 60 + 30);
    }
}
