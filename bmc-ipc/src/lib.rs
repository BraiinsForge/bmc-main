// Copyright (C) 2025  Braiins Systems s.r.o.

mod codec;
pub mod messages;
pub mod types;

pub use codec::{CodecError, JsonLinesCodec};
pub use messages::{AppMessage, WidgetMessage};
pub use types::{
    ActionPayload, LedEffect, Localization, RgbColor, SettingUpdate, Settings, SizeInfo, SizeType,
};
