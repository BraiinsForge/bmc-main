// Copyright (C) 2025  Braiins Systems s.r.o.

//! Widget system for discovering, spawning, and managing widget processes.

mod coordinator;
mod discovery;
mod manager;
mod registry;
mod spawner;

pub use coordinator::Coordinator;
pub use discovery::{PathDiscovery, WidgetDiscovery};
pub use manager::WidgetManager;
pub use registry::{RegistryError, WidgetInfo, WidgetRegistry};
pub use spawner::{SpawnError, UnixConnection, UnixSpawner};
