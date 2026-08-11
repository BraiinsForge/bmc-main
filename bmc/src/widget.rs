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

//! Widget system for discovering, spawning, and managing widget processes.

pub mod action_handler;
mod capture;
pub(crate) mod coordinator;
mod discovery;
mod led_state;
mod manager;
mod registry;
mod signals;
mod spawner;

pub(crate) use coordinator::UpgradeWidgetLifecycle;
pub use coordinator::{Coordinator, WidgetEnv};
pub use discovery::{PathDiscovery, WidgetDiscovery};
pub use manager::WidgetManager;
pub use registry::{
    RegistryError, ViewportDescriptor, WidgetIdentity, WidgetInfo, WidgetRegistry,
    slot_span_descriptor,
};
pub(crate) use signals::spawn_reload_signal_task;
pub use spawner::{SpawnError, WaylandSpawner};
