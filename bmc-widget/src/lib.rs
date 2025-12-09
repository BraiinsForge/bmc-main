// Copyright (C) 2025  Braiins Systems s.r.o.

mod client;
mod error;
mod manifest;

pub use client::{ClientError, WidgetClient};
pub use error::ManifestError;
pub use manifest::{Author, Manifest, ParamDefinition, ParamType, SettingKey};
