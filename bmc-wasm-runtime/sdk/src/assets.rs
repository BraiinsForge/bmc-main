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
//! On native targets (the gallery), the embedder installs an `AssetHost`
//! with `install_asset_host` before any registration occurs.
//!
//! The host's registry is idempotent by `name` — the SDK keeps no consumer-side
//! cache; every call routes through the host, which returns the same ID for the
//! same `name`.

use bmc_ui::{AudioId, BitmapId, MeshId, NinePatch, SkinEntry, SvgId};

use crate::cache::CacheSource;

trait AssetIdProvider {
    fn svg(icon: &Svg) -> Option<SvgId>;
    fn bitmap(bmp: &Bitmap) -> Option<BitmapId>;
    fn image(source: CacheSource<'_>) -> Option<BitmapId>;
    fn mesh(mesh: &Mesh) -> Option<MeshId>;
    fn audio(audio: &Audio) -> Option<AudioId>;
    fn skin_bitmap(tag: &str, source: Source) -> Option<BitmapId>;
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
pub use native::{AssetHost, init_test_registrars, install_asset_host};
#[cfg(not(target_arch = "wasm32"))]
use native::{Host as Platform, Source};
#[cfg(target_arch = "wasm32")]
pub use wasm::init_test_registrars;
#[cfg(target_arch = "wasm32")]
use wasm::{Host as Platform, Source};

pub type Svg = bmc_ui::Svg<Source>;
pub type Bitmap = bmc_ui::Bitmap<Source>;
pub type Audio = bmc_ui::Audio<Source>;
pub type Mesh = bmc_ui::Mesh<Source>;
pub type NinePatchAsset = bmc_ui::NinePatchAsset<Source>;
pub type SkinAsset = bmc_ui::SkinAsset<Source>;
pub type Skin = bmc_ui::Skin<Source>;

// ── Svg ─────────────────────────────────────────────────────────────

/// Register an icon (host-side dedup by `icon.name`) and return its ID.
#[must_use]
pub fn ensure_registered(icon: &Svg) -> Option<SvgId> {
    Platform::svg(icon)
}

// ── Bitmap ───────────────────────────────────────────────────────────

/// Register a bitmap (host-side dedup by `bmp.name`) and return its ID.
#[must_use]
pub fn ensure_bitmap_registered(bmp: &Bitmap) -> Option<BitmapId> {
    Platform::bitmap(bmp)
}

/// A skin bitmap, registered with nearest-neighbour filtering.
fn register_skin_bitmap(tag: &str, source: Source) -> Option<BitmapId> {
    Platform::skin_bitmap(tag, source)
}

/// Register a 9-patch asset with the host and return a [`NinePatch`].
///
/// Idempotent: the host dedups by `asset.name`, so a second call returns the
/// cached `BitmapId` without re-decoding/re-uploading.
#[must_use]
pub fn ensure_nine_patch_registered(asset: &NinePatchAsset) -> NinePatch {
    NinePatch {
        bitmap_id: register_skin_bitmap(asset.name, asset.source),
        left: asset.left,
        top: asset.top,
        right: asset.right,
        bottom: asset.bottom,
    }
}

/// A [`Skin`]'s image assets, registered with the host on lookup.
pub trait SkinRegistry {
    /// Look up a 9-patch asset by name and register it with the host if needed.
    ///
    /// Registration is idempotent host-side: the tag combines the skin name and
    /// the asset name so two skins with the same asset name don't alias.
    fn get_nine_patch(&self, name: &str) -> Option<SkinEntry>;

    /// Get the preview thumbnail asset, if this skin includes a `preview.png`.
    fn preview(&self) -> Option<SkinEntry> {
        self.get_nine_patch("preview")
    }
}

impl SkinRegistry for Skin {
    fn get_nine_patch(&self, name: &str) -> Option<SkinEntry> {
        let asset = self.asset(name)?;
        let mut tag = String::from("skin::");
        tag.push_str(self.name);
        tag.push_str("::");
        tag.push_str(asset.name);
        Some(SkinEntry {
            nine_patch: NinePatch {
                bitmap_id: register_skin_bitmap(&tag, asset.source),
                left: asset.left,
                top: asset.top,
                right: asset.right,
                bottom: asset.bottom,
            },
            width: asset.width,
            height: asset.height,
            color: asset.color,
        })
    }
}

/// Restore a bitmap from a cache source (e.g. `cache::lazy_get(tag)`).
///
/// On the device the RGBA goes mmap → texture entirely host-side;
/// natively the calling side's own store is read and the installed `AssetHost`
/// uploads. `None` on a cache miss or unreadable metadata.
#[must_use]
pub fn register_image(source: CacheSource<'_>) -> Option<BitmapId> {
    Platform::image(source)
}

// ── Audio ────────────────────────────────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::{init_test_registrars, register_image};
    use crate::cache;
    use crate::wasm_test;
    use bmc_wasm_protocol::encode_image_meta;

    #[wasm_test]
    fn a_cached_image_restores_through_the_registrar() {
        init_test_registrars();
        cache::put("img", &encode_image_meta(2, 2, b"url"), &[0_u8; 16]);
        assert!(register_image(cache::lazy_get("img")).is_some());
    }

    #[wasm_test]
    fn a_cache_miss_restores_nothing() {
        init_test_registrars();
        assert!(register_image(cache::lazy_get("absent")).is_none());
    }

    #[wasm_test]
    fn metadata_without_dims_restores_nothing() {
        init_test_registrars();
        cache::put("undersized", b"meta", b"payload");
        assert!(register_image(cache::lazy_get("undersized")).is_none());
    }
}
