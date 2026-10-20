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

#[cfg(target_arch = "wasm32")]
use bmc_ui::{AudioId, BitmapId, MeshId, PackageAssetRef, SvgId};
#[cfg(target_arch = "wasm32")]
use bmc_wasm_protocol::ImageJobId;

#[cfg(target_arch = "wasm32")]
use crate::net::FetchBodyRef;

/// Register icon data with the host under `tag`. Idempotent host-side.
/// Wire `0` lifts to `None`.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_svg(tag: &str, data: &[u8]) -> Option<SvgId> {
    Platform::register_svg(tag, data)
}

#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_svg_package(tag: &str, package_ref: &PackageAssetRef) -> Option<SvgId> {
    Platform::register_svg_package(tag, package_ref)
}

/// Register mesh data (optimized binary format) with the host under `tag`.
/// Idempotent host-side; first call uploads VBO/IBO/texture to GPU.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_mesh(tag: &str, data: &[u8]) -> Option<MeshId> {
    Platform::register_mesh(tag, data)
}

#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_mesh_package(tag: &str, package_ref: &PackageAssetRef) -> Option<MeshId> {
    Platform::register_mesh_package(tag, package_ref)
}

/// Register audio data (WAV/OGG/MP3 bytes) with the host.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_audio(data: &[u8], name: &str) -> Option<AudioId> {
    Platform::register_audio(data, name)
}

#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_audio_package(name: &str, package_ref: &PackageAssetRef) -> Option<AudioId> {
    Platform::register_audio_package(name, package_ref)
}

/// Register bitmap data (PNG bytes) with the host under `tag`. Idempotent
/// host-side.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_bitmap(tag: &str, data: &[u8]) -> Option<BitmapId> {
    Platform::register_bitmap(tag, data)
}

#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_bitmap_package(tag: &str, package_ref: &PackageAssetRef) -> Option<BitmapId> {
    Platform::register_bitmap_package(tag, package_ref)
}

/// Register bitmap data with nearest-neighbor filtering (no bilinear
/// interpolation) under `tag`. Idempotent host-side.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_bitmap_nearest(tag: &str, data: &[u8]) -> Option<BitmapId> {
    Platform::register_bitmap_nearest(tag, data)
}

#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_bitmap_nearest_package(
    tag: &str,
    package_ref: &PackageAssetRef,
) -> Option<BitmapId> {
    Platform::register_bitmap_nearest_package(tag, package_ref)
}

/// Decode + downscale `data` to fit `max_w`×`max_h` off the render thread.
/// The bitmap is delivered later via `__on_image_ready`; returns the job
/// handle (`None` = rejected).
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_bitmap_fit(
    tag: &str,
    data: &[u8],
    max_w: u32,
    max_h: u32,
    cover: bool,
    identity: &[u8],
) -> Option<ImageJobId> {
    Platform::register_bitmap_fit(tag, data, max_w, max_h, cover, identity)
}

/// Start a fitted bitmap decode from a callback-scoped host fetch body.
/// Rejection returns the body reference for another operation.
#[cfg(target_arch = "wasm32")]
pub fn register_bitmap_fit_ref<'a>(
    tag: &str,
    body: FetchBodyRef<'a>,
    max_w: u32,
    max_h: u32,
    cover: bool,
    identity: &[u8],
) -> Result<ImageJobId, FetchBodyRef<'a>> {
    Platform::register_bitmap_fit_ref(tag, body, max_w, max_h, cover, identity)
}

/// Restore a bitmap from its per-instance cache entry
/// (host-side mmap → texture; no bytes cross into wasm).
/// `None` on a miss.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn register_bitmap_from_cache(tag: &str) -> Option<BitmapId> {
    Platform::register_bitmap_from_cache(tag)
}

/// Drop every host-side asset (icon, bitmap, mesh, audio) whose tag
/// starts with `prefix`. The host implicitly namespaces by guest ID,
/// so the prefix only sees this widget's own registrations.
/// IDs for evicted assets become invalid and must be discarded.
/// Returns the number of entries evicted across all four registries.
#[expect(
    clippy::must_use_candidate,
    reason = "evicting is the effect; callers may ignore the count"
)]
#[cfg(target_arch = "wasm32")]
pub fn evict_prefix(prefix: &str) -> u32 {
    Platform::evict_prefix(prefix)
}

/// Evict everything this widget registered — its whole namespace.
/// All previously returned asset IDs become invalid and must be discarded.
/// Returns the number of entries evicted.
#[expect(
    clippy::must_use_candidate,
    reason = "evicting is the effect; callers may ignore the count"
)]
#[cfg(target_arch = "wasm32")]
pub fn evict_all() -> u32 {
    Platform::evict_all()
}

/// Get the dimensions of an image (PNG, JPEG, etc.) without decoding the full pixel data.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    Platform::image_dimensions(data)
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
#[cfg(target_arch = "wasm32")]
pub fn image_dimensions_ref(body: &FetchBodyRef<'_>) -> Option<RetainedImageDimensions> {
    Platform::image_dimensions_ref(body)
}

/// Decode image data (PNG, JPEG, etc.) to RGBA pixels on the host.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn decode_image(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    Platform::decode_image(data)
}

/// Maximum image size, in pixels, the host will decode.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn max_image_pixels() -> u32 {
    Platform::max_image_pixels()
}

#[cfg(target_arch = "wasm32")]
trait AssetRegistry {
    fn register_svg(tag: &str, data: &[u8]) -> Option<SvgId>;
    fn register_svg_package(tag: &str, package_ref: &PackageAssetRef) -> Option<SvgId>;
    fn register_mesh(tag: &str, data: &[u8]) -> Option<MeshId>;
    fn register_mesh_package(tag: &str, package_ref: &PackageAssetRef) -> Option<MeshId>;
    fn register_audio(data: &[u8], name: &str) -> Option<AudioId>;
    fn register_audio_package(name: &str, package_ref: &PackageAssetRef) -> Option<AudioId>;
    fn register_bitmap(tag: &str, data: &[u8]) -> Option<BitmapId>;
    fn register_bitmap_package(tag: &str, package_ref: &PackageAssetRef) -> Option<BitmapId>;
    fn register_bitmap_nearest(tag: &str, data: &[u8]) -> Option<BitmapId>;
    fn register_bitmap_nearest_package(
        tag: &str,
        package_ref: &PackageAssetRef,
    ) -> Option<BitmapId>;
    fn register_bitmap_fit(
        tag: &str,
        data: &[u8],
        max_w: u32,
        max_h: u32,
        cover: bool,
        identity: &[u8],
    ) -> Option<ImageJobId>;
    fn register_bitmap_fit_ref<'a>(
        tag: &str,
        body: FetchBodyRef<'a>,
        max_w: u32,
        max_h: u32,
        cover: bool,
        identity: &[u8],
    ) -> Result<ImageJobId, FetchBodyRef<'a>>;
    fn register_bitmap_from_cache(tag: &str) -> Option<BitmapId>;
    fn evict_prefix(prefix: &str) -> u32;
    fn evict_all() -> u32;
}

#[cfg(target_arch = "wasm32")]
trait ImageDecodeProvider {
    fn image_dimensions(data: &[u8]) -> Option<(u32, u32)>;
    fn image_dimensions_ref(body: &FetchBodyRef<'_>) -> Option<RetainedImageDimensions>;
    fn decode_image(data: &[u8]) -> Option<(Vec<u8>, u32, u32)>;
    fn max_image_pixels() -> u32;
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use bmc_ui::{AudioId, BitmapId, MeshId, PackageAssetRef, SvgId};
    use bmc_wasm_protocol::ImageJobId;

    use super::RetainedImageDimensions;
    use crate::net::FetchBodyRef;

    // Names the module the host registers these under. Without it rustc 1.96+
    // leaves the block as undefined symbols instead of imports, and lld fails.
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        // Svg registration. The host dedups by tag.
        fn host_register_svg(
            tag_ptr: *const u8,
            tag_len: u32,
            data_ptr: *const u8,
            data_len: u32,
        ) -> u32;
        fn host_register_svg_package(tag_ptr: *const u8, tag_len: u32, id_ptr: *const u8) -> u32;
        // Bitmap registration. The host dedups by tag.
        fn host_register_bitmap(
            tag_ptr: *const u8,
            tag_len: u32,
            data_ptr: *const u8,
            data_len: u32,
        ) -> u32;
        fn host_register_bitmap_nearest(
            tag_ptr: *const u8,
            tag_len: u32,
            data_ptr: *const u8,
            data_len: u32,
        ) -> u32;
        fn host_register_bitmap_package(tag_ptr: *const u8, tag_len: u32, id_ptr: *const u8)
        -> u32;
        fn host_register_bitmap_nearest_package(
            tag_ptr: *const u8,
            tag_len: u32,
            id_ptr: *const u8,
        ) -> u32;
        fn host_register_bitmap_fit(
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
        fn host_register_bitmap_fit_ref(
            tag_ptr: *const u8,
            tag_len: u32,
            request_id: u32,
            max_w: u32,
            max_h: u32,
            cover: u32,
            identity_ptr: *const u8,
            identity_len: u32,
        ) -> u32;
        fn host_register_bitmap_from_cache(tag_ptr: *const u8, tag_len: u32) -> u32;

        // Mesh registration. The host dedups by tag.
        fn host_register_mesh(
            tag_ptr: *const u8,
            tag_len: u32,
            data_ptr: *const u8,
            data_len: u32,
        ) -> u32;
        fn host_register_mesh_package(tag_ptr: *const u8, tag_len: u32, id_ptr: *const u8) -> u32;
        // Audio registration
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

    pub(super) struct Host;

    impl super::AssetRegistry for Host {
        fn register_svg(tag: &str, data: &[u8]) -> Option<SvgId> {
            SvgId::from_ffi(unsafe {
                host_register_svg(
                    tag.as_ptr(),
                    crate::ffi_u32(tag.len()),
                    data.as_ptr(),
                    crate::ffi_u32(data.len()),
                )
            })
        }

        fn register_svg_package(tag: &str, package_ref: &PackageAssetRef) -> Option<SvgId> {
            SvgId::from_ffi(unsafe {
                host_register_svg_package(
                    tag.as_ptr(),
                    crate::ffi_u32(tag.len()),
                    package_ref.as_bytes().as_ptr(),
                )
            })
        }

        fn register_mesh(tag: &str, data: &[u8]) -> Option<MeshId> {
            MeshId::from_ffi(unsafe {
                host_register_mesh(
                    tag.as_ptr(),
                    crate::ffi_u32(tag.len()),
                    data.as_ptr(),
                    crate::ffi_u32(data.len()),
                )
            })
        }

        fn register_mesh_package(tag: &str, package_ref: &PackageAssetRef) -> Option<MeshId> {
            MeshId::from_ffi(unsafe {
                host_register_mesh_package(
                    tag.as_ptr(),
                    crate::ffi_u32(tag.len()),
                    package_ref.as_bytes().as_ptr(),
                )
            })
        }

        fn register_audio(data: &[u8], name: &str) -> Option<AudioId> {
            AudioId::from_ffi(unsafe {
                host_register_audio(
                    data.as_ptr(),
                    crate::ffi_u32(data.len()),
                    name.as_ptr(),
                    crate::ffi_u32(name.len()),
                )
            })
        }

        fn register_audio_package(name: &str, package_ref: &PackageAssetRef) -> Option<AudioId> {
            AudioId::from_ffi(unsafe {
                host_register_audio_package(
                    name.as_ptr(),
                    crate::ffi_u32(name.len()),
                    package_ref.as_bytes().as_ptr(),
                )
            })
        }

        fn register_bitmap(tag: &str, data: &[u8]) -> Option<BitmapId> {
            BitmapId::from_ffi(unsafe {
                host_register_bitmap(
                    tag.as_ptr(),
                    crate::ffi_u32(tag.len()),
                    data.as_ptr(),
                    crate::ffi_u32(data.len()),
                )
            })
        }

        fn register_bitmap_package(tag: &str, package_ref: &PackageAssetRef) -> Option<BitmapId> {
            BitmapId::from_ffi(unsafe {
                host_register_bitmap_package(
                    tag.as_ptr(),
                    crate::ffi_u32(tag.len()),
                    package_ref.as_bytes().as_ptr(),
                )
            })
        }

        fn register_bitmap_nearest(tag: &str, data: &[u8]) -> Option<BitmapId> {
            BitmapId::from_ffi(unsafe {
                host_register_bitmap_nearest(
                    tag.as_ptr(),
                    crate::ffi_u32(tag.len()),
                    data.as_ptr(),
                    crate::ffi_u32(data.len()),
                )
            })
        }

        fn register_bitmap_nearest_package(
            tag: &str,
            package_ref: &PackageAssetRef,
        ) -> Option<BitmapId> {
            BitmapId::from_ffi(unsafe {
                host_register_bitmap_nearest_package(
                    tag.as_ptr(),
                    crate::ffi_u32(tag.len()),
                    package_ref.as_bytes().as_ptr(),
                )
            })
        }

        fn register_bitmap_fit(
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
                    crate::ffi_u32(tag.len()),
                    data.as_ptr(),
                    crate::ffi_u32(data.len()),
                    max_w,
                    max_h,
                    u32::from(cover),
                    identity.as_ptr(),
                    crate::ffi_u32(identity.len()),
                )
            })
        }

        fn register_bitmap_from_cache(tag: &str) -> Option<BitmapId> {
            BitmapId::from_ffi(unsafe {
                host_register_bitmap_from_cache(tag.as_ptr(), crate::ffi_u32(tag.len()))
            })
        }

        fn register_bitmap_fit_ref<'a>(
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
                    crate::ffi_u32(tag.len()),
                    body.request_id_wire(),
                    max_w,
                    max_h,
                    u32::from(cover),
                    identity.as_ptr(),
                    crate::ffi_u32(identity.len()),
                )
            });
            job_id.ok_or(body)
        }

        fn evict_prefix(prefix: &str) -> u32 {
            unsafe { host_evict_prefix(prefix.as_ptr(), crate::ffi_u32(prefix.len())) }
        }

        fn evict_all() -> u32 {
            unsafe { host_evict_all() }
        }
    }

    /// The `width << 32 | height` a host packs image dimensions as;
    /// `None` when the host refused, or either side is zero.
    fn unpack_dimensions(packed: i64) -> Option<(u32, u32)> {
        if packed < 0 {
            return None;
        }
        let [h0, h1, h2, h3, w0, w1, w2, w3] = packed.to_le_bytes();
        let (width, height) = (
            u32::from_le_bytes([w0, w1, w2, w3]),
            u32::from_le_bytes([h0, h1, h2, h3]),
        );
        (width != 0 && height != 0).then_some((width, height))
    }

    impl super::ImageDecodeProvider for Host {
        fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
            let packed = unsafe {
                host_decode_image(
                    data.as_ptr(),
                    crate::ffi_u32(data.len()),
                    core::ptr::null_mut(),
                    0,
                )
            };
            unpack_dimensions(packed)
        }

        fn image_dimensions_ref(body: &FetchBodyRef<'_>) -> Option<RetainedImageDimensions> {
            let mut max_source_pixels = 0;
            let packed = unsafe {
                host_image_dimensions_ref(body.request_id_wire(), &raw mut max_source_pixels)
            };
            let (width, height) = unpack_dimensions(packed)?;
            Some(RetainedImageDimensions {
                width,
                height,
                max_source_pixels,
            })
        }

        fn decode_image(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
            let (w, h) = Self::image_dimensions(data)?;
            let needed = w * h * 4;

            let mut buf = vec![0_u8; needed as usize];
            let packed2 = unsafe {
                host_decode_image(
                    data.as_ptr(),
                    crate::ffi_u32(data.len()),
                    buf.as_mut_ptr(),
                    needed,
                )
            };
            if packed2 < 0 {
                return None;
            }
            Some((buf, w, h))
        }

        fn max_image_pixels() -> u32 {
            unsafe { host_max_image_pixels() }
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;
