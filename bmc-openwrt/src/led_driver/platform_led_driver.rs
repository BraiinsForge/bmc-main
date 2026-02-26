// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::led_driver::embedded_hal::SpidevHalWrapper;
use crate::led_driver::{
    config::{self, APA102_MAX_BRIGHTNESS},
    effects,
};
use apa102_spi::{Apa102Pixel, SmartLedsWrite};
use bmc_led::{
    data::{self, LedCommand, LedEffect, LedEventPersistence},
    led_driver::{LedDriver, LedDriverFactory},
};
use spidev::{SpiModeFlags, Spidev, SpidevOptions};
use std::{path::PathBuf, time::Duration};
use tokio::{
    sync::mpsc::Receiver,
    time::{Instant, MissedTickBehavior},
};
use tracing::{debug, error};

const COMMAND_BUFFER_SIZE: usize = 16;

#[derive(Debug)]
pub struct LedState {
    enabled: bool,
    brightness: u8,

    /// Frame tick interval.  `None` for static effects (Solid, None) —
    /// the worker blocks on `recv()` only.  Animation period is recovered
    /// via `interval.period() * LED_COUNT * SUB_STEPS`.
    interval: Option<tokio::time::Interval>,
    /// When the current animation cycle started (for phase calculation).
    animation_start: Instant,

    persistent_effect: LedEffect,
    temporary_effect: LedEffect,
    temporary_effect_started: Instant,
    temporary_effect_duration: Duration,
}

impl LedState {
    fn new() -> Self {
        Self {
            enabled: true,
            brightness: APA102_MAX_BRIGHTNESS,
            interval: None,
            animation_start: Instant::now(),
            persistent_effect: LedEffect::None,
            temporary_effect: LedEffect::None,
            temporary_effect_started: Instant::now(),
            temporary_effect_duration: Duration::ZERO,
        }
    }

    /// Compute the frame-tick interval from an animation period.
    /// Returns `None` for static effects (period is `None` or zero).
    fn frame_interval(period: Option<Duration>) -> Option<Duration> {
        let period = period?;
        let period_us = u64::try_from(period.as_micros()).unwrap_or(1);
        let divisor = u64::from(bmc_led::config::LED_COUNT) * u64::from(config::SUB_STEPS);
        Some(Duration::from_micros(period_us.saturating_div(divisor)))
    }

    fn make_interval(duration: Duration) -> tokio::time::Interval {
        let mut iv = tokio::time::interval(duration);
        iv.set_missed_tick_behavior(MissedTickBehavior::Delay);
        iv
    }

    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_precision_loss)]
    fn phase(&self) -> f32 {
        let iv = match &self.interval {
            Some(iv) => iv,
            None => return 0.0,
        };
        let tick_us = iv.period().as_micros() as u64;
        let divisor = u64::from(bmc_led::config::LED_COUNT) * u64::from(config::SUB_STEPS);
        let period_us = tick_us * divisor;
        let elapsed = self.animation_start.elapsed().as_micros() as u64 % period_us;
        (elapsed as f64 / period_us as f64) as f32
    }

    /// Determine the currently active LED effect, expiring temporary
    /// effects that have exceeded their duration.
    fn active_effect(&mut self) -> LedEffect {
        if self.temporary_effect == LedEffect::None {
            return self.persistent_effect;
        }
        if self.temporary_effect_started.elapsed() < self.temporary_effect_duration {
            return self.temporary_effect;
        }
        self.temporary_effect = LedEffect::None;
        self.persistent_effect
    }

    /// Expiry instant for the active temporary effect, if any.
    fn temp_expiry(&self) -> Option<Instant> {
        if self.temporary_effect != LedEffect::None {
            Some(self.temporary_effect_started + self.temporary_effect_duration)
        } else {
            None
        }
    }

    fn apply_command(&mut self, command: LedCommand) {
        match command {
            LedCommand::SetBrightness(new_brightness) => {
                self.set_brightness(new_brightness);
            }
            LedCommand::SetEffect(new_effect, persistence, period) => {
                let period = if period.is_zero() { None } else { Some(period) };

                match persistence {
                    LedEventPersistence::Temporary(duration) => {
                        self.temporary_effect = new_effect;
                        self.temporary_effect_duration = duration;
                        self.temporary_effect_started = Instant::now();
                    }
                    LedEventPersistence::Persistent => {
                        self.persistent_effect = new_effect;
                        self.animation_start = Instant::now();
                        self.interval = Self::frame_interval(period).map(Self::make_interval);
                    }
                }

                debug!(
                    "Set effect to {:?} with {:?} persistence for {:?}",
                    new_effect, persistence, period
                );
            }
            LedCommand::Enable => {
                self.enabled = true;
            }
            LedCommand::Disable => {
                self.enabled = false;
            }
        }
    }

    /// Render the current frame to the LED strip.
    fn update_effect<W>(&mut self, strip: &mut W)
    where
        W: SmartLedsWrite<Color = Apa102Pixel>,
    {
        let effect = self.active_effect();
        let phase = self.phase();

        match effect {
            data::LedEffect::Snake(color) => {
                effects::update_snake(phase, config::SNAKE_LEN, self.brightness, color, strip);
            }
            data::LedEffect::Chase(color) => {
                effects::update_chase(phase, config::SNAKE_LEN, self.brightness, color, strip);
            }
            data::LedEffect::Scan(color) => {
                effects::update_scan(phase, config::SNAKE_LEN, self.brightness, color, strip);
            }
            data::LedEffect::KnightRider(color) => {
                effects::update_knight_rider(
                    phase,
                    config::SNAKE_LEN,
                    config::SNAKE_LEN + 1,
                    self.brightness,
                    color,
                    strip,
                );
            }
            data::LedEffect::Breathe(color) => {
                effects::update_breathe(phase, self.brightness, color, strip);
            }
            data::LedEffect::Solid(color) => {
                effects::update_solid(self.brightness, color, strip);
            }
            data::LedEffect::None => effects::update_none(strip),
        }
    }

    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_sign_loss)]
    fn set_brightness(&mut self, brightness: f32) {
        self.brightness = ((f32::from(APA102_MAX_BRIGHTNESS)) * brightness.clamp(0.0, 1.0)) as u8;
    }
}

#[derive(Debug)]
pub struct PlatformLedDriver(pub LedDriver);

impl LedDriverFactory for PlatformLedDriver {
    #[must_use]
    fn new(device_path: &str) -> Self {
        let (command_sender, command_receiver) =
            tokio::sync::mpsc::channel::<LedCommand>(COMMAND_BUFFER_SIZE);

        tokio::spawn(led_worker(PathBuf::from(device_path), command_receiver));

        PlatformLedDriver(LedDriver { command_sender })
    }
}

enum WakeReason {
    Tick,
    Command(LedCommand),
    ChannelClosed,
}

async fn led_worker(device_path: PathBuf, mut led_cmd_rx: Receiver<LedCommand>) {
    let mut raw_spi = match Spidev::open(device_path) {
        Ok(raw_spi) => raw_spi,
        Err(error) => {
            error!("SPI device open failed {error}");
            return;
        }
    };

    let options = SpidevOptions::new()
        .bits_per_word(8)
        .max_speed_hz(4_000_000)
        .mode(SpiModeFlags::SPI_MODE_0)
        .build();

    if let Err(error) = raw_spi.configure(&options) {
        error!("SPI device configuration failed {error}");
        return;
    }

    let mut spi = SpidevHalWrapper(raw_spi);
    let mut strip = apa102_spi::Apa102Writer::new(
        &mut spi,
        bmc_led::config::LED_COUNT as usize,
        apa102_spi::PixelOrder::BGR,
    );
    let mut led_state = LedState::new();

    loop {
        led_state.update_effect(&mut strip);

        // Compute the temp-expiry deadline as an owned value *before*
        // borrowing `interval`, so borrow splitting still works.
        let temp_expiry = led_state.temp_expiry();

        // Wait for the next frame tick, temp expiry, or a command.
        let reason = match &mut led_state.interval {
            // Animated persistent effect — tick-driven rendering.
            // Temp effects expire naturally through the render cycle.
            Some(interval) => tokio::select! {
                _ = interval.tick() => WakeReason::Tick,
                cmd = led_cmd_rx.recv() => match cmd {
                    Some(c) => WakeReason::Command(c),
                    None => WakeReason::ChannelClosed,
                },
            },
            // Static persistent effect with a pending temp expiry —
            // sleep until the temp deadline, then re-render so
            // active_effect() expires it through the normal path.
            None => match temp_expiry {
                Some(expiry) => tokio::select! {
                    _ = tokio::time::sleep_until(expiry) => WakeReason::Tick,
                    cmd = led_cmd_rx.recv() => match cmd {
                        Some(c) => WakeReason::Command(c),
                        None => WakeReason::ChannelClosed,
                    },
                },
                // No animation, no pending temp — block on recv only.
                None => match led_cmd_rx.recv().await {
                    Some(c) => WakeReason::Command(c),
                    None => WakeReason::ChannelClosed,
                },
            },
        };

        match reason {
            WakeReason::Tick => {}
            WakeReason::Command(command) => {
                led_state.apply_command(command);
                while let Ok(command) = led_cmd_rx.try_recv() {
                    led_state.apply_command(command);
                }
            }
            WakeReason::ChannelClosed => break,
        }
    }
}
