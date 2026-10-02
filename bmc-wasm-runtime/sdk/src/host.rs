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

//! Host function bindings and types.
//!
//! FFI declarations and wrapper functions are gated to `wasm32` targets.
//! Pure types (`ButtonStyle`, `ButtonSize`, `SizeVariant`, `WidgetSize`,
//! `SystemTime`, `TouchHit`) are always available.

// Re-export from protocol — single source of truth for wire-format enums
#[cfg(target_arch = "wasm32")]
use crate::net::FetchBodyRef;
#[cfg(target_arch = "wasm32")]
use bmc_wasm_protocol::{AudioId, BitmapId, ImageJobId, MeshId, PackageAssetRef, SvgId};
pub use bmc_wasm_protocol::{ButtonSize, ButtonStyle};

mod clock;
pub use clock::SystemTime;

#[cfg(target_arch = "wasm32")]
mod random;
#[cfg(target_arch = "wasm32")]
pub use random::random_u32;

mod touch;
pub use touch::TouchHit;
#[cfg(target_arch = "wasm32")]
pub use touch::{get_touch_click, get_touch_drag};

mod viewport;
pub use viewport::{
    DisplayInfo, DisplayShape, SizeVariant, ViewportShape, WidgetSize, WidgetViewport,
    display_info, scale_font, widget_size, widget_viewport,
};

// ============================================================================
// FFI declarations and wrappers (wasm32 only)
// ============================================================================

#[cfg(target_arch = "wasm32")]
mod ffi {
    use super::*;

    // Names the module the host registers these under. Without it rustc 1.96+
    // leaves the block as undefined symbols instead of imports, and lld fails.
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_fill_rect(x: i32, y: i32, w: u32, h: u32, color: u32);
        fn host_draw_text(
            text_ptr: *const u8,
            text_len: u32,
            x: i32,
            y: i32,
            size: u32,
            color: u32,
        );
        fn host_request_frame();
        fn host_request_frame_after(delay_ms: u32);
        fn host_button(
            key_ptr: *const u8,
            key_len: u32,
            label_ptr: *const u8,
            label_len: u32,
            x: i32,
            y: i32,
            w: u32,
            h: u32,
            style: u32,
        ) -> i32;

        // New tree-based API
        fn host_submit_tree(ptr: *const u8, len: u32, width: u32, height: u32);

        // Svg registration. The host dedups by tag.
        pub(super) fn host_register_svg(
            tag_ptr: *const u8,
            tag_len: u32,
            data_ptr: *const u8,
            data_len: u32,
        ) -> u32;
        pub(super) fn host_register_svg_package(
            tag_ptr: *const u8,
            tag_len: u32,
            id_ptr: *const u8,
        ) -> u32;
        // Bitmap registration. The host dedups by tag.
        pub(super) fn host_register_bitmap(
            tag_ptr: *const u8,
            tag_len: u32,
            data_ptr: *const u8,
            data_len: u32,
        ) -> u32;
        pub(super) fn host_register_bitmap_nearest(
            tag_ptr: *const u8,
            tag_len: u32,
            data_ptr: *const u8,
            data_len: u32,
        ) -> u32;
        pub(super) fn host_register_bitmap_package(
            tag_ptr: *const u8,
            tag_len: u32,
            id_ptr: *const u8,
        ) -> u32;
        pub(super) fn host_register_bitmap_nearest_package(
            tag_ptr: *const u8,
            tag_len: u32,
            id_ptr: *const u8,
        ) -> u32;
        pub(super) fn host_register_bitmap_fit(
            tag_ptr: *const u8,
            tag_len: u32,
            data_ptr: *const u8,
            data_len: u32,
            max_w: u32,
            max_h: u32,
            cover: u32,
            identity_ptr: *const u8,
            identity_len: u32,
        ) -> u32;
        pub(super) fn host_register_bitmap_fit_ref(
            tag_ptr: *const u8,
            tag_len: u32,
            request_id: u32,
            max_w: u32,
            max_h: u32,
            cover: u32,
            identity_ptr: *const u8,
            identity_len: u32,
        ) -> u32;
        pub(super) fn host_register_bitmap_from_cache(tag_ptr: *const u8, tag_len: u32) -> u32;

        // Mesh registration. The host dedups by tag.
        fn host_register_mesh(
            tag_ptr: *const u8,
            tag_len: u32,
            data_ptr: *const u8,
            data_len: u32,
        ) -> u32;
        fn host_register_mesh_package(tag_ptr: *const u8, tag_len: u32, id_ptr: *const u8) -> u32;
        // Audio registration and playback
        fn host_register_audio(
            data_ptr: *const u8,
            data_len: u32,
            name_ptr: *const u8,
            name_len: u32,
        ) -> u32;
        fn host_register_audio_package(
            name_ptr: *const u8,
            name_len: u32,
            id_ptr: *const u8,
        ) -> u32;
        fn host_audio_play(sound_id: u32, volume: u32);
        fn host_audio_stop(sound_id: u32);

        // Image decoding (returns RGBA pixels)
        fn host_decode_image(
            data_ptr: *const u8,
            data_len: u32,
            rgba_out_ptr: *mut u8,
            rgba_out_cap: u32,
        ) -> i64;
        fn host_image_dimensions_ref(request_id: u32, max_source_pixels_out: *mut u64) -> i64;

        // Tag-prefix eviction across icon, bitmap, mesh, and audio registries.
        fn host_evict_prefix(prefix_ptr: *const u8, prefix_len: u32) -> u32;

        // Evict this widget's entire namespace (all icons, bitmaps, meshes, audio).
        fn host_evict_all() -> u32;

        // Max image size (pixels) the host will decode.
        fn host_max_image_pixels() -> u32;
    }

    /// Fill a rectangle with a solid color.
    pub fn fill_rect(x: i32, y: i32, w: u32, h: u32, color: u32) {
        unsafe { host_fill_rect(x, y, w, h, color) }
    }

    /// Draw text at position.
    pub fn draw_text(text: &[u8], x: i32, y: i32, size: u32, color: u32) {
        unsafe { host_draw_text(text.as_ptr(), text.len() as u32, x, y, size, color) }
    }

    /// Request next frame immediately.
    pub fn request_frame() {
        unsafe { host_request_frame() }
    }

    /// Request next frame after delay.
    pub fn request_frame_after(delay_ms: u32) {
        unsafe { host_request_frame_after(delay_ms) }
    }

    /// Draw a styled button with label and check for click.
    /// Returns `true` the frame the button was clicked.
    #[must_use]
    pub fn button(
        key: &[u8],
        label: &[u8],
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        style: ButtonStyle,
    ) -> bool {
        unsafe {
            host_button(
                key.as_ptr(),
                key.len() as u32,
                label.as_ptr(),
                label.len() as u32,
                x,
                y,
                w,
                h,
                style as u32,
            ) != 0
        }
    }

    /// Submit a serialized tree for host-side layout and rendering.
    pub fn submit_tree(data: &[u8], width: u32, height: u32) {
        unsafe { host_submit_tree(data.as_ptr(), data.len() as u32, width, height) }
    }

    /// Register icon data with the host under `tag`. Idempotent host-side.
    /// Wire `0` lifts to `None`.
    #[expect(clippy::cast_possible_truncation)]
    #[must_use]
    pub fn register_svg(tag: &str, data: &[u8]) -> Option<SvgId> {
        SvgId::from_ffi(unsafe {
            host_register_svg(
                tag.as_ptr(),
                tag.len() as u32,
                data.as_ptr(),
                data.len() as u32,
            )
        })
    }

    #[must_use]
    pub fn register_svg_package(tag: &str, package_ref: &PackageAssetRef) -> Option<SvgId> {
        SvgId::from_ffi(unsafe {
            host_register_svg_package(
                tag.as_ptr(),
                tag.len() as u32,
                package_ref.as_bytes().as_ptr(),
            )
        })
    }

    /// Register mesh data (optimized binary format) with the host under `tag`.
    /// Idempotent host-side; first call uploads VBO/IBO/texture to GPU.
    #[expect(clippy::cast_possible_truncation)]
    #[must_use]
    pub fn register_mesh(tag: &str, data: &[u8]) -> Option<MeshId> {
        MeshId::from_ffi(unsafe {
            host_register_mesh(
                tag.as_ptr(),
                tag.len() as u32,
                data.as_ptr(),
                data.len() as u32,
            )
        })
    }

    #[must_use]
    pub fn register_mesh_package(tag: &str, package_ref: &PackageAssetRef) -> Option<MeshId> {
        MeshId::from_ffi(unsafe {
            host_register_mesh_package(
                tag.as_ptr(),
                tag.len() as u32,
                package_ref.as_bytes().as_ptr(),
            )
        })
    }

    /// Register audio data (WAV/OGG/MP3 bytes) with the host.
    #[expect(clippy::cast_possible_truncation)]
    #[must_use]
    pub fn register_audio(data: &[u8], name: &str) -> Option<AudioId> {
        AudioId::from_ffi(unsafe {
            host_register_audio(
                data.as_ptr(),
                data.len() as u32,
                name.as_ptr(),
                name.len() as u32,
            )
        })
    }

    #[must_use]
    pub fn register_audio_package(name: &str, package_ref: &PackageAssetRef) -> Option<AudioId> {
        AudioId::from_ffi(unsafe {
            host_register_audio_package(
                name.as_ptr(),
                name.len() as u32,
                package_ref.as_bytes().as_ptr(),
            )
        })
    }

    /// Play a registered audio sample at the given [`Volume`].
    ///
    /// Fire-and-forget: the host mixes and plays asynchronously. `None`
    /// no-ops, so callers can thread an `Option<AudioId>` straight from
    /// `ensure_audio_registered` without unwrapping.
    pub fn audio_play(sound_id: Option<AudioId>, volume: super::Volume) {
        let Some(id) = sound_id else { return };
        unsafe { host_audio_play(id.to_ffi(), u32::from(volume)) }
    }

    /// Stop playback of a registered audio sample. `None` no-ops.
    pub fn audio_stop(sound_id: Option<AudioId>) {
        let Some(id) = sound_id else { return };
        unsafe { host_audio_stop(id.to_ffi()) }
    }

    /// Register bitmap data (PNG bytes) with the host under `tag`. Idempotent
    /// host-side.
    #[expect(clippy::cast_possible_truncation)]
    #[must_use]
    pub fn register_bitmap(tag: &str, data: &[u8]) -> Option<BitmapId> {
        BitmapId::from_ffi(unsafe {
            host_register_bitmap(
                tag.as_ptr(),
                tag.len() as u32,
                data.as_ptr(),
                data.len() as u32,
            )
        })
    }

    #[must_use]
    pub fn register_bitmap_package(tag: &str, package_ref: &PackageAssetRef) -> Option<BitmapId> {
        BitmapId::from_ffi(unsafe {
            host_register_bitmap_package(
                tag.as_ptr(),
                tag.len() as u32,
                package_ref.as_bytes().as_ptr(),
            )
        })
    }

    /// Register bitmap data with nearest-neighbor filtering (no bilinear
    /// interpolation) under `tag`. Idempotent host-side.
    #[expect(clippy::cast_possible_truncation)]
    #[must_use]
    pub fn register_bitmap_nearest(tag: &str, data: &[u8]) -> Option<BitmapId> {
        BitmapId::from_ffi(unsafe {
            host_register_bitmap_nearest(
                tag.as_ptr(),
                tag.len() as u32,
                data.as_ptr(),
                data.len() as u32,
            )
        })
    }

    #[must_use]
    pub fn register_bitmap_nearest_package(
        tag: &str,
        package_ref: &PackageAssetRef,
    ) -> Option<BitmapId> {
        BitmapId::from_ffi(unsafe {
            host_register_bitmap_nearest_package(
                tag.as_ptr(),
                tag.len() as u32,
                package_ref.as_bytes().as_ptr(),
            )
        })
    }

    /// Decode + downscale `data` to fit `max_w`×`max_h` off the render thread.
    /// The bitmap is delivered later via `__on_image_ready`; returns the job
    /// handle (`None` = rejected).
    #[expect(clippy::cast_possible_truncation)]
    #[must_use]
    pub fn register_bitmap_fit(
        tag: &str,
        data: &[u8],
        max_w: u32,
        max_h: u32,
        cover: bool,
        identity: &[u8],
    ) -> Option<ImageJobId> {
        ImageJobId::from_wire(unsafe {
            host_register_bitmap_fit(
                tag.as_ptr(),
                tag.len() as u32,
                data.as_ptr(),
                data.len() as u32,
                max_w,
                max_h,
                u32::from(cover),
                identity.as_ptr(),
                identity.len() as u32,
            )
        })
    }

    /// Start a fitted bitmap decode from a callback-scoped host fetch body.
    /// Rejection returns the body reference for another operation.
    #[must_use]
    pub fn register_bitmap_fit_ref<'a>(
        tag: &str,
        body: FetchBodyRef<'a>,
        max_w: u32,
        max_h: u32,
        cover: bool,
        identity: &[u8],
    ) -> Result<ImageJobId, FetchBodyRef<'a>> {
        let job_id = ImageJobId::from_wire(unsafe {
            host_register_bitmap_fit_ref(
                tag.as_ptr(),
                tag.len() as u32,
                body.request_id_wire(),
                max_w,
                max_h,
                u32::from(cover),
                identity.as_ptr(),
                identity.len() as u32,
            )
        });
        job_id.ok_or(body)
    }

    /// Restore a bitmap from its per-instance cache entry
    /// (host-side mmap → texture; no bytes cross into wasm).
    /// `None` on a miss.
    #[must_use]
    pub fn register_bitmap_from_cache(tag: &str) -> Option<BitmapId> {
        BitmapId::from_ffi(unsafe {
            host_register_bitmap_from_cache(tag.as_ptr(), tag.len() as u32)
        })
    }

    /// Drop every host-side asset (icon, bitmap, mesh, audio) whose tag
    /// starts with `prefix`. The host implicitly namespaces by guest ID,
    /// so the prefix only sees this widget's own registrations.
    /// IDs for evicted assets become invalid and must be discarded.
    /// Returns the number of entries evicted across all four registries.
    #[expect(clippy::cast_possible_truncation)]
    pub fn evict_prefix(prefix: &str) -> u32 {
        unsafe { host_evict_prefix(prefix.as_ptr(), prefix.len() as u32) }
    }

    /// Evict everything this widget registered — its whole namespace.
    /// All previously returned asset IDs become invalid and must be discarded.
    /// Returns the number of entries evicted.
    pub fn evict_all() -> u32 {
        unsafe { host_evict_all() }
    }

    /// Get the dimensions of an image (PNG, JPEG, etc.) without decoding the full pixel data.
    pub fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
        let packed = unsafe {
            host_decode_image(data.as_ptr(), data.len() as u32, core::ptr::null_mut(), 0)
        };
        if packed < 0 {
            return None;
        }
        let w = (packed >> 32) as u32;
        let h = (packed & 0xFFFF_FFFF) as u32;
        if w == 0 || h == 0 {
            return None;
        }
        Some((w, h))
    }

    /// Dimensions and the format-aware source-pixel allowance for a retained image.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct RetainedImageDimensions {
        /// Declared image width.
        pub width: u32,
        /// Declared image height.
        pub height: u32,
        /// Source-pixel ceiling for the host-detected image format.
        pub max_source_pixels: u64,
    }

    /// Get image dimensions without copying a retained fetch body into WASM.
    /// The host-detected format determines the returned source-pixel allowance.
    /// Callers must reject images whose width times height exceeds that
    /// allowance before registering a decode.
    #[must_use]
    pub fn image_dimensions_ref(body: &FetchBodyRef<'_>) -> Option<RetainedImageDimensions> {
        let mut max_source_pixels = 0;
        let packed = unsafe {
            host_image_dimensions_ref(body.request_id_wire(), &raw mut max_source_pixels)
        };
        if packed < 0 {
            return None;
        }
        let width = (packed >> 32) as u32;
        let height = (packed & 0xFFFF_FFFF) as u32;
        if width == 0 || height == 0 {
            return None;
        }
        Some(RetainedImageDimensions {
            width,
            height,
            max_source_pixels,
        })
    }

    /// Decode image data (PNG, JPEG, etc.) to RGBA pixels on the host.
    pub fn decode_image(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
        let (w, h) = image_dimensions(data)?;
        let needed = w * h * 4;

        let mut buf = vec![0u8; needed as usize];
        let packed2 = unsafe {
            host_decode_image(data.as_ptr(), data.len() as u32, buf.as_mut_ptr(), needed)
        };
        if packed2 < 0 {
            return None;
        }
        Some((buf, w, h))
    }

    /// Maximum image size, in pixels, the host will decode.
    #[must_use]
    pub fn max_image_pixels() -> u32 {
        unsafe { host_max_image_pixels() }
    }
}

#[cfg(target_arch = "wasm32")]
pub use ffi::*;

// ============================================================================
// Pure types (always available)
// ============================================================================

/// Audio playback volume, in 0–100.
///
/// Values are clamped at construction, so the host never sees an
/// out-of-range byte. Use [`Volume::SILENT`] / [`Volume::FULL`] for the
/// canonical endpoints, [`Volume::new`] for arbitrary values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Volume(u8);

impl Volume {
    pub const SILENT: Volume = Volume(0);
    pub const FULL: Volume = Volume(100);

    /// Build a volume in 0–100. Values above 100 are clamped down to 100.
    #[must_use]
    pub const fn new(v: u8) -> Self {
        Self(if v > 100 { 100 } else { v })
    }
}

impl From<Volume> for u32 {
    fn from(v: Volume) -> Self {
        Self::from(v.0)
    }
}
