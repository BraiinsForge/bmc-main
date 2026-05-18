// Copyright (C) 2026  Braiins Systems s.r.o.

use std::env;
use std::os::unix::net::UnixStream;

use anyhow::{Context as _, Result};

use crate::paths::resolve_wayland_display_path;

pub fn connect_from_env() -> Result<UnixStream> {
    let wayland_display = env::var("WAYLAND_DISPLAY").ok();
    let xdg_runtime_dir = env::var("XDG_RUNTIME_DIR").ok();
    let path = resolve_wayland_display_path(wayland_display.as_deref(), xdg_runtime_dir.as_deref());
    UnixStream::connect(&path).with_context(|| format!("connect Wayland socket {}", path.display()))
}
