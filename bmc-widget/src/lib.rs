// Copyright (C) 2025  Braiins Systems s.r.o.

mod discovery;
mod error;
mod manifest;
mod registry;
mod spawner;

pub use discovery::{PathDiscovery, WidgetDiscovery};
pub use error::ManifestError;
pub use manifest::{Author, Manifest, ParamDefinition, ParamType, SettingKey};
pub use registry::{RegistryError, WidgetInfo, WidgetRegistry};
pub use spawner::{ProcessSpawner, SpawnError, WidgetConnection};
