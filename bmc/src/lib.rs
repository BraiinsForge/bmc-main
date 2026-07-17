// Copyright (C) 2025  Braiins Systems s.r.o.

mod alarm;
pub mod backlight;
pub mod bootloader_config;
mod button_manager;
pub mod compositor;
mod config;
pub mod config_migration;
mod data;
pub mod entry;
pub mod firmware;
mod initial_setup;
mod led;
pub mod led_coordinator;
pub mod log;
pub mod manager;
mod night_mode;
pub mod scene;
pub mod session;
mod sound;
mod startup;
mod system_manager;
mod system_upgrade;
pub mod utils;
mod web;
pub mod widget;

pub use led_coordinator::{Layer, LedCoordinatorHandle, spawn_led_coordinator};
pub use manager::{BmcManager, UpgradeError};
pub use startup::{App, Configuration};
pub use web::ServerConfig;
