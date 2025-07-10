// Copyright (C) 2025  Braiins Systems s.r.o.

use super::config;
use super::data;
use super::effects;
use super::embedded_hal;
use crate::config::LED_COLOR_CYAN;
use crate::config::LED_COLOR_MAGENTA;
use crate::config::LED_COLOR_RED;
use crate::config::LED_COLOR_WARM_WHITE;
use crate::data::{LedCommand, LedEffect, Rgb};
use spidev::{SpiModeFlags, Spidev, SpidevOptions};
use std::fmt::Debug;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio::time::interval;
use tracing::error;

const COMMAND_BUFFER_SIZE: usize = 4;
const EVENT_BUFFER_SIZE: usize = 4;

#[derive(Debug, Default)]
struct LedState {
    period_us: u64,
    frame_interval: Duration,
    fireflies_state: effects::FirefliesState,
    brightness: u8,
    effect: LedEffect,
    color: Rgb,
}

impl LedState {
    #[expect(clippy::integer_division)]
    fn default() -> Self {
        let period_us_temp = config::DEFAULT_PERIOD * 1_000;

        LedState {
            period_us: period_us_temp,
            frame_interval: Duration::from_micros(
                period_us_temp / (u64::from(config::LED_COUNT) * u64::from(config::SUB_STEPS)),
            ),
            brightness: data::APA102_MAX_BRIGHTNESS,
            color: Rgb::new(0x8B, 0x31, 0xCF),
            ..Default::default()
        }
    }

    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_sign_loss)]
    fn set_brightness(&mut self, brightness: f32) {
        self.brightness =
            ((f32::from(data::APA102_MAX_BRIGHTNESS)) * brightness.clamp(0.0, 1.0)) as u8;
    }
}

#[derive(Debug)]
pub struct LedDriver {
    pub command_sender: Sender<LedCommand>,
}

impl LedDriver {
    #[must_use]
    pub fn new(device_path: &str) -> Self {
        let (command_sender, command_receiver) =
            tokio::sync::mpsc::channel::<LedCommand>(COMMAND_BUFFER_SIZE);

        tokio::spawn(Self::led_worker(
            PathBuf::from(device_path),
            command_receiver,
        ));

        Self { command_sender }
    }

    pub fn change_state(&self, _enabled: bool) -> anyhow::Result<()> {
        Ok(()) // TODO: Implement once new BMC board config is available
    }

    pub fn state(&self) -> anyhow::Result<bool> {
        Ok(true) // TODO: Implement once new BMC board config is available
    }

    pub fn toggle_state(&mut self) -> anyhow::Result<()> {
        self.state().and_then(|state| self.change_state(!state))
    }

    pub fn turn_on(&self) -> anyhow::Result<()> {
        self.change_state(true)
    }

    pub fn turn_off(&self) -> anyhow::Result<()> {
        self.change_state(false)
    }

    pub fn brightness(&self) -> anyhow::Result<f32> {
        Ok(1.0) // TODO: Implement once new BMC board config is available
    }

    #[must_use]
    pub fn max_brightness(&self) -> f32 {
        data::LED_MAX_BRIGHTNESS
    }

    pub fn set_brightness(&self, _value: f32) -> anyhow::Result<()> {
        Ok(()) // TODO: Implement once new BMC board config is available
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

        let mut spi = embedded_hal::SpidevHalWrapper(raw_spi);
        let mut strip = apa102_spi::Apa102Writer::new(
            &mut spi,
            config::LED_COUNT as usize,
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
                            led_state.color = new_color;
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
            }

            interval.tick().await;
        }
    }
}

#[derive(Debug, Default)]
pub struct LedEventHandler;

impl LedEventHandler {
    #[must_use]
    pub fn init(&self, command_sender: Sender<LedCommand>) -> Sender<data::LedEvent> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(EVENT_BUFFER_SIZE);

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                let update = match event {
                    data::LedEvent::Idle => {
                        Some((data::LedEffect::Fireflies, LED_COLOR_MAGENTA))
                    }
                    data::LedEvent::Alarm => Some((data::LedEffect::Scan, LED_COLOR_WARM_WHITE)),
                    data::LedEvent::DownloadStarted => {
                        Some((data::LedEffect::Snake, LED_COLOR_CYAN))
                    }
                    data::LedEvent::UpgradeStarted => {
                        Some((data::LedEffect::Chase, LED_COLOR_CYAN))
                    }
                    data::LedEvent::Failed => {
                        Some((data::LedEffect::Fireflies, LED_COLOR_RED))
                    }
                    data::LedEvent::DownloadFinished | data::LedEvent::DownloadProgress => None,
                };

                if let Some((effect, color)) = update {
                    let cmd = LedCommand::SetPersistentEffect(effect, color);
                    // Ignore the result, since we don't care if the send fails
                    if let Err(e) = command_sender.send(cmd).await {
                        error!("Failed to send command: {}", e);
                    }
                }
            }
        });

        sender
    }
}
