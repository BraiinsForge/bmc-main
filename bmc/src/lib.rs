// Copyright (C) 2025  Braiins Systems s.r.o.

mod alarm;
pub mod backlight;
pub mod bootloader_config;
mod button_manager;
pub mod compositor;
mod config;
mod display_tasks;
pub mod entry;
pub mod firmware;
mod initial_setup;
mod led;
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
// TODO: display refactor
#[expect(dead_code)]
mod web;
pub mod widget;
// TODO: display refactor
#[expect(dead_code)]
mod widget_tasks;

pub use manager::BmcManager;
pub use startup::{App, Configuration};
pub use web::ServerConfig;
