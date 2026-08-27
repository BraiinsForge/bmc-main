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

//! Replace-style ephemeral asset slots.
//!
//! A `*Slot` owns a stable `&'static str` name and binds it to one
//! registration at a time. `set(data)` evicts any previous registration
//! under that name and registers the new payload; `evict()` drops the
//! current registration without re-registering.
//!
//! # When to use a Slot vs an `include_*!` macro
//!
//! - **Static asset** baked in at compile time (icons, UI bitmaps, sound
//!   effects that ship with the widget): use `include_bitmap!` /
//!   `include_svg!` / `include_mesh!` / `include_audio!` and the matching
//!   `ensure_*_registered` helper. The host dedups by the macro-emitted
//!   tag, so repeated calls are free.
//! - **Dynamic asset** whose bytes are fetched at runtime and change over
//!   time (album art, dynamically generated charts, downloaded skins):
//!   use a Slot. Each `set(bytes)` releases the previous payload's memory
//!   and GPU resources before registering the new one — without a Slot
//!   (or an equivalent manual `evict_prefix`), each variant would
//!   accumulate forever.
//!
//! Slot names participate in the host's segment-delimited namespace, so
//! `BitmapSlot::new("album_art")` and `BitmapSlot::new("album_art_thumb")`
//! coexist safely — eviction respects segment boundaries (`:`).
//!
//! Slots are wasm32-only. On native targets (the gallery) they panic; static
//! assets registered via `Bitmap`/`Svg`/`Mesh`/`Audio` cover the gallery
//! use case.

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_protocol::{AudioId, BitmapId, ImageJobId, MeshId, SvgId};

use crate::host;

macro_rules! define_slot {
    (
        $(#[$meta:meta])*
        $slot:ident => $id:ty, |$name:ident, $data:ident| $register:expr
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy)]
        pub struct $slot {
            name: &'static str,
        }

        impl $slot {
            #[must_use]
            pub const fn new(name: &'static str) -> Self {
                Self { name }
            }

            /// Replace the current registration under this slot's name with
            /// `data`. Any previously-registered payload (and, for audio,
            /// any in-flight playback sinks) are evicted first. Any ID returned
            /// by an earlier `set` is invalid after this call.
            #[must_use]
            pub fn set(&self, $data: &[u8]) -> Option<$id> {
                let $name = self.name;
                host::evict_prefix($name);
                $register
            }

            /// Drop the current registration under this slot's name. Any ID
            /// returned by an earlier `set` is invalid after this call.
            pub fn evict(&self) {
                host::evict_prefix(self.name);
            }

            #[must_use]
            pub const fn name(&self) -> &'static str {
                self.name
            }
        }
    };
}

define_slot! {
    /// Slot for a dynamic raster bitmap (PNG/JPEG bytes).
    BitmapSlot => BitmapId,
    |name, data| host::register_bitmap(name, data)
}

impl BitmapSlot {
    /// Decode `data` to fit within (`cover` false) or cover-crop to (`cover` true)
    /// `max_w`×`max_h`, off the render thread; `on_ready` fires when it replaces
    /// the slot's bitmap. `identity` is recorded in the per-instance asset cache
    /// so the host can restore the bitmap when a draw uses it.
    #[must_use]
    pub fn set_fit(
        &self,
        data: &[u8],
        max_w: u32,
        max_h: u32,
        cover: bool,
        identity: &[u8],
        on_ready: ImageReadyCallback,
    ) -> Option<ImageJobId> {
        let job_id = host::register_bitmap_fit(self.name, data, max_w, max_h, cover, identity)?;
        let idx = register_image_callback(on_ready);
        IMAGE_PENDING.with(|p| p.borrow_mut().insert(job_id, idx));
        Some(job_id)
    }
}

/// Async-decode result: the job handle, and `Some(id)` on success / `None` on failure.
pub type ImageReadyCallback = fn(ImageJobId, Option<BitmapId>);

thread_local! {
    static IMAGE_CALLBACKS: RefCell<Vec<ImageReadyCallback>> = const { RefCell::new(Vec::new()) };
    static IMAGE_PENDING: RefCell<HashMap<ImageJobId, usize>> = RefCell::new(HashMap::new());
}

/// Register a callback, deduping by function pointer; returns its index.
fn register_image_callback(cb: ImageReadyCallback) -> usize {
    IMAGE_CALLBACKS.with(|cbs| {
        let mut cbs = cbs.borrow_mut();
        for (i, existing) in cbs.iter().enumerate() {
            if *existing as usize == cb as usize {
                return i;
            }
        }
        let idx = cbs.len();
        cbs.push(cb);
        idx
    })
}

/// Host entry point: dispatch a finished decode to its `set_fit` callback.
#[unsafe(no_mangle)]
pub extern "C" fn __on_image_ready(job_id: u32, bitmap_id: u32) {
    let Some(job_id) = ImageJobId::from_wire(job_id) else {
        return;
    };
    let bitmap = BitmapId::from_ffi(bitmap_id);
    let cb = IMAGE_PENDING
        .with(|p| p.borrow_mut().remove(&job_id))
        .and_then(|idx| IMAGE_CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));
    if let Some(cb) = cb {
        cb(job_id, bitmap);
    }
}

/// Reclaim a decode's pending entry without dispatching — fired when the decode
/// finished while dormant (the result stays cached until a draw uses it).
#[unsafe(no_mangle)]
pub extern "C" fn __on_image_dropped(job_id: u32) {
    let Some(job_id) = ImageJobId::from_wire(job_id) else {
        return;
    };
    IMAGE_PENDING.with(|p| p.borrow_mut().remove(&job_id));
}

define_slot! {
    /// Slot for a dynamic raster bitmap rendered with nearest-neighbor
    /// filtering (pixel-art, 9-patch).
    BitmapNearestSlot => BitmapId,
    |name, data| host::register_bitmap_nearest(name, data)
}

define_slot! {
    /// Slot for a dynamic icon (compact binary SVG-path representation
    /// from the `include_svg!` macro's wire format).
    IconSlot => SvgId,
    |name, data| host::register_svg(name, data)
}

define_slot! {
    /// Slot for a dynamic mesh (compact binary mesh format from the
    /// `include_mesh!` macro's wire format).
    MeshSlot => MeshId,
    |name, data| host::register_mesh(name, data)
}

define_slot! {
    /// Slot for a dynamic audio sample (WAV/OGG/MP3 bytes).
    AudioSlot => AudioId,
    |name, data| host::register_audio(data, name)
}
