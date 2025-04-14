// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{fs, path::PathBuf};

use anyhow::bail;
use bmc_display::display_driver::DisplayBacklightDriver;
use std::io::Error;
use tracing::info;

pub const BL_POWER: &str = "bl_power";
pub const BRIGHTNESS: &str = "brightness";
pub const ACTUAL_BRIGHTNESS: &str = "actual_brightness";
pub const MAX_BRIGHTNESS: &str = "max_brightness";
const CMD_ON: &str = "0";
const CMD_OFF: &str = "4";

#[derive(Debug)]
pub struct GenericBacklightDriver {
    name: String,
    driver_path: PathBuf,
    max_brightness: u8,
}

impl GenericBacklightDriver {
    pub fn new(driver_path: &str) -> Self {
        GenericBacklightDriver {
            name: "Generic LCD backlight driver".to_string(),
            driver_path: PathBuf::from(driver_path),
            max_brightness: 0,
        }
    }
}

impl GenericBacklightDriver {
    fn read_value_from_fs(&self, file_name: &str) -> Result<String, Error> {
        fs::read(self.driver_path.join(file_name))
            .map(|val| String::from_utf8_lossy(&val).trim().to_string())
    }
}

impl DisplayBacklightDriver for GenericBacklightDriver {
    fn change_state(&self, enabled: bool) -> anyhow::Result<()> {
        info!(
            "{}: Setting display {}",
            self.name,
            if enabled { "on" } else { "off" }
        );

        let cmd = if enabled { CMD_ON } else { CMD_OFF };

        fs::write(self.driver_path.join(BL_POWER), cmd)?;
        Ok(())
    }

    fn state(&self) -> anyhow::Result<bool> {
        let state = self.read_value_from_fs(BL_POWER)?;

        if state == CMD_ON {
            Ok(true)
        } else if state == CMD_OFF {
            Ok(false)
        } else {
            bail!("{}: Unknown value: {}", self.name, state);
        }
    }

    fn brightness(&self) -> anyhow::Result<u8> {
        let brightness = self.read_value_from_fs(ACTUAL_BRIGHTNESS)?;
        Ok(brightness.parse::<u8>()?)
    }

    fn max_brightness(&self) -> u8 {
        self.max_brightness
    }

    fn set_brightness(&self, value: u8) -> anyhow::Result<()> {
        if value > self.max_brightness {
            bail!(
                "Brightness value {} cannot be greater than max_brightness {}.",
                value,
                self.max_brightness
            );
        }

        info!("{}: Setting display brightness {}", self.name, value);
        fs::write(self.driver_path.join(BRIGHTNESS), value.to_string())?;
        Ok(())
    }

    fn init(&mut self) -> anyhow::Result<()> {
        let max_brightness = self.read_value_from_fs(MAX_BRIGHTNESS)?;
        self.max_brightness = max_brightness.parse::<u8>()?;
        Ok(())
    }
}
