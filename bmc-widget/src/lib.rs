// Copyright (C) 2025  Braiins Systems s.r.o.

mod client;
mod error;
mod ipc;
mod manifest;

pub use client::{ClientError, WidgetClient};
pub use error::ManifestError;
pub use ipc::{connect_widget, run_message_loop};
pub use manifest::{Author, Manifest, ParamDefinition, ParamType, SettingKey};
