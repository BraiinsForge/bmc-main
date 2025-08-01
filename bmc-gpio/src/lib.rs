// Copyright (C) 2023  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

//! Provides GPIO primitives

use anyhow::{Result, anyhow};
use gpiod::{Active, Chip, Direction, Input, LineId, Lines, Options, Output};
use std::sync::{Arc, Mutex};

/// Trait for types that can be used as pin indexes.
pub trait Pin: Sized {
    /// Opens the pin and returns a PinIn instance.
    fn open_pin_in(&self) -> Result<PinIn>;
    /// Opens the pin and returns a PinOut instance.
    fn open_pin_out(&self, default_value: bool) -> Result<PinOut>;
}

/// Enum to hold different pin indexes
#[derive(Clone, Debug)]
pub enum PinSelector {
    Num(usize),
    PortLine((u32, u32)),
    Name(&'static str),
    PreparedPin(Arc<PreparedPin>),
}

impl Pin for PinSelector {
    fn open_pin_in(&self) -> Result<PinIn> {
        match self {
            PinSelector::Num(n) => n.open_pin_in(),
            PinSelector::PortLine(n) => n.open_pin_in(),
            PinSelector::Name(n) => n.open_pin_in(),
            PinSelector::PreparedPin(n) => n.open_pin_in(),
        }
    }

    fn open_pin_out(&self, default_value: bool) -> Result<PinOut> {
        match self {
            PinSelector::Num(n) => n.open_pin_out(default_value),
            PinSelector::PortLine(n) => n.open_pin_out(default_value),
            PinSelector::Name(n) => n.open_pin_out(default_value),
            PinSelector::PreparedPin(n) => n.open_pin_out(default_value),
        }
    }
}

/// Implementation for `usize` using `sysfs_gpio` as backend
impl Pin for usize {
    fn open_pin_in(&self) -> Result<PinIn> {
        let pin = sysfs_gpio::Pin::new(*self as u64);
        pin.export()?;
        pin.set_direction(sysfs_gpio::Direction::In)?;

        Ok(PinIn {
            pin: Arc::new(Mutex::new(PinSysfs { pin })),
        })
    }

    fn open_pin_out(&self, default_value: bool) -> Result<PinOut> {
        let pin = sysfs_gpio::Pin::new(*self as u64);
        pin.export()?;
        pin.set_direction(sysfs_gpio::Direction::Out)?;
        // Set the initial value by explicitly writing it to `value`.
        // This is to avoid the confusing behavior that when active_low is 1, then the output
        // logic is inverted but *NOT* the Direction::High and Direction::Low logic.
        let value = u8::from(default_value);
        pin.set_value(value)?;
        Ok(PinOut {
            pin: Arc::new(Mutex::new(PinSysfs { pin })),
        })
    }
}

/// Implementation for `(u32, u32)` aka (port, line) using `gpiod` as backend
impl Pin for (u32, u32) {
    fn open_pin_in(&self) -> Result<PinIn> {
        let &(port, line) = self;
        let chip = Chip::new(format!("/dev/gpiochip{port}"))?;
        let options = Options::input([line]).active(Active::High);
        let pin = chip.request_lines(options)?;

        Ok(PinIn {
            pin: Arc::new(Mutex::new(PinGpiod::<Input> { inner: pin })),
        })
    }

    fn open_pin_out(&self, default_value: bool) -> Result<PinOut> {
        let &(port, line) = self;
        let chip = Chip::new(format!("/dev/gpiochip{port}"))?;
        let options = Options::output([line]);
        let pin = chip.request_lines(options)?;

        // Set default_value
        pin.set_values([default_value])?;

        Ok(PinOut {
            pin: Arc::new(Mutex::new(PinGpiod::<Output> { inner: pin })),
        })
    }
}

/// Implementation for `&'static str` aka name of the pin using `gpiod` as backend
impl Pin for &'static str {
    fn open_pin_in(&self) -> Result<PinIn> {
        let (chip, line) = PinGpiod::find_pin(self)?;
        let options = Options::input([line]).active(Active::High);
        let pin = chip.request_lines(options)?;

        Ok(PinIn {
            pin: Arc::new(Mutex::new(PinGpiod::<Input> { inner: pin })),
        })
    }

    fn open_pin_out(&self, default_value: bool) -> Result<PinOut> {
        let (chip, line) = PinGpiod::find_pin(self)?;
        let options = Options::output([line]);
        let pin = chip.request_lines(options)?;
        // Set default_value
        pin.set_values([default_value])?;

        Ok(PinOut {
            pin: Arc::new(Mutex::new(PinGpiod::<Output> { inner: pin })),
        })
    }
}

/// Pin that has already cached the translated name -> (chip, line)
pub struct PreparedPin {
    chip: Chip,
    line: u32,
}

impl PreparedPin {
    pub fn lookup(name: &'static str) -> Result<Self> {
        let (chip, line) = PinGpiod::find_pin(name)?;
        Ok(Self { chip, line })
    }

    pub fn lookup_and_make_into_selector(name: &'static str) -> Result<PinSelector> {
        let pp = Self::lookup(name)?;
        Ok(PinSelector::PreparedPin(Arc::new(pp)))
    }
}

impl std::fmt::Debug for PreparedPin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PreparedPin")
    }
}

impl Pin for PreparedPin {
    fn open_pin_in(&self) -> Result<PinIn> {
        let options = Options::input([self.line]).active(Active::High);
        let pin = self.chip.request_lines(options)?;

        Ok(PinIn {
            pin: Arc::new(Mutex::new(PinGpiod::<Input> { inner: pin })),
        })
    }

    fn open_pin_out(&self, default_value: bool) -> Result<PinOut> {
        let options = Options::output([self.line]);
        let pin = self.chip.request_lines(options)?;
        // Set default_value
        pin.set_values([default_value])?;

        Ok(PinOut {
            pin: Arc::new(Mutex::new(PinGpiod::<Output> { inner: pin })),
        })
    }
}

/// Helper struct for altering output pins
#[derive(Clone)]
pub struct PinOut {
    pin: Arc<Mutex<dyn WritablePin>>,
}

// Implementing Debug for PinOut
impl std::fmt::Debug for PinOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PinOut")
    }
}

impl PinOut {
    pub fn write(&self, value: bool) -> Result<()> {
        self.pin
            .lock()
            .expect("BUG: PinIn mutex error")
            .write(value)
    }
}

/// Trait that had to be implement in order to work as Pin that can be read
pub trait ReadablePin: Send + Sync {
    fn read(&self) -> Result<bool>;
}

/// Trait that had to be implement in order to work as Pin that can be written
pub trait WritablePin: Send + Sync {
    fn write(&self, value: bool) -> Result<()>;
}

/// Helper struct for reading input pins
#[derive(Clone)]
pub struct PinIn {
    pin: Arc<Mutex<dyn ReadablePin>>,
}

impl std::fmt::Debug for PinIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PinIn")
    }
}

impl PinIn {
    pub fn read(&self) -> Result<bool> {
        self.pin.lock().expect("BUG: PinIn mutex error").read()
    }
}

/// Struct that implement `ReadablePin` and `WritablePin` with using `gpiod` library
pub struct PinGpiod<Direction> {
    inner: Lines<Direction>,
}

impl<Direction> std::fmt::Debug for PinGpiod<Direction> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PinGpiod")
    }
}

impl PinGpiod<Direction> {
    pub fn find_pin(pin_name: &'static str) -> Result<(Chip, LineId)> {
        // Loop trough all chips
        for device in Chip::list_devices()? {
            let chip = Chip::new(device)?;
            // Loop trough all lines
            for line in 0..chip.num_lines() {
                // Check if the name of the current line is same as defined PinIndex
                if chip.line_info(line)?.name == pin_name {
                    return Ok((chip, line));
                }
            }
        }
        Err(anyhow!("BUG: pin name {} not found!", pin_name))
    }
}

impl ReadablePin for PinGpiod<Input> {
    fn read(&self) -> Result<bool> {
        let value = self.inner.get_values([false])?;
        Ok(value[0])
    }
}

impl WritablePin for PinGpiod<Output> {
    fn write(&self, value: bool) -> Result<()> {
        Ok(self.inner.set_values([value])?)
    }
}

/// Struct that implement `ReadablePin` and `WritablePin` with using `sysfs_gpio` library
pub struct PinSysfs {
    pin: sysfs_gpio::Pin,
}

impl std::fmt::Debug for PinSysfs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PinSysfs {{ pin: {} }}", self.pin.get_pin())
    }
}

impl ReadablePin for PinSysfs {
    fn read(&self) -> Result<bool> {
        let value = self.pin.get_value()?;
        Ok(value != 0)
    }
}

impl WritablePin for PinSysfs {
    fn write(&self, value: bool) -> Result<()> {
        self.pin.set_value(u8::from(value))?;
        Ok(())
    }
}
