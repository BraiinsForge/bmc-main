// Copyright (C) 2026  Braiins Systems s.r.o.

//! Asset registration (icons, bitmaps, audio, meshes).
//!
//! On WASM targets, registration goes through host FFI calls.
//! On native targets (storybook), callers must initialize pluggable registrars
//! via [`init_icon_registrar`], [`init_bitmap_registrar`], [`init_mesh_registrar`]
//! before any registration occurs.
//!
//! The host's registry is idempotent by `name` — the SDK keeps no consumer-side
//! cache; every call routes through the host, which returns the same ID for the
//! same `name`.

#![cfg_attr(target_arch = "wasm32", expect(clippy::cast_sign_loss))]

use bmc_wasm_protocol::{AudioId, BitmapId, MeshId, SvgId};

#[cfg(target_arch = "wasm32")]
use crate::host;
use crate::mesh::Mesh;

#[cfg(not(target_arch = "wasm32"))]
type IconRegistrar = fn(&str, &[u8]) -> Option<SvgId>;
#[cfg(not(target_arch = "wasm32"))]
type BitmapRegistrar = fn(&str, &[u8]) -> Option<BitmapId>;
#[cfg(not(target_arch = "wasm32"))]
type MeshRegistrar = fn(&str, &[u8]) -> Option<MeshId>;

// ── Svg ─────────────────────────────────────────────────────────────

/// Compiled icon data (output of `include_svg!` proc macro).
///
/// `data` is the compact binary representation of SVG paths produced at
/// compile time. `name` is a stable, host-unique tag (typically
/// `"<crate>::<file_stem>"`) used by the host to dedup registrations.
#[derive(Debug)]
pub struct Svg {
    pub data: &'static [u8],
    pub name: &'static str,
}

#[cfg(not(target_arch = "wasm32"))]
mod icon_native {
    use super::IconRegistrar;
    use std::cell::RefCell;

    thread_local! {
        pub(super) static ICON_REGISTRAR: RefCell<IconRegistrar> = RefCell::new(|_, _| panic!("BUG: icon registrar not initialized — call init_icon_registrar()"));
    }
}

/// Initialize the icon registrar for native targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn init_icon_registrar(f: IconRegistrar) {
    icon_native::ICON_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Register an icon (host-side dedup by `icon.name`) and return its ID.
#[must_use]
pub fn ensure_registered(icon: &Svg) -> Option<SvgId> {
    #[cfg(target_arch = "wasm32")]
    {
        host::register_svg(icon.name, icon.data)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        icon_native::ICON_REGISTRAR.with(|r| r.borrow()(icon.name, icon.data))
    }
}

// ── Bitmap ───────────────────────────────────────────────────────────

/// Embedded raster image data (output of `include_bitmap!` proc macro).
///
/// `data` is the raw image bytes (PNG/JPEG/etc.). `name` is a stable,
/// host-unique tag (typically `"<crate>::<file_stem>"`) used by the host to
/// dedup registrations.
#[derive(Debug)]
pub struct Bitmap {
    pub data: &'static [u8],
    pub name: &'static str,
}

#[cfg(not(target_arch = "wasm32"))]
mod bitmap_native {
    use super::BitmapRegistrar;
    use std::cell::RefCell;

    thread_local! {
        pub(super) static BITMAP_REGISTRAR: RefCell<BitmapRegistrar> = RefCell::new(|_, _| panic!("BUG: bitmap registrar not initialized — call init_bitmap_registrar()"));
    }
}

/// Initialize the bitmap registrar for native targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn init_bitmap_registrar(f: BitmapRegistrar) {
    bitmap_native::BITMAP_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Register a bitmap (host-side dedup by `bmp.name`) and return its ID.
#[must_use]
pub fn ensure_bitmap_registered(bmp: &Bitmap) -> Option<BitmapId> {
    #[cfg(target_arch = "wasm32")]
    {
        host::register_bitmap(bmp.name, bmp.data)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        bitmap_native::BITMAP_REGISTRAR.with(|r| r.borrow()(bmp.name, bmp.data))
    }
}

// ── Audio ────────────────────────────────────────────────────────────

/// Embedded audio data (output of `include_audio!` proc macro).
///
/// `data` is the raw audio bytes (WAV/OGG/MP3). `name` is a stable,
/// host-unique tag used by the host to dedup registrations and for fixture
/// debugging.
#[derive(Debug)]
pub struct Audio {
    pub data: &'static [u8],
    pub name: &'static str,
}

/// Register an audio asset (host-side dedup by `audio.name`) and return its ID.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn ensure_audio_registered(audio: &Audio) -> Option<AudioId> {
    host::register_audio(audio.data, audio.name)
}

/// Placeholder for native compilation (audio not used in storybook).
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn ensure_audio_registered(_audio: &Audio) -> Option<AudioId> {
    panic!("ensure_audio_registered() is not available on native targets")
}

// ── Mesh ─────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod mesh_native {
    use super::MeshRegistrar;
    use std::cell::RefCell;

    thread_local! {
        pub(super) static MESH_REGISTRAR: RefCell<MeshRegistrar> = RefCell::new(|_, _| panic!("BUG: mesh registrar not initialized — call init_mesh_registrar()"));
    }
}

/// Initialize the mesh registrar for native targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn init_mesh_registrar(f: MeshRegistrar) {
    mesh_native::MESH_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Register a mesh (host-side dedup by `mesh.name`) and return its ID.
#[must_use]
pub fn ensure_mesh_registered(mesh: &Mesh) -> Option<MeshId> {
    #[cfg(target_arch = "wasm32")]
    {
        host::register_mesh(mesh.name, mesh.data)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        mesh_native::MESH_REGISTRAR.with(|r| r.borrow()(mesh.name, mesh.data))
    }
}
