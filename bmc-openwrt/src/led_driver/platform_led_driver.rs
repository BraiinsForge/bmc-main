// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::led_driver::embedded_hal::SpidevHalWrapper;
use crate::led_driver::{
    config::{self, APA102_MAX_BRIGHTNESS},
    effects,
};
use apa102_spi::{Apa102Pixel, SmartLedsWrite};
use bmc_led::config::SOLID_PERIOD;
use bmc_led::{
    data::{self, LedCommand, LedEffect, LedEventPersistence},
    led_driver::{LedDriver, LedDriverFactory},
};
use spidev::{SpiModeFlags, Spidev, SpidevOptions};
use std::{path::PathBuf, time::Duration};
use tokio::{sync::mpsc::Receiver, time::Instant};
use tracing::{debug, error};

const COMMAND_BUFFER_SIZE: usize = 16;

#[derive(Debug)]
pub struct LedState {
    enabled: bool,

    period_us: u64,
    frame_interval: Duration,
    brightness: u8,

    persistent_effect: LedEffect,
    temporary_effect: LedEffect,
    temporary_effect_started: Instant,
    temporary_effect_duration: Duration,
}

impl LedState {
    fn calc_frame_interval(period_us: u64) -> Duration {
        let result =
            Duration::from_micros(period_us.saturating_div(
                u64::from(bmc_led::config::LED_COUNT) * u64::from(config::SUB_STEPS),
            ));

        if result == Duration::from_micros(0) {
            Duration::from_micros(1)
        } else {
            result
        }
    }

    fn default() -> Self {
        LedState {
            enabled: true,
            period_us: u64::try_from(SOLID_PERIOD.as_micros()).unwrap_or_default(),
            frame_interval: LedState::calc_frame_interval(
                u64::try_from(SOLID_PERIOD.as_micros()).unwrap_or_default(),
            ),
            brightness: APA102_MAX_BRIGHTNESS,
            persistent_effect: LedEffect::None,
            temporary_effect: LedEffect::None,
            temporary_effect_started: Instant::now(),
            temporary_effect_duration: Duration::from_millis(0),
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
    let mut led_state: LedState = LedState::default();
    let mut start = Instant::now();
    let mut interval = tokio::time::interval(led_state.frame_interval);

    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        if !led_cmd_rx.is_empty() {
            if let Some(command) = led_cmd_rx.recv().await {
                match command {
                    LedCommand::SetBrightness(new_brightness) => {
                        led_state.set_brightness(new_brightness);
                    }
                    LedCommand::SetEffect(new_effect, persistence, period) => {
                        led_state.period_us = u64::try_from(period.as_micros()).unwrap_or_default();
                        led_state.frame_interval =
                            LedState::calc_frame_interval(led_state.period_us);

                        start = Instant::now();
                        interval = tokio::time::interval(led_state.frame_interval);

                        match persistence {
                            LedEventPersistence::Temporary(duration) => {
                                led_state.temporary_effect = new_effect;
                                led_state.temporary_effect_duration = duration;
                                led_state.temporary_effect_started = Instant::now();
                            }
                            LedEventPersistence::Persistent => {
                                led_state.persistent_effect = new_effect;
                            }
                        }

                        debug!(
                            "Set effect to {:?} with {:?} persistence for {:?}",
                            new_effect, persistence, period
                        );
                    }
                    LedCommand::Enable => {
                        led_state.enabled = true;
                    }
                    LedCommand::Disable => {
                        led_state.enabled = false;
                    }
                }
            }
        }

        #[expect(clippy::cast_possible_truncation)]
        #[expect(clippy::cast_precision_loss)]
        let phase = if led_state.period_us == 0 {
            0.0
        } else {
            #[expect(clippy::cast_possible_truncation)]
            let elapsed = start.elapsed().as_micros() as u64 % led_state.period_us;

            (elapsed as f64 / led_state.period_us as f64) as f32
        };

        if !led_state.enabled {
            effects::update_none(&mut strip);
            interval.tick().await;
            continue;
        }

        let effect = if led_state.temporary_effect == LedEffect::None {
            led_state.persistent_effect
        } else if led_state.temporary_effect_started.elapsed() < led_state.temporary_effect_duration
        {
            led_state.temporary_effect
        } else {
            led_state.temporary_effect = LedEffect::None;
            led_state.persistent_effect
        };

        update_effect(effect, phase, &led_state, &mut strip);

        interval.tick().await;
    }
}

fn update_effect<W>(effect: LedEffect, phase: f32, led_state: &LedState, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    match effect {
        data::LedEffect::Snake(color) => {
            effects::update_snake(phase, config::SNAKE_LEN, led_state.brightness, color, strip);
        }
        data::LedEffect::Chase(color) => {
            effects::update_chase(phase, config::SNAKE_LEN, led_state.brightness, color, strip);
        }
        data::LedEffect::Scan(color) => {
            effects::update_scan(phase, config::SNAKE_LEN, led_state.brightness, color, strip);
        }
        data::LedEffect::KnightRider(color) => {
            effects::update_knight_rider(
                phase,
                config::SNAKE_LEN,
                config::SNAKE_LEN + 1,
                led_state.brightness,
                color,
                strip,
            );
        }
        data::LedEffect::Breathe(color) => {
            effects::update_breathe(phase, led_state.brightness, color, strip);
        }
        data::LedEffect::Solid(color) => effects::update_solid(led_state.brightness, color, strip),
        data::LedEffect::None => effects::update_none(strip),
    }
}
