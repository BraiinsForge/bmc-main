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

mod alarm;
pub mod backlight;
pub mod bootloader_config;
mod button_manager;
pub mod compositor;
mod config;
pub mod config_migration;
mod credential;
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
pub mod secret_store;
pub mod session;
pub mod shutdown;
mod sound;
mod startup;
mod system_manager;
mod system_upgrade;
pub mod utils;
mod web;
pub mod widget;

pub use led_coordinator::{Layer, LedCoordinatorHandle, spawn_led_coordinator};
pub use manager::{BmcManager, UpgradeError, UpgradeMarker};
pub use startup::{App, Configuration};
pub use web::ServerConfig;
