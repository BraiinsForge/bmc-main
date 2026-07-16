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

//! Storybook dev tool for WASM widget SDK components.
//!
//! Provides a visual reference and interactive playground for widget
//! developers. Stories are collected at startup via `inventory` and
//! rendered through the real FemtoVG rendering pipeline.
//!
//! Supports two modes:
//! - **Static**: stories compiled in via `include!()` (default, `make storybook`)
//! - **Hot-reload**: stories loaded from cdylib .so (`--hot-reload`, `make storybook-hot`)

mod ansi;
mod app;
pub mod hot_reload;
mod icons;
mod knobs_ui;
mod preview;
mod sidebar;
// Story modules auto-discovered by build.rs from SDK src/*.stories.rs files
include!(concat!(env!("OUT_DIR"), "/stories.rs"));

pub mod prelude;

pub use app::StorybookApp;
pub use bmc_storybook_api::knobs::StoryCtx;
pub use bmc_storybook_api::{StoryEntry, StoryGroupMeta};

// Re-export inventory so story_meta! / #[story] work from included story files.
#[doc(hidden)]
pub use inventory;

/// Convert a protocol `Color` to an egui `Color32` (preserving alpha).
pub(crate) fn to_egui(c: bmc_render::colors::Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.red(), c.green(), c.blue(), c.alpha())
}
