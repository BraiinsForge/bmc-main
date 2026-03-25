// Copyright (C) 2025  Braiins Systems s.r.o.

use super::embedded_hal::SpidevHalWrapper;
use super::{
    config::{self, APA102_MAX_BRIGHTNESS, FRAME_RATE_HZ},
    effects,
};
use crate::{
    data::{self, LedCommand, LedEffect, LedScene},
    led_driver::{LedDriver, LedDriverFactory},
};
use apa102_spi::{Apa102Pixel, SmartLedsWrite};
use spidev::{SpiModeFlags, Spidev, SpidevOptions};
use std::{path::PathBuf, time::Duration};
use tokio::{
    sync::mpsc::Receiver,
    time::{Instant, MissedTickBehavior},
};
use tracing::{debug, error};

const COMMAND_BUFFER_SIZE: usize = 16;

/// An active scene: a `LedScene` paired with the instant it was activated.
#[derive(Debug)]
struct ActiveScene {
    scene: LedScene,
    started: Instant,
}

impl ActiveScene {
    fn new(scene: LedScene) -> Self {
        Self {
            scene,
            started: Instant::now(),
        }
    }

    /// Whether this scene has expired (only possible when `duration` is set).
    fn expired(&self) -> bool {
        match self.scene.duration {
            Some(duration) => self.started.elapsed() >= duration,
            None => false,
        }
    }

    /// Expiry instant, if this scene has a finite duration.
    fn expiry(&self) -> Option<Instant> {
        self.scene.duration.map(|d| self.started + d)
    }

    fn effect(&self) -> LedEffect {
        self.scene.effect
    }

    /// Whether this scene needs frame ticking (has an animation period).
    fn is_animated(&self) -> bool {
        self.scene.period.is_some()
    }

    /// Animation phase (0.0–1.0) within the current period cycle.
    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_precision_loss)]
    fn phase(&self) -> f32 {
        let period_us = match self.scene.period {
            Some(period) => period.as_micros(),
            None => return 0.0,
        };
        let elapsed = self.started.elapsed().as_micros() % period_us;
        (elapsed as f64 / period_us as f64) as f32
    }
}

#[derive(Debug)]
pub struct LedState {
    enabled: bool,
    brightness: u8,

    /// Frame tick interval used for running non-static effects.
    frame_interval: tokio::time::Interval,

    persistent: ActiveScene,
    temporary: Option<ActiveScene>,
}

impl LedState {
    fn new() -> Self {
        let mut frame_interval =
            tokio::time::interval(Duration::from_secs_f64(1.0 / FRAME_RATE_HZ));
        frame_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            enabled: true,
            brightness: APA102_MAX_BRIGHTNESS,
            frame_interval,
            persistent: ActiveScene::new(LedScene {
                effect: LedEffect::None,
                period: None,
                duration: None,
            }),
            temporary: None,
        }
    }

    /// Return the currently active scene, expiring temporary scenes that have
    /// exceeded their duration.
    fn active_scene(&mut self) -> &ActiveScene {
        if self.temporary.as_ref().is_some_and(ActiveScene::expired) {
            self.temporary = None;
        }
        self.temporary.as_ref().unwrap_or(&self.persistent)
    }

    fn apply_command(&mut self, command: LedCommand) {
        match command {
            LedCommand::SetBrightness(new_brightness) => {
                self.set_brightness(new_brightness);
            }
            LedCommand::SetEffect(scene) => {
                debug!("Set effect: {:?}", scene);
                if scene.duration.is_some() {
                    self.temporary = Some(ActiveScene::new(scene));
                } else {
                    self.persistent = ActiveScene::new(scene);
                }
                self.frame_interval.reset();
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
    fn render<W>(&mut self, strip: &mut W)
    where
        W: SmartLedsWrite<Color = Apa102Pixel>,
    {
        if !self.enabled {
            effects::update_none(strip);
            return;
        }

        let scene = self.active_scene();
        let effect = scene.effect();
        let phase = scene.phase();

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

    /// Wait until the next render is needed: frame tick, temp-effect expiry,
    /// or block forever (only a command can wake the loop).
    async fn next_wake(&mut self) {
        let animated = self.active_scene().is_animated();
        if self.enabled && animated {
            self.frame_interval.tick().await;
        } else {
            match self.temporary.as_ref().and_then(ActiveScene::expiry) {
                Some(expiry) => tokio::time::sleep_until(expiry).await,
                None => std::future::pending().await,
            }
        }
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
        crate::config::LED_COUNT as usize,
        apa102_spi::PixelOrder::BGR,
    );
    let mut led_state = LedState::new();

    loop {
        led_state.render(&mut strip);

        let reason = tokio::select! {
            () = led_state.next_wake() => WakeReason::Tick,
            cmd = led_cmd_rx.recv() => match cmd {
                Some(c) => WakeReason::Command(c),
                None => WakeReason::ChannelClosed,
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
