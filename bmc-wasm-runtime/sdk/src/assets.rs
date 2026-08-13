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

//! Asset registration (icons, bitmaps, audio, meshes).
//!
//! On WASM targets, registration goes through host FFI calls.
//! On native targets (the gallery), callers must initialize pluggable registrars
//! via [`init_icon_registrar`], [`init_bitmap_registrar`], [`init_mesh_registrar`],
//! and [`init_image_registrar`] before any registration occurs.
//!
//! The host's registry is idempotent by `name` — the SDK keeps no consumer-side
//! cache; every call routes through the host, which returns the same ID for the
//! same `name`.

#![cfg_attr(target_arch = "wasm32", expect(clippy::cast_sign_loss))]

use bmc_wasm_protocol::{AudioId, BitmapId, MeshId, StaticAssetSource, SvgId};

#[cfg(target_arch = "wasm32")]
use crate::host;
use crate::mesh::Mesh;

#[cfg(not(target_arch = "wasm32"))]
type IconRegistrar = fn(&str, &[u8]) -> Option<SvgId>;
#[cfg(not(target_arch = "wasm32"))]
type BitmapRegistrar = fn(&str, &[u8]) -> Option<BitmapId>;
#[cfg(not(target_arch = "wasm32"))]
type MeshRegistrar = fn(&str, &[u8]) -> Option<MeshId>;
/// `(tag, rgba, width, height)` — pre-decoded pixels, upload only.
#[cfg(not(target_arch = "wasm32"))]
type ImageRegistrar = fn(&str, &[u8], u32, u32) -> Option<BitmapId>;

// ── Svg ─────────────────────────────────────────────────────────────

/// Compiled icon descriptor (output of `include_svg!`).
///
/// WASM builds keep the processed paths in the widget package; native builds
/// retain embedded bytes for storybook rendering.
#[derive(Debug)]
pub struct Svg {
    pub source: StaticAssetSource,
    pub name: &'static str,
    pub viewbox: (f32, f32),
}

impl Svg {
    /// The compiled-SVG viewBox `(width, height)`.
    /// The host scales X and Y independently, so a non-square glyph must be
    /// fitted by the caller (see [`crate::Draw::svg_contain`]) or it comes out
    /// stretched.
    #[must_use]
    pub fn viewbox(&self) -> (f32, f32) {
        self.viewbox
    }
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
        host::register_svg_package(icon.name, icon.source.package_ref())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        icon_native::ICON_REGISTRAR.with(|r| r.borrow()(icon.name, icon.source.data()))
    }
}

// ── Bitmap ───────────────────────────────────────────────────────────

/// Static raster image descriptor (output of `include_bitmap!`).
///
/// WASM builds load the encoded image from the widget package; native builds
/// retain embedded bytes for storybook rendering.
#[derive(Debug)]
pub struct Bitmap {
    pub source: StaticAssetSource,
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
        host::register_bitmap_package(bmp.name, bmp.source.package_ref())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        bitmap_native::BITMAP_REGISTRAR.with(|r| r.borrow()(bmp.name, bmp.source.data()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod image_native {
    use super::ImageRegistrar;
    use std::cell::RefCell;

    thread_local! {
        pub(super) static IMAGE_REGISTRAR: RefCell<ImageRegistrar> = RefCell::new(|_, _, _, _| panic!("BUG: image registrar not initialized — call init_image_registrar()"));
    }
}

/// Initialize the image registrar for native targets.
#[cfg(not(target_arch = "wasm32"))]
pub fn init_image_registrar(f: ImageRegistrar) {
    image_native::IMAGE_REGISTRAR.with(|r| *r.borrow_mut() = f);
}

/// Restore a bitmap from a cache source (e.g. `cache::lazy_get(tag)`).
///
/// On the device the RGBA goes mmap → texture entirely host-side;
/// natively the calling side's own store is read and the registrar
/// uploads. `None` on a cache miss or unreadable metadata.
#[must_use]
pub fn register_image(source: crate::cache::CacheSource<'_>) -> Option<BitmapId> {
    #[cfg(target_arch = "wasm32")]
    {
        host::register_bitmap_from_cache(source.tag())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let entry = crate::cache::read_bytes(source.tag())?;
        let (width, height, _identity) = bmc_wasm_protocol::decode_image_meta(&entry.metadata)?;
        image_native::IMAGE_REGISTRAR
            .with(|r| r.borrow()(source.tag(), &entry.bytes, width, height))
    }
}

// ── Audio ────────────────────────────────────────────────────────────

/// Static audio descriptor (output of `include_audio!`).
///
/// WASM builds load the sample from the widget package.
/// `name` remains the stable host registration tag.
#[derive(Debug)]
pub struct Audio {
    pub source: StaticAssetSource,
    pub name: &'static str,
}

/// Register an audio asset (host-side dedup by `audio.name`) and return its ID.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn ensure_audio_registered(audio: &Audio) -> Option<AudioId> {
    host::register_audio_package(audio.name, audio.source.package_ref())
}

/// Placeholder for native compilation (audio not used in the gallery).
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

/// Install dummy-id registrars for widget unit tests,
/// which assemble asset-bearing nodes without rendering them.
///
/// Per-thread, so call it at the top of each test.
/// Anything actually rendering (the gallery) must install real registrars instead.
#[cfg(not(target_arch = "wasm32"))]
pub fn init_test_registrars() {
    init_icon_registrar(|_, _| SvgId::from_wire(1));
    init_bitmap_registrar(|_, _| BitmapId::from_wire(1));
    init_mesh_registrar(|_, _| MeshId::from_wire(1));
    init_image_registrar(|_, _, _, _| BitmapId::from_wire(1));
}

/// Register a mesh (host-side dedup by `mesh.name`) and return its ID.
#[must_use]
pub fn ensure_mesh_registered(mesh: &Mesh) -> Option<MeshId> {
    #[cfg(target_arch = "wasm32")]
    {
        host::register_mesh_package(mesh.name, mesh.source.package_ref())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        mesh_native::MESH_REGISTRAR.with(|r| r.borrow()(mesh.name, mesh.source.data()))
    }
}

#[cfg(test)]
mod tests {
    use super::{init_test_registrars, register_image};
    use crate::cache;
    use bmc_wasm_protocol::encode_image_meta;

    #[test]
    fn a_cached_image_restores_through_the_registrar() {
        init_test_registrars();
        cache::put("img", &encode_image_meta(2, 2, b"url"), &[0_u8; 16]);
        assert!(register_image(cache::lazy_get("img")).is_some());
    }

    #[test]
    fn a_cache_miss_restores_nothing() {
        init_test_registrars();
        assert!(register_image(cache::lazy_get("absent")).is_none());
    }

    #[test]
    fn metadata_without_dims_restores_nothing() {
        init_test_registrars();
        cache::put("undersized", b"meta", b"payload");
        assert!(register_image(cache::lazy_get("undersized")).is_none());
    }
}
