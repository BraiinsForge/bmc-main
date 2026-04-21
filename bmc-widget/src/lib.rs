// Copyright (C) 2025  Braiins Systems s.r.o.

#[cfg(feature = "gpu")]
pub mod egl;
mod error;
mod manifest;
mod poll;
#[cfg(feature = "gpu")]
pub mod surface;
pub mod wayland;

pub use error::ManifestError;
pub use manifest::{Author, Manifest, ParamDefinition, ParamType, SettingKey};
pub use wayland::{WaylandError, WidgetEventHandler, WidgetProtocolClient};
