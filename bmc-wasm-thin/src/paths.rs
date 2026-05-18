// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};

pub use bmc_wasm_thin_protocol::derive_lockfile_path;

#[must_use]
pub fn resolve_wayland_display_path(
    wayland_display: Option<&str>,
    xdg_runtime_dir: Option<&str>,
) -> PathBuf {
    let display = wayland_display.unwrap_or("wayland-0");
    if display.starts_with('/') {
        return PathBuf::from(display);
    }
    Path::new(xdg_runtime_dir.unwrap_or("/tmp/run")).join(display)
}
