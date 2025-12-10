// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bmc_shared_time::time::DateFormat;
use chrono::Utc;
use chrono_tz::Tz;
use slint::{ComponentHandle, LogicalSize, Timer, TimerMode, WindowSize};

use crate::{Config, DateTime, DigitalClock};

/// The digital clock widget with encapsulated business logic.
pub struct DigitalClockWidget {
    ui: DigitalClock,
    date_format: Arc<AtomicU8>,
    timezone: Arc<RwLock<String>>,
    is_24_format: Arc<AtomicBool>,
    _timer: Timer,
}

impl std::fmt::Debug for DigitalClockWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DigitalClockWidget").finish_non_exhaustive()
    }
}

impl DigitalClockWidget {
    /// Creates a new digital clock widget with the given configuration.
    pub fn new(config: Config) -> Result<Self, slint::PlatformError> {
        let ui = DigitalClock::new()?;

        #[expect(clippy::cast_precision_loss)]
        ui.window().set_size(WindowSize::Logical(LogicalSize::new(
            config.width as f32,
            config.height as f32,
        )));

        ui.set_size(config.size);
        ui.set_show_seconds(config.show_seconds);
        ui.set_show_timezone(config.show_timezone);
        ui.set_font_style(config.font_style);

        let date_format = Arc::new(AtomicU8::new(config.date_format as u8));
        let timezone = Arc::new(RwLock::new(config.timezone));
        let is_24_format = Arc::new(AtomicBool::new(config.is_24_format));

        Self::update_time(&ui, &timezone, &date_format, &is_24_format);

        let timer = Timer::default();
        let ui_handle = ui.as_weak();
        let date_format_clone = Arc::clone(&date_format);
        let timezone_clone = Arc::clone(&timezone);
        let is_24_format_clone = Arc::clone(&is_24_format);
        timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
            if let Some(ui) = ui_handle.upgrade() {
                Self::update_time(
                    &ui,
                    &timezone_clone,
                    &date_format_clone,
                    &is_24_format_clone,
                );
            }
        });

        Ok(Self {
            ui,
            date_format,
            timezone,
            is_24_format,
            _timer: timer,
        })
    }

    fn update_time(
        ui: &DigitalClock,
        timezone: &Arc<RwLock<String>>,
        date_format: &Arc<AtomicU8>,
        is_24_format: &Arc<AtomicBool>,
    ) {
        let tz_str = timezone
            .read()
            .expect("BUG: timezone lock poisoned")
            .clone();
        let tz: Tz = tz_str.parse().unwrap_or(Tz::UTC);
        let now_utc = Utc::now();
        let now_tz = now_utc.with_timezone(&tz);
        let datetime_fixed = now_tz.fixed_offset();

        let df: DateFormat = date_format.load(Ordering::Relaxed).into();
        let is_24 = is_24_format.load(Ordering::Relaxed);

        let datetime = bmc_shared_slint::to_datetime!(datetime_fixed, tz_str, is_24, df);
        ui.set_datetime(datetime);
    }

    /// Returns a weak handle to the UI for use in async contexts.
    pub fn as_weak(&self) -> slint::Weak<DigitalClock> {
        self.ui.as_weak()
    }

    /// Returns a clone of the date format atomic for external updates.
    pub fn date_format(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.date_format)
    }

    /// Returns a clone of the timezone for external updates.
    pub fn timezone(&self) -> Arc<RwLock<String>> {
        Arc::clone(&self.timezone)
    }

    /// Returns a clone of the is_24_format for external updates.
    pub fn is_24_format(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.is_24_format)
    }

    /// Runs the widget event loop (blocking).
    pub fn run(&self) -> Result<(), slint::PlatformError> {
        self.ui.run()
    }
}
