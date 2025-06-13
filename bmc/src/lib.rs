// Copyright (C) 2025  Braiins Systems s.r.o.

mod display_tasks;
pub mod entry;
pub mod firmware;
pub mod log;
pub mod manager;
pub mod session;
mod startup;
mod storage_checker;
mod system_upgrade;
pub mod time;
pub mod timezone_variant;
pub mod utils;
mod web;

pub use manager::BmcManager;
pub use startup::{App, Configuration};
pub use web::ServerConfig;
