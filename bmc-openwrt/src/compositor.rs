// Copyright (C) 2025  Braiins Systems s.r.o.

//! EGL Compositor implementation for bmc-openwrt.

mod commands;
mod egl_compositor;
mod protocol;
mod render_egl;
mod state;

pub use commands::{CompositorCommand, CompositorResponse};
pub use egl_compositor::EglCompositor;
