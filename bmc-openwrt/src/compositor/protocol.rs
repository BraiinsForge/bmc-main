// Copyright (C) 2025  Braiins Systems s.r.o.

//! deck_widget_v1 Wayland protocol handlers.

mod conversions;
mod dispatch;
mod state;

pub use dispatch::{
    DeckWidgetHandler, WidgetManagerUserData, WidgetSurfaceUserData, create_global,
};
pub use state::DeckWidgetProtocolState;
