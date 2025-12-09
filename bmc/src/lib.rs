// Copyright (C) 2025  Braiins Systems s.r.o.

mod alarm;
mod backlight;
pub mod bootloader_config;
mod button_manager;
mod config;
mod countdown_types;
mod display_tasks;
pub mod entry;
pub mod firmware;
mod initial_setup;
mod led;
pub mod log;
pub mod manager;
mod night_mode;
pub mod session;
mod sound;
mod startup;
mod storage_checker;
mod system_manager;
mod system_upgrade;
pub mod utils;
mod web;
pub mod widget;
mod widget_tasks;

pub use manager::BmcManager;
pub use startup::{App, Configuration};
pub use web::ServerConfig;
