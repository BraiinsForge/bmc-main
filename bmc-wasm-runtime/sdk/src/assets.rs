// Copyright (C) 2026  Braiins Systems s.r.o.

//! Asset registration (icons, bitmaps, audio, meshes).
//!
//! On WASM targets, registration goes through host FFI calls.
//! On native targets (storybook), callers must initialize pluggable registrars
//! via [`init_icon_registrar`], [`init_bitmap_registrar`], [`init_mesh_registrar`]
//! before any registration occurs.

#![cfg_attr(target_arch = "wasm32", expect(clippy::cast_sign_loss))]

use std::cell::RefCell;

use bmc_wasm_protocol::{BitmapId, IconId, MeshId};

#[cfg(target_arch = "wasm32")]
use crate::host;
use crate::mesh::Mesh;

/// Sentinel returned by host registration when the asset cannot be registered.
///
/// Failed registrations must NOT be cached, so the next call has a chance to
/// retry once the host is ready (e.g. after lazy GL initialisation). Audio is
/// the only asset that still uses the raw `u16` handle (no `AudioId` newtype yet);
/// the typed registrars use `IconId::NONE` / `BitmapId::NONE` / `MeshId::NONE`.
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) const INVALID_RESOURCE_ID: u16 = 0;

/// Look up `key` in `ids`; on miss, call `register` and only cache the result
/// if it is not the `invalid` sentinel. Centralises the "register-or-fetch"
/// pattern used by every asset type so the failure-cache rule cannot drift
/// per-asset.
///
/// Generic over the ID type so the typed `IconId` / `BitmapId` / `MeshId` newtypes
/// and the raw `u16` audio handle share one path.
pub(crate) fn cache_successful_resource_id<T, F>(
    ids: &mut Vec<(usize, T)>,
    key: usize,
    invalid: T,
    register: F,
) -> T
where
    T: Copy + Eq,
    F: FnOnce() -> T,
{
    for &(k, id) in ids.iter() {
        if k == key {
            return id;
        }
    }
    let id = register();
    if id != invalid {
        ids.push((key, id));
    }
    id
}

// ── Icon ─────────────────────────────────────────────────────────────

/// Compiled icon data (output of `include_icon!` proc macro).
///
/// The `data` field contains the compact binary representation of SVG paths
/// produced at compile time. On first use, this data is sent to the host via
/// `host_register_icon()` which returns an opaque ID used for rendering.
#[derive(Debug)]
pub struct Icon {
    pub data: &'static [u8],
}

thread_local! {
    static ICON_IDS: RefCell<Vec<(usize, IconId)>> = const { RefCell::new(Vec::new()) };
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static ICON_REGISTRAR: RefCell<fn(&[u8]) -> IconId> = RefCell::new(|_| panic!("BUG: icon registrar not initialized — call init_icon_registrar()"));
}

/// Initialize the icon registrar for native targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn init_icon_registrar(f: fn(&[u8]) -> IconId) {
    ICON_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Register an icon (if not already registered) and return its ID.
#[must_use]
pub fn ensure_registered(icon: &Icon) -> IconId {
    ICON_IDS.with(|ids| {
        cache_successful_resource_id(
            &mut ids.borrow_mut(),
            icon.data.as_ptr() as usize,
            IconId::NONE,
            || {
                #[cfg(target_arch = "wasm32")]
                {
                    host::register_icon(icon.data)
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ICON_REGISTRAR.with(|r| r.borrow()(icon.data))
                }
            },
        )
    })
}

// ── Bitmap ───────────────────────────────────────────────────────────

/// Embedded raster image data (output of `include_bitmap!` proc macro).
///
/// The `data` field contains raw PNG (or other image format) bytes embedded
/// at compile time. On first use, this data is sent to the host via
/// `host_register_bitmap()` which decodes it and uploads the texture to VRAM.
#[derive(Debug)]
pub struct Bitmap {
    pub data: &'static [u8],
}

thread_local! {
    static BITMAP_IDS: RefCell<Vec<(usize, BitmapId)>> = const { RefCell::new(Vec::new()) };
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static BITMAP_REGISTRAR: RefCell<fn(&[u8]) -> BitmapId> = RefCell::new(|_| panic!("BUG: bitmap registrar not initialized — call init_bitmap_registrar()"));
}

/// Initialize the bitmap registrar for native targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn init_bitmap_registrar(f: fn(&[u8]) -> BitmapId) {
    BITMAP_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Register a bitmap (if not already registered) and return its ID.
#[must_use]
pub fn ensure_bitmap_registered(bmp: &Bitmap) -> BitmapId {
    BITMAP_IDS.with(|ids| {
        cache_successful_resource_id(
            &mut ids.borrow_mut(),
            bmp.data.as_ptr() as usize,
            BitmapId::NONE,
            || {
                #[cfg(target_arch = "wasm32")]
                {
                    host::register_bitmap(bmp.data)
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    BITMAP_REGISTRAR.with(|r| r.borrow()(bmp.data))
                }
            },
        )
    })
}

// ── Audio ────────────────────────────────────────────────────────────

/// Embedded audio data (output of `include_audio!` proc macro).
///
/// The `data` field contains raw WAV/OGG/MP3 bytes embedded at compile time.
/// On first use, this data is sent to the host via `host_register_audio()`
/// which decodes to PCM and caches the samples for playback.
#[derive(Debug)]
pub struct Audio {
    pub data: &'static [u8],
    /// Human-readable name derived from filename (for fixture debugging).
    pub name: &'static str,
}

thread_local! {
    static AUDIO_IDS: RefCell<Vec<(usize, u16)>> = const { RefCell::new(Vec::new()) };
}

/// Register an audio asset (if not already registered) and return its ID.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn ensure_audio_registered(audio: &Audio) -> u16 {
    AUDIO_IDS.with(|ids| {
        cache_successful_resource_id(
            &mut ids.borrow_mut(),
            audio.data.as_ptr() as usize,
            INVALID_RESOURCE_ID,
            || host::register_audio(audio.data, audio.name),
        )
    })
}

/// Placeholder for native compilation (audio not used in storybook).
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn ensure_audio_registered(_audio: &Audio) -> u16 {
    panic!("ensure_audio_registered() is not available on native targets")
}

// ── Mesh ─────────────────────────────────────────────────────────────

thread_local! {
    static MESH_IDS: RefCell<Vec<(usize, MeshId)>> = const { RefCell::new(Vec::new()) };
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static MESH_REGISTRAR: RefCell<fn(&[u8]) -> MeshId> = RefCell::new(|_| panic!("BUG: mesh registrar not initialized — call init_mesh_registrar()"));
}

/// Initialize the mesh registrar for native targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn init_mesh_registrar(f: fn(&[u8]) -> MeshId) {
    MESH_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Register a mesh (if not already registered) and return its ID.
#[must_use]
pub fn ensure_mesh_registered(mesh: &Mesh) -> MeshId {
    MESH_IDS.with(|ids| {
        cache_successful_resource_id(
            &mut ids.borrow_mut(),
            mesh.data.as_ptr() as usize,
            MeshId::NONE,
            || {
                #[cfg(target_arch = "wasm32")]
                {
                    host::register_mesh(mesh.data)
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    MESH_REGISTRAR.with(|r| r.borrow()(mesh.data))
                }
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{INVALID_RESOURCE_ID, cache_successful_resource_id};

    #[test]
    fn successful_registrations_are_cached() {
        let mut ids = Vec::new();
        let mut calls = 0;

        let first = cache_successful_resource_id(&mut ids, 7, INVALID_RESOURCE_ID, || {
            calls += 1;
            11
        });
        let second = cache_successful_resource_id(&mut ids, 7, INVALID_RESOURCE_ID, || {
            calls += 1;
            12
        });

        assert_eq!(first, 11);
        assert_eq!(second, 11);
        assert_eq!(calls, 1);
    }

    #[test]
    fn failed_registrations_are_not_cached() {
        let mut ids = Vec::new();
        let mut responses = [INVALID_RESOURCE_ID, 9].into_iter();

        let first = cache_successful_resource_id(&mut ids, 42, INVALID_RESOURCE_ID, || {
            responses.next().expect("BUG: missing first test response")
        });
        let second = cache_successful_resource_id(&mut ids, 42, INVALID_RESOURCE_ID, || {
            responses.next().expect("BUG: missing second test response")
        });

        assert_eq!(first, INVALID_RESOURCE_ID);
        assert_eq!(second, 9);
    }
}
