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

//! EGL Compositor implementation for bmc-openwrt.

mod alarm;
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
mod settings;
mod state;
mod touch_gesture;
mod upgrade;
mod widget_tracker;

pub use commands::{CompositorCommand, CompositorResponse};
pub use device_access::DeviceAccessConfig;
pub use egl_compositor::EglCompositor;
