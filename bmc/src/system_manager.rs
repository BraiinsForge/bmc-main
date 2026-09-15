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

const BOOTLOADER_SYNC_INTERVAL: Duration = Duration::from_hours(1); // 1 hour
const BOOTLOADER_SYNC_DEBOUNCE: Duration = Duration::from_secs(5);
const MIN_SCREEN_OFF_TIMEOUT_SECS: u32 = 5;

/// What the user last asked the screen to do.
///
/// A level rather than an edge, carried on a `watch`:
/// a writer can only be superseded by a later writer, never lost to a reader that was busy.
/// The auto-off loop reconciles toward it every pass instead of consuming it,
/// so a request needs no acknowledgement and re-applying one costs nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenRequest {
    /// The user interacted with the device, or an alarm refused a `Blank`;
    /// either way the panel belongs lit.
    Wake,
    /// The user asked for the panel dark, and it holds until they come back.
    Blank,
}

/// What `run_screen_auto_off` should do for the current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoOffMode {
    /// The backlight stays on: nothing asks for a blank,
    /// or an alarm is ringing and refuses one so the firing-alarm UI never sits on a dark panel.
    KeepOn,
    /// The panel is dark and held there: the loop neither wakes it nor arms the timer.
    HoldDark,
    /// Night mode is active with a timeout and nothing inhibits:
    /// arm the auto-off timer for this long.
    ArmTimer(Duration),
}

/// What the auto-off loop knows at the top of a pass.
#[derive(Debug, Clone, Copy)]
struct AutoOffInputs {
    night_mode_active: bool,
    alarm_ringing: bool,
    timeout_secs: Option<u32>,
    request: ScreenRequest,
    /// The loop's own blank rather than the user's,
    /// which is why it is not a [`ScreenRequest`]: it must not outlive what armed the timer.
    timer_blanked: bool,
}

/// Decide what the auto-off loop does this pass.
fn auto_off_decision(inputs: AutoOffInputs) -> AutoOffMode {
    let AutoOffInputs {
        night_mode_active,
        alarm_ringing,
        timeout_secs,
        request,
        timer_blanked,
    } = inputs;
    if alarm_ringing {
        return AutoOffMode::KeepOn;
    }
    if request == ScreenRequest::Blank {
        return AutoOffMode::HoldDark;
    }
    // A persisted zero predates the gRPC setter mapping it to `None`.
    let Some(timeout_secs) = timeout_secs.filter(|secs| *secs > 0) else {
        return AutoOffMode::KeepOn;
    };
    if !night_mode_active {
        return AutoOffMode::KeepOn;
    }
    if timer_blanked {
        return AutoOffMode::HoldDark;
    }
    let clamped = timeout_secs.max(MIN_SCREEN_OFF_TIMEOUT_SECS);
    AutoOffMode::ArmTimer(Duration::from_secs(u64::from(clamped)))
}

#[derive(Clone, Debug)]
pub(crate) struct SystemManager<T: DisplayBacklightDriver> {
    night_mode_controller: NightModeController,
    backlight_controller: DisplayBacklightController<T>,
    brightness_modified: Arc<Notify>,
    sound_controller: SoundController,
    sound_volume_modified: Arc<Notify>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    led_state_modified: Arc<Notify>,
    screen_blanked_tx: broadcast::Sender<()>,
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
        screen_request: watch::Sender<ScreenRequest>,
        alarm_ringing: watch::Receiver<bool>,
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

        // Capacity 1 is enough. The payload is "the panel just went dark", and a lagged
        // subscriber resets anyway.
        let (screen_blanked_tx, _) = broadcast::channel(1);

        let timeout_changed = config_handle
            .read()
            .await
            .subscribe_screen_off_timeout_change();
        tokio::spawn(Self::run_screen_auto_off(
            backlight_controller.clone(),
            night_mode_controller.clone(),
            brightness_modified.clone(),
            screen_request,
            timeout_changed,
            screen_blanked_tx.clone(),
            alarm_ringing,
        ));

        Self {
            night_mode_controller,
            backlight_controller,
            brightness_modified,
            sound_controller,
            sound_volume_modified,
            config_handle,
            led_state_modified,
            screen_blanked_tx,
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

    async fn is_screen_dark(backlight_controller: &DisplayBacklightController<T>) -> bool {
        match backlight_controller.is_visible().await {
            Ok(visible) => !visible,
            Err(err) => {
                warn!(error = %err, "Failed to query panel visibility, assuming visible");
                false
            }
        }
    }

    async fn run_screen_auto_off(
        backlight_controller: DisplayBacklightController<T>,
        night_mode_controller: NightModeController,
        brightness_modified: Arc<Notify>,
        screen_request: watch::Sender<ScreenRequest>,
        mut timeout_changed: broadcast::Receiver<Option<u32>>,
        screen_blanked_tx: broadcast::Sender<()>,
        mut alarm_ringing: watch::Receiver<bool>,
    ) {
        let mut night_mode_receiver = night_mode_controller.subscribe();
        let mut screen_request_rx = screen_request.subscribe();
        let mut timer_blanked = false;

        loop {
            let night_mode_active = *night_mode_receiver.borrow_and_update();
            let timeout_secs = night_mode_controller.config().await.screen_off_timeout_secs;
            // Sampled after the timeout read, the pass's one await, and next
            // to the request: the refusal below judges the request against
            // the alarm as it is now, not as it was before a config save.
            let alarm_ringing_now = *alarm_ringing.borrow_and_update();
            // `borrow`, not `borrow_and_update`: only the `changed()` arm
            // below marks a request seen, so a write landing anywhere in
            // this pass, the blank included, still gets a pass of its own.
            let request = *screen_request_rx.borrow();

            let mode = auto_off_decision(AutoOffInputs {
                night_mode_active,
                alarm_ringing: alarm_ringing_now,
                timeout_secs,
                request,
                timer_blanked,
            });

            if alarm_ringing_now && request == ScreenRequest::Blank {
                // Overwritten rather than remembered as handled:
                // a request left standing blanks the panel the moment the alarm stops.
                // Only the `Blank` seen above is overwritten; a `Wake` that
                // landed since already says what the refusal would.
                let refused = screen_request.send_if_modified(|request| {
                    if *request == ScreenRequest::Blank {
                        *request = ScreenRequest::Wake;
                        true
                    } else {
                        false
                    }
                });
                if refused {
                    info!("Alarm ringing: dropping the display-off request, the panel stays lit");
                }
            }

            match mode {
                AutoOffMode::KeepOn | AutoOffMode::ArmTimer(_) => {
                    // Whatever lit the panel ended the timer's blank; left standing,
                    // the flag would re-blank it the moment that cause went away.
                    timer_blanked = false;
                    if Self::wake_if_dark(&backlight_controller, &brightness_modified).await {
                        info!("Screen woken");
                    }
                }
                AutoOffMode::HoldDark => {
                    let cause = match request {
                        ScreenRequest::Blank => "button hold",
                        ScreenRequest::Wake => "auto-off timeout",
                    };
                    Self::blank_screen(&backlight_controller, &screen_blanked_tx, cause).await;
                }
            }

            let auto_off_timer = async {
                match mode {
                    AutoOffMode::ArmTimer(timeout) => tokio::time::sleep(timeout).await,
                    AutoOffMode::KeepOn | AutoOffMode::HoldDark => {
                        std::future::pending::<()>().await;
                    }
                }
            };

            tokio::select! {
                biased;
                result = night_mode_receiver.changed() => {
                    if result.is_err() { break; }
                },
                // A ring starting mid-countdown pre-empts the blank: the next
                // iteration decides `KeepOn` and wakes the screen.
                result = alarm_ringing.changed() => {
                    if result.is_err() { break; }
                },
                result = screen_request_rx.changed() => {
                    result.expect("BUG: the auto-off loop owns the screen-request sender");
                    // Every write ends the timer's blank, even `Wake` over
                    // `Wake`. The loop's own refusal write lands here too and
                    // costs one idle pass; a ringing alarm decides `KeepOn` anyway.
                    timer_blanked = false;
                },
                Ok(_) = timeout_changed.recv() => {},
                () = auto_off_timer => {
                    // A panel left visible gets another try at the next timeout;
                    // any touch meanwhile wakes it.
                    timer_blanked = Self::blank_screen(
                        &backlight_controller,
                        &screen_blanked_tx,
                        "auto-off timeout",
                    ).await;
                },
            }
        }
    }

    /// Blank the panel and report whether it is dark afterwards.
    /// An already dark panel is left alone.
    ///
    /// Brightness goes to zero before the power pin,
    /// so the kernel backlight driver does not flash.
    /// The blank is announced on `screen_blanked_tx` only once the panel is confirmed dark:
    /// the scene-0 reset is a visible jump on a screen that stayed lit.
    /// A failed write needs no rollback: the panel is still visible,
    /// so a retry is only ever a repeat of this call.
    async fn blank_screen(
        backlight_controller: &DisplayBacklightController<T>,
        screen_blanked_tx: &broadcast::Sender<()>,
        cause: &'static str,
    ) -> bool {
        if Self::is_screen_dark(backlight_controller).await {
            return true;
        }
        if let Err(err) = backlight_controller.set_display_brightness(0).await {
            warn!(error = %err, cause, "Failed to zero brightness for the blank");
        }
        if let Err(err) = backlight_controller.turn_off().await {
            warn!(error = %err, cause, "Failed to turn off the backlight for the blank");
        }
        if !Self::is_screen_dark(backlight_controller).await {
            warn!(cause, "Screen blank left the panel visible");
            return false;
        }
        if let Err(err) = screen_blanked_tx.send(()) {
            warn!(error = %err, "No screen-blanked subscriber; scene reset skipped");
        }
        info!(cause, "Screen blanked");
        true
    }

    /// Wake a dark panel and report whether it woke one, so a caller can log
    /// the wake without claiming one that never happened. A lit panel is left
    /// alone.
    ///
    /// Power comes on before brightness is restored, the reverse of the blank,
    /// so the kernel backlight driver never flashes. Scene 0 is already on
    /// glass, a `screen_blanked` subscriber reset to it when the panel went
    /// dark, so the panel can light at once without exposing a stale scene.
    ///
    /// Emits nothing about cycling. During night mode the night-mode listener
    /// holds it suspended and owns the transition back; outside night mode
    /// it was never suspended and carries on from the first scene.
    async fn wake_if_dark(
        backlight_controller: &DisplayBacklightController<T>,
        brightness_modified: &Arc<Notify>,
    ) -> bool {
        if !Self::is_screen_dark(backlight_controller).await {
            return false;
        }
        if let Err(err) = backlight_controller.turn_on().await {
            warn!(error = %err, "Failed to turn on backlight on wake");
        }
        brightness_modified.notify_waiters();
        true
    }

    pub(crate) async fn set_night_mode_screen_off_timeout(
        &self,
        timeout: Option<u32>,
    ) -> anyhow::Result<()> {
        self.night_mode_controller
            .set_screen_off_timeout(timeout)
            .await
    }

    pub(crate) fn subscribe_night_mode(&self) -> watch::Receiver<bool> {
        self.night_mode_controller.subscribe()
    }

    /// Fires once each time screen auto-off blanks the panel.
    ///
    /// Used e.g., by compositor, which resets the scene while nothing is visible.
    ///
    /// Subscribe before the first auto-off can fire, and treat `Lagged` as a
    /// notification rather than an error — resetting to the first scene is
    /// idempotent, so coalesced notifications must still trigger it.
    pub(crate) fn subscribe_screen_blanked(&self) -> broadcast::Receiver<()> {
        self.screen_blanked_tx.subscribe()
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
mod tests;
