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

        // System time
        pub(super) fn host_get_system_time(out_ptr: *mut u8);

        // Widget viewport dimensions, packed as `(width << 32) | height`.
        pub(super) fn host_widget_size() -> u64;

        // Widget viewport shape from configure, as a wire u32.
        pub(super) fn host_widget_viewport_shape() -> u32;
        // Logical display size, packed `(width << 32) | height`.
        pub(super) fn host_display_size() -> u64;
        // Display shape (wire u32) and dpi, packed `(shape << 32) | dpi`.
        pub(super) fn host_display_shape_dpi() -> u64;

        // Date parsing
        fn host_parse_date(str_ptr: *const u8, str_len: u32) -> i64;

        // New tree-based API
        fn host_submit_tree(ptr: *const u8, len: u32, width: u32, height: u32);
        fn host_get_touch_click(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8) -> i32;
        fn host_get_touch_drag(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8) -> i32;

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

        // Random number generation (host-seeded for deterministic replay)
        fn host_random_u32() -> u32;

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

    /// Get the click position for an interactive canvas (one-shot, on finger-up).
    ///
    /// Returns `None` if the canvas was not clicked this frame.
    #[must_use]
    pub fn get_touch_click(key: &str) -> Option<TouchHit> {
        let mut buf = [0u8; 16];
        let clicked =
            unsafe { host_get_touch_click(key.as_ptr(), key.len() as u32, buf.as_mut_ptr()) };
        if clicked != 0 {
            Some(TouchHit::from_buf(&buf))
        } else {
            None
        }
    }

    /// Get the drag position for an interactive canvas (continuous, while finger is down).
    ///
    /// Returns `None` if the canvas is not being dragged this frame.
    #[must_use]
    pub fn get_touch_drag(key: &str) -> Option<TouchHit> {
        let mut buf = [0u8; 16];
        let dragging =
            unsafe { host_get_touch_drag(key.as_ptr(), key.len() as u32, buf.as_mut_ptr()) };
        if dragging != 0 {
            Some(TouchHit::from_buf(&buf))
        } else {
            None
        }
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

    /// Parse an ISO 8601 date string (e.g. "2026-02-13T10:15:56Z") into a unix timestamp.
    ///
    /// Returns `None` if the string is not a valid date.
    #[must_use]
    pub fn parse_date(s: &str) -> Option<i64> {
        let val = unsafe { host_parse_date(s.as_ptr(), s.len() as u32) };
        if val == i64::MIN { None } else { Some(val) }
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

    /// Get a random `u32` from the host.
    #[must_use]
    pub fn random_u32() -> u32 {
        unsafe { host_random_u32() }
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

// Re-export the shared wire enums so widgets match on the SDK's own path.
pub use bmc_wasm_protocol::{DisplayShape, ViewportShape};

/// The drawable rectangle assigned to this widget, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetViewport {
    pub width: u32,
    pub height: u32,
    pub shape: ViewportShape,
}

/// The logical display this widget's viewport lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub shape: DisplayShape,
    pub dpi: u32,
}

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

/// Known widget size variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeVariant {
    Full,   // 1280×480
    Large,  //  638×480
    Medium, //  638×238
    Small,  //  317×238
}

impl SizeVariant {
    /// Canonical pixel width for this variant.
    #[must_use]
    pub const fn width(self) -> u32 {
        match self {
            Self::Full => 1_280,
            Self::Large | Self::Medium => 638,
            Self::Small => 317,
        }
    }

    /// Canonical pixel height for this variant.
    #[must_use]
    pub const fn height(self) -> u32 {
        match self {
            Self::Full | Self::Large => 480,
            Self::Medium | Self::Small => 238,
        }
    }

    /// All variants. Order is irrelevant to the result — the comparator below
    /// gives a total order — but keeping it explicit documents the candidate set.
    const ALL: [SizeVariant; 4] = [
        SizeVariant::Small,
        SizeVariant::Medium,
        SizeVariant::Large,
        SizeVariant::Full,
    ];

    /// Pixel area of this variant's canonical dimensions.
    #[must_use]
    const fn area(self) -> u32 {
        self.width() * self.height()
    }

    /// Normalized distance from `(w, h)` to this variant's canonical size:
    /// `|w-vw|/vw + |h-vh|/vh`. Zero on an exact dimensional match.
    #[must_use]
    fn distance_from(self, w: u32, h: u32) -> f64 {
        let vw = f64::from(self.width());
        let vh = f64::from(self.height());
        (f64::from(w) - vw).abs() / vw + (f64::from(h) - vh).abs() / vh
    }

    /// Classify a viewport to the closest BMC100 variant.
    ///
    /// Minimize the normalized distance; on a genuine distance tie, prefer the
    /// larger-area variant so a non-BMC100 fullscreen display never collapses
    /// to a compact layout. An exact dimensional match has distance 0 and so
    /// always wins. Deterministic for identical inputs.
    #[must_use]
    pub fn closest(w: u32, h: u32) -> Self {
        // Destructuring the fixed-size array seeds the fold from a concrete
        // variant, so the result is `Self` rather than `Option<Self>` and the
        // never-empty invariant is enforced at compile time.
        let [first, rest @ ..] = Self::ALL;
        rest.into_iter().fold(first, |best, cand| {
            // `total_cmp` orders by distance without a float `==`
            // (which `clippy::float_cmp` rejects under -D warnings);
            // the `.then` tie-break makes the larger-area variant compare
            // as "less" so it wins an exact distance tie. Variant areas
            // are all distinct, so the comparator is never `Equal` and the
            // result is fully deterministic.
            if cand
                .distance_from(w, h)
                .total_cmp(&best.distance_from(w, h))
                .then(best.area().cmp(&cand.area()))
                .is_lt()
            {
                cand
            } else {
                best
            }
        })
    }
}

/// Diameter of the round BFM100 face, in pixels.
const ROUND_FACE_PX: f32 = 480.0;

/// Widget viewport dimensions and size variant.
///
/// Created from the raw `(width, height)` the host passes to `init()`.
/// Carries both the classified variant (for layout matching) and the
/// actual pixel dimensions (for `render_ui`).
#[derive(Debug, Clone, Copy)]
pub struct WidgetSize {
    pub variant: SizeVariant,
    pub width: u32,
    pub height: u32,
}

impl WidgetSize {
    #[must_use]
    pub fn from_dimensions(w: u32, h: u32) -> Self {
        Self {
            variant: SizeVariant::closest(w, h),
            width: w,
            height: h,
        }
    }

    /// Downscale factor of the actual viewport against the matched variant's
    /// canonical box: `min(width/cw, height/ch)`, clamped to `1.0`.
    ///
    /// The binding axis wins (`min`) so neither dimension overflows; the clamp
    /// keeps a larger-than-canonical viewport at authored sizes instead of
    /// inflating. This is the scale for *per-variant-layout* widgets (e.g. the
    /// digital and rectangular clock faces). A widget whose artwork is a single
    /// asset fit to one axis (e.g. the round clock dial) uses
    /// [`round_scale`](Self::round_scale) instead.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport and canonical dimensions are <= 1280, exact in f32"
    )]
    pub fn fit(self) -> f32 {
        let w_ratio = self.width as f32 / self.variant.width() as f32;
        let h_ratio = self.height as f32 / self.variant.height() as f32;
        w_ratio.min(h_ratio).min(1.0)
    }

    /// Asset-relative scale for the round face: `min(width, height) / 480`.
    /// Round widgets author their layout for the reference circle and multiply
    /// sizes by this, rather than [`fit`](Self::fit) (the rectangular downscale).
    #[must_use]
    pub fn round_scale(self) -> f32 {
        let diameter = u16::try_from(self.width.min(self.height)).unwrap_or(480);
        f32::from(diameter) / ROUND_FACE_PX
    }
}

/// Scale a native font size by `factor`, rounded to the nearest pixel with a
/// 1px minimum so a font never vanishes. `factor` is non-negative and normally
/// `<= 1.0` (e.g. the value from [`WidgetSize::fit`] or a widget's own
/// asset-relative scale).
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "sizes are small; factor is non-negative, so the result is small non-negative"
)]
pub fn scale_font(size: u32, factor: f32) -> u32 {
    let scaled = (size as f32 * factor).round() as u32;
    scaled.max(1)
}

/// Widget viewport dimensions, fetched from the host on demand.
///
/// The host caches these immutably at runtime construction, so this is a single
/// register read across the wasm boundary — call it as often as you need (in
/// `init`, in `render`, in helper functions) instead of stashing dimensions in
/// thread-locals.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn widget_size() -> WidgetSize {
    let packed = unsafe { ffi::host_widget_size() };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "low 32 bits intentionally selected via `as u32` after shift"
    )]
    let width = (packed >> 32) as u32;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "low 32 bits intentionally selected via `as u32`"
    )]
    let height = packed as u32;
    WidgetSize::from_dimensions(width, height)
}

/// Native-target stub. There is no host off-target, so there is no real
/// geometry to report; fabricating `0x0` would let callers mistake "no host"
/// for a genuine zero-sized widget. Off-target consumers that need geometry
/// must construct a [`WidgetSize`] themselves.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn widget_size() -> WidgetSize {
    panic!("BUG: widget_size() called off-target — no host geometry exists; supply it explicitly")
}

/// The widget's drawable rectangle, fetched from the host on demand.
/// Same single-register read as [`widget_size`]; call it anywhere.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn widget_viewport() -> WidgetViewport {
    let packed = unsafe { ffi::host_widget_size() };
    let shape_wire = unsafe { ffi::host_widget_viewport_shape() };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "high 32 bits selected via shift, low via truncation"
    )]
    let (width, height) = ((packed >> 32) as u32, packed as u32);
    WidgetViewport {
        width,
        height,
        shape: ViewportShape::try_from(shape_wire)
            .expect("BUG: host sent an invalid ViewportShape wire value"),
    }
}

/// Native stub. Like [`widget_size`], there is no host off-target to read a
/// viewport from; callers must supply a [`WidgetViewport`] themselves rather
/// than receive a fabricated rectangle.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn widget_viewport() -> WidgetViewport {
    panic!(
        "BUG: widget_viewport() called off-target — no host geometry exists; supply it explicitly"
    )
}

/// The logical display info, fetched from the host on demand.
/// Cached host-side and immutable for the runtime's life.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn display_info() -> DisplayInfo {
    let size = unsafe { ffi::host_display_size() };
    let shape_dpi = unsafe { ffi::host_display_shape_dpi() };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "high 32 bits selected via shift, low via truncation"
    )]
    let (width, height) = ((size >> 32) as u32, size as u32);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "high 32 bits selected via shift, low via truncation"
    )]
    let (shape_wire, dpi) = ((shape_dpi >> 32) as u32, shape_dpi as u32);
    DisplayInfo {
        width,
        height,
        shape: DisplayShape::try_from(shape_wire)
            .expect("BUG: host sent an invalid DisplayShape wire value"),
        dpi,
    }
}

/// Native stub. There is no host off-target, so there is no display to
/// describe; callers that need display geometry must supply a [`DisplayInfo`]
/// themselves rather than read a fabricated device size here.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn display_info() -> DisplayInfo {
    panic!("BUG: display_info() called off-target — no host geometry exists; supply it explicitly")
}

/// UTC instant. Wire format is the 8-byte LE `i64` of `unix_secs`.
#[derive(Debug, Clone, Copy)]
pub struct SystemTime {
    pub unix_secs: i64,
}

/// Decomposed wall-clock view of a `SystemTime` in a specific zone.
#[derive(Debug, Clone, Copy)]
pub struct LocalDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// 0 = Monday, 6 = Sunday.
    pub weekday: u8,
}

impl LocalDateTime {
    #[must_use]
    pub fn seconds_since_midnight(&self) -> u32 {
        u32::from(self.hour) * 3_600 + u32::from(self.minute) * 60 + u32::from(self.second)
    }
}

impl SystemTime {
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn now() -> Self {
        let mut buf = [0u8; 8];
        unsafe { ffi::host_get_system_time(buf.as_mut_ptr()) }
        Self {
            unix_secs: i64::from_le_bytes(buf),
        }
    }

    /// Project into local wall-clock fields for `tz`.
    /// Returns `None` when the host doesn't recognise the tz name.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn local(&self, tz: &crate::Tz) -> Option<LocalDateTime> {
        crate::calendar::tz_convert(self.unix_secs, tz.iana())
    }

    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn utc(&self) -> LocalDateTime {
        decompose(self.unix_secs)
    }
}

#[cfg(target_arch = "wasm32")]
fn decompose(unix_secs: i64) -> LocalDateTime {
    fn parse<T: core::str::FromStr + Default>(p: Option<&str>) -> T {
        p.and_then(|s| s.parse().ok()).unwrap_or_default()
    }
    let s = crate::format::strftime(unix_secs, "%Y %m %d %H %M %S %u");
    let mut parts = s.split_ascii_whitespace();
    let year: u16 = parse(parts.next());
    let month: u8 = parse(parts.next());
    let day: u8 = parse(parts.next());
    let hour: u8 = parse(parts.next());
    let minute: u8 = parse(parts.next());
    let second: u8 = parse(parts.next());
    // `%u` is ISO weekday: 1=Mon, 7=Sun. Re-map to 0=Mon, 6=Sun.
    let iso_weekday: u8 = parse(parts.next());
    let weekday = iso_weekday.saturating_sub(1);
    LocalDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        weekday,
    }
}

/// Touch interaction result (click or drag position + element dimensions).
#[derive(Debug, Clone, Copy)]
pub struct TouchHit {
    /// Local x position (relative to element left edge)
    pub x: f32,
    /// Local y position (relative to element top edge)
    pub y: f32,
    /// Element layout width
    pub width: f32,
    /// Element layout height
    pub height: f32,
}

impl TouchHit {
    #[cfg(target_arch = "wasm32")]
    fn from_buf(buf: &[u8; 16]) -> Self {
        Self {
            x: f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            y: f32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            width: f32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            height: f32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
        }
    }

    /// Horizontal fraction (0.0 = left edge, 1.0 = right edge), clamped.
    #[must_use]
    pub fn frac_x(&self) -> f32 {
        if self.width > 0.0 {
            (self.x / self.width).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod size_variant_tests {
    use super::{SizeVariant, WidgetSize};

    fn variant_of(w: u32, h: u32) -> SizeVariant {
        WidgetSize::from_dimensions(w, h).variant
    }

    #[test]
    fn exact_bmc100_dimensions_keep_their_variant() {
        assert_eq!(variant_of(1_280, 480), SizeVariant::Full);
        assert_eq!(variant_of(638, 480), SizeVariant::Large);
        assert_eq!(variant_of(638, 238), SizeVariant::Medium);
        assert_eq!(variant_of(317, 238), SizeVariant::Small);
    }

    #[test]
    fn non_deck_fullscreen_viewports_map_to_closest_legacy_variant() {
        assert_eq!(variant_of(320, 240), SizeVariant::Small);
        // BMM101 (480x320) lands on Large by a thin margin: normalized
        // distance Large 0.581 vs Medium 0.592. This assertion guards that
        // margin — a change to any canonical variant size that flips BMM101
        // from Large to Medium must update this test deliberately, not
        // silently. If the mapping target is wrong, fix the spec, not the test.
        assert_eq!(variant_of(480, 320), SizeVariant::Large);
        assert_eq!(variant_of(480, 480), SizeVariant::Large);
    }

    #[test]
    fn classification_is_deterministic() {
        // Same input, same output across repeated calls.
        assert_eq!(variant_of(200, 200), variant_of(200, 200));
    }
}

#[cfg(test)]
mod geometry_api_tests {
    use super::{display_info, widget_viewport};

    // Off-target there is no Wayland handshake, so the geometry getters have no
    // host to read from. They fail loud rather than fabricate a size a caller
    // could mistake for real geometry.

    #[test]
    #[should_panic(expected = "no host geometry exists")]
    fn native_widget_viewport_panics() {
        let _ = widget_viewport();
    }

    #[test]
    #[should_panic(expected = "no host geometry exists")]
    fn native_display_info_panics() {
        let _ = display_info();
    }
}

#[cfg(test)]
mod fit_and_scale_tests {
    use super::{WidgetSize, scale_font};

    fn fit_of(w: u32, h: u32) -> f32 {
        WidgetSize::from_dimensions(w, h).fit()
    }

    #[test]
    fn canonical_viewport_fits_at_one() {
        assert!((fit_of(1_280, 480) - 1.0).abs() < 1e-4);
        assert!((fit_of(638, 480) - 1.0).abs() < 1e-4);
        assert!((fit_of(317, 238) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn medium_canonical_fits_at_one() {
        // 638x238 is Medium's canonical box, so fit() = 1.0 — a Medium-authored
        // font renders at its authored size there, shrinking only below canonical.
        // (The round dial's geometry scales separately by its own dial ratio;
        // its per-variant text annotations use fit, like the rectangular face.)
        assert!((fit_of(638, 238) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn bmm101_large_downscales_by_binding_axis() {
        // 480x320 classifies to Large (638x480); min(480/638, 320/480) = 0.6667,
        // the height (binding) axis wins so neither dimension overflows.
        assert!((fit_of(480, 320) - 0.666_67).abs() < 1e-3);
    }

    #[test]
    fn larger_than_canonical_clamps_to_one() {
        // 1920x720 classifies to Full (1280x480); ratios 1.5/1.5 clamp to 1.0.
        assert!((fit_of(1_920, 720) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn scale_font_at_unit_factor_is_identity() {
        assert_eq!(scale_font(24, 1.0), 24);
    }

    #[test]
    fn scale_font_halves_and_rounds() {
        assert_eq!(scale_font(24, 0.5), 12);
        assert_eq!(scale_font(16, 0.5), 8);
    }

    #[test]
    fn scale_font_floors_at_one_px() {
        // Degenerate factors never yield a 0px (invisible) font.
        assert_eq!(scale_font(16, 0.01), 1);
    }

    #[test]
    fn rect_numeral_size_shrinks_on_bmm101() {
        // Pins the size the rect fix must produce: Large numerals (40px authored)
        // on BMM101's 480x320 scale to 27px.
        let fit = WidgetSize::from_dimensions(480, 320).fit();
        assert_eq!(scale_font(40, fit), 27);
    }
}
