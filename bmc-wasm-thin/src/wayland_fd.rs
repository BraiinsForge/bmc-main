// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

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
