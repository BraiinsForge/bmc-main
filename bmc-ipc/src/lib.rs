// Copyright (C) 2025  Braiins Systems s.r.o.

mod error;
mod json_protocol;
pub mod messages;
mod protocol;
pub mod types;

pub use error::ProtocolError;
pub use json_protocol::JsonProtocol;
pub use messages::{AppMessage, WidgetMessage};
pub use protocol::Protocol;
pub use types::{
    ActionPayload, LedEffect, Localization, RgbColor, SettingUpdate, Settings, SizeInfo, SizeType,
};
