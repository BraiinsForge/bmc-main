// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_platform::backlight::DisplayBacklightDriver;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct MockBacklightDriver {
    state: Arc<AtomicBool>,
    brightness: Arc<AtomicU8>,
    max_brightness: u8,
}

impl MockBacklightDriver {
    #[must_use]
    pub fn new(state: bool, brightness: u8, max_brightness: u8) -> Self {
        Self {
            state: Arc::new(AtomicBool::new(state)),
            brightness: Arc::new(AtomicU8::new(brightness)),
            max_brightness,
        }
    }
}

impl DisplayBacklightDriver for MockBacklightDriver {
    fn init(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn change_state(&self, enabled: bool) -> anyhow::Result<()> {
        info!("Setting display {}", if enabled { "on" } else { "off" });
        self.state.store(enabled, Ordering::Release);
        Ok(())
    }

    fn state(&self) -> anyhow::Result<bool> {
        let state = self.state.load(Ordering::Acquire);
        Ok(state)
    }

    fn brightness(&self) -> anyhow::Result<u8> {
        Ok(self.brightness.load(Ordering::Acquire))
    }

    fn max_brightness(&self) -> u8 {
        self.max_brightness
    }

    fn set_brightness(&self, value: u8) -> anyhow::Result<()> {
        debug!("Setting display brightness to {}", value);
        self.brightness.store(value, Ordering::Release);
        debug!("New brightness {}", self.brightness().unwrap_or_default());
        Ok(())
    }
}
