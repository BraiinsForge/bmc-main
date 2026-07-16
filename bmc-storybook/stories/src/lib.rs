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

//! Hot-swappable stories cdylib.
//!
//! Compiled as a `.so` and loaded by the shell via `dlopen`. Exports a single
//! function `__story_entries()` that returns the collected story manifest.
//!
//! # Safety
//!
//! This crate must use `panic = "unwind"` (the default). The shell wraps calls
//! to story `render_fn` with `catch_unwind` — a `panic = "abort"` cdylib would
//! kill the entire process on any story panic.

// Prelude re-export so stories can `use crate::prelude::*`
pub mod prelude {
    pub use bmc_storybook_api::prelude::*;
}

// Story modules auto-discovered by build.rs from SDK src/*.stories.rs files
include!(concat!(env!("OUT_DIR"), "/stories.rs"));

use bmc_storybook_api::{StoryEntry, StoryGroupMeta, StoryManifest};

/// Initialize asset registrars in the cdylib's address space.
///
/// Thread-locals are per-shared-object, so the binary's `init_*_registrar`
/// calls only set the binary's copies. This function sets the cdylib's
/// copies so that `ensure_*_registered()` works from story code.
///
/// Called by the shell before each story render.
#[unsafe(no_mangle)]
pub extern "Rust" fn __init_registrars(
    icon: fn(&str, &[u8]) -> Option<bmc_wasm_sdk::SvgId>,
    bitmap: fn(&str, &[u8]) -> Option<bmc_wasm_sdk::BitmapId>,
    mesh: fn(&str, &[u8]) -> Option<bmc_wasm_sdk::MeshId>,
    bitmap_nearest: fn(&str, &[u8]) -> Option<bmc_wasm_sdk::BitmapId>,
) {
    bmc_wasm_sdk::assets::init_icon_registrar(icon);
    bmc_wasm_sdk::assets::init_bitmap_registrar(bitmap);
    bmc_wasm_sdk::assets::init_mesh_registrar(mesh);
    bmc_render_skin::init(bitmap_nearest);
}

/// Entry point called by the shell after `dlopen`.
///
/// Returns owned copies of all inventory-collected entries and groups.
/// The shell must convert `&'static str` fields to owned `String`s before
/// dropping the `Library`.
///
/// # Safety
///
/// Uses Rust calling convention (NOT `extern "C"`). `StoryManifest` contains
/// `Vec` which has no stable C ABI. Both sides must be compiled by the same
/// rustc with the same settings (guaranteed within a cargo workspace build).
#[unsafe(no_mangle)]
pub extern "Rust" fn __story_entries() -> StoryManifest {
    StoryManifest {
        entries: inventory::iter::<StoryEntry>.into_iter().copied().collect(),
        groups: inventory::iter::<StoryGroupMeta>
            .into_iter()
            .copied()
            .collect(),
    }
}
