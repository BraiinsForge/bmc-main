// Copyright (C) 2025  Braiins Systems s.r.o.

mod client;
#[cfg(feature = "gpu")]
pub mod egl;
pub mod env;
mod error;
mod ipc;
mod manifest;
pub mod wayland;

pub use client::{ClientError, WidgetClient};
pub use env::{EnvError, read_instance_id, read_params, read_settings, read_size};
pub use error::ManifestError;
pub use ipc::{connect_widget, run_message_loop};
pub use manifest::{Author, Manifest, ParamDefinition, ParamType, SettingKey};
pub use wayland::{WaylandError, WidgetEventHandler, WidgetProtocolClient};
