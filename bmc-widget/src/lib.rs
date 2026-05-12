// Copyright (C) 2025  Braiins Systems s.r.o.

#[cfg(feature = "gpu")]
pub mod egl;
mod poll;
#[cfg(feature = "gpu")]
pub mod surface;
pub mod wayland;

pub use wayland::{WaylandError, WidgetEventHandler, WidgetProtocolClient};
