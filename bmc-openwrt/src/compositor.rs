// Copyright (C) 2025  Braiins Systems s.r.o.

//! EGL Compositor implementation for bmc-openwrt.

mod commands;
mod device_access;
mod egl_compositor;
mod layer_surface;
mod lifecycle_emitter;
mod protocol;
mod render;
mod scene_cycling;
mod scene_renderer;
mod screen_edge;
mod state;
mod touch_gesture;
mod widget_tracker;

pub use commands::{CompositorCommand, CompositorResponse};
pub use device_access::DeviceAccessConfig;
pub use egl_compositor::EglCompositor;
