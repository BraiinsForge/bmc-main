// Copyright (C) 2025  Braiins Systems s.r.o.

mod config;
pub mod data;
#[cfg(feature = "hardware")]
pub mod effects;
#[cfg(feature = "hardware")]
mod embedded_hal;
#[cfg(feature = "hardware")]
pub mod led_driver;
#[cfg(feature = "simulator")]
pub mod led_driver_simulator;

use tracing as _;
