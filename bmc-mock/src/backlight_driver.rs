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
