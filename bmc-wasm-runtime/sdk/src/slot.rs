// Copyright (C) 2026  Braiins Systems s.r.o.

//! Replace-style ephemeral asset slots.
//!
//! A `*Slot` owns a stable `&'static str` name and binds it to one
//! registration at a time. `set(data)` evicts any previous registration
//! under that name and registers the new payload; `evict()` drops the
//! current registration without re-registering.
//!
//! Use for assets whose content changes during the widget's lifetime —
//! album art, dynamic charts, anything you'd otherwise leak by registering
//! each variant under a fresh tag.
//!
//! Slot names participate in the host's segment-delimited namespace, so
//! `BitmapSlot::new("album_art")` and `BitmapSlot::new("album_art_thumb")`
//! coexist safely — eviction respects segment boundaries (`:`).
//!
//! Slots are wasm32-only. On native targets (storybook) they panic; static
//! assets registered via `Bitmap`/`Icon`/`Mesh`/`Audio` cover the storybook
//! use case.

#![cfg(target_arch = "wasm32")]

use bmc_wasm_protocol::{AudioId, BitmapId, IconId, MeshId};

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
            /// any in-flight playback sinks) are evicted first.
            #[must_use]
            pub fn set(&self, $data: &[u8]) -> Option<$id> {
                let $name = self.name;
                host::evict_prefix($name);
                $register
            }

            /// Drop the current registration under this slot's name without
            /// re-registering.
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

define_slot! {
    /// Slot for a dynamic raster bitmap rendered with nearest-neighbor
    /// filtering (pixel-art, 9-patch).
    BitmapNearestSlot => BitmapId,
    |name, data| host::register_bitmap_nearest(name, data)
}

define_slot! {
    /// Slot for a dynamic icon (compact binary SVG-path representation
    /// from the `include_icon!` macro's wire format).
    IconSlot => IconId,
    |name, data| host::register_icon(name, data)
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
