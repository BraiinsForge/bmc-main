// Copyright (C) 2025  Braiins Systems s.r.o.

use super::config;
use super::data;
use super::effects;
use super::embedded_hal;
use crate::data::{LedCommand, LedEffect, Rgb};
use crate::effects::FirefliesState;
use spidev::{SpiModeFlags, Spidev, SpidevOptions};
use std::fmt::Debug;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;

const EVENT_BUFFER_SIZE: usize = 4;
const COMMAND_BUFFER_SIZE: usize = 4;

#[async_trait::async_trait]
pub trait LedHandle: Sync + Send + Clone + Debug {
    fn init(&self) -> anyhow::Result<()>;
    async fn emit_event(&self, event: data::LedEvent);
}

#[derive(Debug)]
struct LedState {
    period_us: u64,
    frame_interval: Duration,
    fireflies_state: effects::FirefliesState,
    brightness: u8,
    effect: LedEffect,
    color: Rgb,
}

impl LedState {
    const fn default() -> Self {
        let mut temp = LedState {
            period_us: config::DEFAULT_PERIOD * 1_000,
            frame_interval: Duration::new(0, 0),
            fireflies_state: FirefliesState::default(),
            brightness: config::APA102_MAX_BRIGHTNESS,
            effect: LedEffect::Fireflies,
            color: Rgb::from(0x8B, 0x31, 0xCF),
        };

        temp.frame_interval = Duration::from_micros(
            temp.period_us / (config::LED_COUNT as u64 * config::SUB_STEPS as u64),
        );

        temp
    }

    fn set_brightness(&mut self, brightness: f32) {
        self.brightness =
            ((config::APA102_MAX_BRIGHTNESS as f32) * brightness.clamp(0.0, 1.0)) as u8
    }
}

#[derive(Debug)]
pub struct LedDriver;

impl LedDriver {
    pub fn new() -> Self {
        Self {}
    }

    pub fn change_state(&self, _enabled: bool) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn state(&self) -> anyhow::Result<bool> {
        Ok(true)
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

    pub fn brightness(&self) -> anyhow::Result<u8> {
        Ok(0)
    }

    pub fn max_brightness(&self) -> u8 {
        config::APA102_MAX_BRIGHTNESS
    }

    pub fn set_brightness(&self, _value: u8) -> anyhow::Result<()> {
        Ok(())
    }

    async fn led_worker(mut led_cmd_rx: Receiver<LedCommand>) -> ! {
        let mut raw_spi = Spidev::open(config::SPI_DEV).expect("Failed to open SPI device");
        let options = SpidevOptions::new()
            .bits_per_word(8)
            .max_speed_hz(4_000_000)
            .mode(SpiModeFlags::SPI_MODE_0)
            .build();
        raw_spi
            .configure(&options)
            .expect("Failed to configure SPI");

        let mut spi = embedded_hal::SpidevHalWrapper(raw_spi);
        let mut strip =
            apa102_spi::Apa102Writer::new(&mut spi, config::LED_COUNT, apa102_spi::PixelOrder::BGR);

        let mut led_state: LedState = LedState::default();

        let start = Instant::now();
        let mut next = start + led_state.frame_interval;

        loop {
            if !led_cmd_rx.is_empty() {
                if let Some(command) = led_cmd_rx.recv().await {
                    match command {
                        /* LedCommand::SetTemporaryEffect(new_effect, new_color) => {
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
                        LedCommand::NoChange => {
                            // Do nothing
                        }
                    }
                }
            }

            let elapsed = (Instant::now() - start).as_micros() as u64 % led_state.period_us;
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
            }

            tokio::time::sleep_until(tokio::time::Instant::from_std(next)).await;

            next += led_state.frame_interval;
        }
    }

    pub fn init(&mut self) -> anyhow::Result<Sender<LedCommand>> {
        let (command_sender, command_receiver) =
            tokio::sync::mpsc::channel::<LedCommand>(COMMAND_BUFFER_SIZE);

        tokio::spawn(Self::led_worker(command_receiver));

        Ok(command_sender)
    }
}

#[derive(Debug, Clone)]
pub struct LedHandler {
    event_sender: Sender<data::LedEvent>,
}

impl LedHandler {
    pub fn new(command_sender: Sender<LedCommand>) -> Self {
        Self {
            event_sender: EventHandler::init(command_sender),
        }
    }
}

#[async_trait::async_trait]
impl LedHandle for LedHandler {
    fn init(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn emit_event(&self, event: data::LedEvent) {
        _ = self.event_sender.send(event).await;
    }
}

struct EventHandler;

impl EventHandler {
    fn init(command_sender: Sender<LedCommand>) -> Sender<data::LedEvent> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(EVENT_BUFFER_SIZE);

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                let cmd = match event {
                    data::LedEvent::Idle => data::LedCommand::SetPersistentEffect(
                        data::LedEffect::Fireflies,
                        Rgb::from(226, 52, 235),
                    ),
                    data::LedEvent::Alarm => data::LedCommand::SetPersistentEffect(
                        data::LedEffect::Scan,
                        Rgb::from(235, 232, 52),
                    ),
                    data::LedEvent::DownloadStarted => data::LedCommand::SetPersistentEffect(
                        data::LedEffect::Snake,
                        Rgb::from(52, 180, 235),
                    ),
                    data::LedEvent::UpgradeStarted => data::LedCommand::SetPersistentEffect(
                        data::LedEffect::Chase,
                        Rgb::from(52, 180, 235),
                    ),
                    data::LedEvent::UpgradeFailed => data::LedCommand::SetPersistentEffect(
                        data::LedEffect::Fireflies,
                        Rgb::from(255, 0, 0),
                    ),
                    data::LedEvent::UpgradeFinishedSuccessfully => {
                        data::LedCommand::SetPersistentEffect(
                            data::LedEffect::Fireflies,
                            Rgb::from(0, 255, 0),
                        )
                    }
                    _ => data::LedCommand::NoChange,
                };

                let _ = command_sender.send(cmd);
            }
        });

        sender
    }
}
