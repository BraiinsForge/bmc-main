// Copyright (C) 2025  Braiins Systems s.r.o.

//! Widget system for discovering, spawning, and managing widget processes.

mod discovery;
mod registry;
mod spawner;

pub use discovery::{PathDiscovery, WidgetDiscovery};
pub use registry::{RegistryError, WidgetInfo, WidgetRegistry};
pub use spawner::{ProcessSpawner, SpawnError, UnixConnection, UnixSpawner, WidgetConnection};
