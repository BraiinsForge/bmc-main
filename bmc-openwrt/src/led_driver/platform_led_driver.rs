// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::led_driver::{
    config::{self, APA102_MAX_BRIGHTNESS},
    effects,
};

use crate::led_driver::embedded_hal::SpidevHalWrapper;
use bmc_led::{
    data::{self, LedCommand, LedEffect, Rgb},
    led_driver::{LedDriver, LedDriverFactory},
};
use spidev::{SpiModeFlags, Spidev, SpidevOptions};
use std::{path::PathBuf, time::Duration};
use tokio::{
    sync::mpsc::Receiver,
    time::{Instant, interval},
};
use tracing::error;

const COMMAND_BUFFER_SIZE: usize = 4;

#[derive(Debug, Default)]
pub struct LedState {
    period_us: u64,
    frame_interval: Duration,
    fireflies_state: effects::FirefliesState,
    brightness: u8,
    effect: LedEffect,
    color: Rgb,
    last_effect: LedEffect,
}

impl LedState {
    #[expect(clippy::integer_division)]
    fn default() -> Self {
        let period_us_temp = config::DEFAULT_PERIOD * 1_000;

        LedState {
            period_us: period_us_temp,
            frame_interval: Duration::from_micros(
                period_us_temp
                    / (u64::from(bmc_led::config::LED_COUNT) * u64::from(config::SUB_STEPS)),
            ),
            brightness: APA102_MAX_BRIGHTNESS,
            color: Rgb::new(0x8B, 0x31, 0xCF),
            ..Default::default()
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
    let start = Instant::now();
    let mut interval = interval(led_state.frame_interval);

    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        if !led_cmd_rx.is_empty() {
            if let Some(command) = led_cmd_rx.recv().await {
                match command {
                    /* LedCommand::SetTemporaryEffect(new_effect, new_color) => { // TODO: Implement temporary effects once we decide how to handle temporary states
                        led_state.effect = new_effect;
                        led_state.color = new_color;
                    } */
                    LedCommand::SetBrightness(new_brightness) => {
                        led_state.set_brightness(new_brightness);
                    }
                    LedCommand::SetColor(new_color) => {
                        led_state.color = new_color;
                    }
                    LedCommand::SetPersistentEffect(new_effect, new_color) => {
                        led_state.effect = new_effect;
                        led_state.last_effect = new_effect;
                        led_state.color = new_color;
                    }
                    LedCommand::Enable => {
                        led_state.effect = led_state.last_effect;
                    }
                    LedCommand::Disable => {
                        led_state.effect = LedEffect::None; // Turn off all effects
                    }
                }
            }
        }

        #[expect(clippy::cast_possible_truncation)]
        let elapsed = start.elapsed().as_micros() as u64 % led_state.period_us;
        #[expect(clippy::cast_precision_loss)]
        let phase = elapsed as f32 / led_state.period_us as f32;

        match led_state.effect {
            data::LedEffect::Snake => effects::update_snake(
                phase,
                config::SNAKE_LEN,
                led_state.brightness,
                led_state.color,
                &mut strip,
            ),
            data::LedEffect::Chase => effects::update_chase(
                phase,
                config::SNAKE_LEN,
                led_state.brightness,
                led_state.color,
                &mut strip,
            ),
            data::LedEffect::Scan => effects::update_scan(
                phase,
                config::SNAKE_LEN,
                led_state.brightness,
                led_state.color,
                &mut strip,
            ),
            data::LedEffect::Fireflies => effects::update_fireflies(
                &mut led_state.fireflies_state,
                phase,
                config::MAX_FLIES,
                led_state.brightness,
                led_state.color,
                &mut strip,
            ),
            data::LedEffect::KnightRider => effects::update_knight_rider(
                phase,
                3,
                4,
                led_state.brightness,
                led_state.color,
                &mut strip,
            ),
            data::LedEffect::None => effects::update_none(&mut strip),
        }

        interval.tick().await;
    }
}
