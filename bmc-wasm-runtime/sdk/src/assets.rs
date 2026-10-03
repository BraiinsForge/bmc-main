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
//! via `init_icon_registrar`, `init_bitmap_registrar`, `init_mesh_registrar`,
//! and `init_image_registrar` before any registration occurs.
//!
//! The host's registry is idempotent by `name` — the SDK keeps no consumer-side
//! cache; every call routes through the host, which returns the same ID for the
//! same `name`.

use bmc_wasm_protocol::{AudioId, BitmapId, MeshId, StaticAssetSource, SvgId};

use crate::cache::CacheSource;
use crate::mesh::Mesh;

trait AssetIdProvider {
    fn svg(icon: &Svg) -> Option<SvgId>;
    fn bitmap(bmp: &Bitmap) -> Option<BitmapId>;
    fn image(source: CacheSource<'_>) -> Option<BitmapId>;
    fn mesh(mesh: &Mesh) -> Option<MeshId>;
    fn audio(audio: &Audio) -> Option<AudioId>;
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;
#[cfg(not(target_arch = "wasm32"))]
pub use native::{
    init_bitmap_registrar, init_icon_registrar, init_image_registrar, init_mesh_registrar,
    init_test_registrars,
};
#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

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

/// Register an icon (host-side dedup by `icon.name`) and return its ID.
#[must_use]
pub fn ensure_registered(icon: &Svg) -> Option<SvgId> {
    Platform::svg(icon)
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

/// Register a bitmap (host-side dedup by `bmp.name`) and return its ID.
#[must_use]
pub fn ensure_bitmap_registered(bmp: &Bitmap) -> Option<BitmapId> {
    Platform::bitmap(bmp)
}

/// Restore a bitmap from a cache source (e.g. `cache::lazy_get(tag)`).
///
/// On the device the RGBA goes mmap → texture entirely host-side;
/// natively the calling side's own store is read and the registrar
/// uploads. `None` on a cache miss or unreadable metadata.
#[must_use]
pub fn register_image(source: CacheSource<'_>) -> Option<BitmapId> {
    Platform::image(source)
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
///
/// # Panics
///
/// Off-target, where audio is not used (the gallery plays none).
#[must_use]
pub fn ensure_audio_registered(audio: &Audio) -> Option<AudioId> {
    Platform::audio(audio)
}

// ── Mesh ─────────────────────────────────────────────────────────────

/// Register a mesh (host-side dedup by `mesh.name`) and return its ID.
#[must_use]
pub fn ensure_mesh_registered(mesh: &Mesh) -> Option<MeshId> {
    Platform::mesh(mesh)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
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
