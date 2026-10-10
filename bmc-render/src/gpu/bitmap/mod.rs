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

//! Bitmap registry — decodes raster images and uploads them as GPU textures.
//!
//! Tags reserve opaque `u16` bitmap IDs until eviction.
//! GPU textures exist only while their reservations are resident.

use std::collections::HashMap;
use std::fmt;
use std::io::Cursor;
use std::panic;

use bmc_ui::BitmapId;
use bmc_wasm_protocol::IdPool;
use femtovg::{ImageFlags, ImageId, ImageSource, Paint, Path};
use imgref::ImgRef;
use rgb::{FromSlice as _, RGBA8};

use crate::renderer::{AssetSuspendResult, AssetTagState};

const BITMAP_ID_EXCLUSIVE_CAP: u16 = u16::MAX;

/// GPU texture handle and source dimensions for a registered bitmap.
struct StoredBitmap {
    image_id: ImageId,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy)]
struct BitmapReservation {
    id: BitmapId,
    flags: ImageFlags,
}

/// Registry for tagged bitmap reservations and their resident GPU textures.
///
/// A tag reservation retains its ID and sampling flags across suspension.
pub struct BitmapRegistry {
    bitmaps: HashMap<BitmapId, StoredBitmap>,
    by_tag: HashMap<String, BitmapReservation>,
    ids: IdPool<BitmapId>,
}

impl fmt::Debug for BitmapRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitmapRegistry")
            .field("count", &self.bitmaps.len())
            .field("ids", &self.ids)
            .finish_non_exhaustive()
    }
}

impl BitmapRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bitmaps: HashMap::new(),
            by_tag: HashMap::new(),
            ids: IdPool::new(BITMAP_ID_EXCLUSIVE_CAP),
        }
    }

    pub fn reserve(&mut self, tag: &str, flags: ImageFlags) -> Option<BitmapId> {
        if let Some(reservation) = self.by_tag.get(tag) {
            if reservation.flags != flags {
                tracing::error!("bitmap reservation sampling mismatch ({tag})");
                return None;
            }
            return Some(reservation.id);
        }
        let Some(id) = self.ids.alloc() else {
            tracing::error!("bitmap registry exhausted ({tag})");
            return None;
        };
        self.by_tag
            .insert(tag.to_owned(), BitmapReservation { id, flags });
        Some(id)
    }

    /// Decode image bytes (PNG, JPEG, etc.) and upload a GPU texture under `tag`.
    ///
    /// A resident tag returns its ID without re-decoding.
    /// A suspended tag restores its payload using the stored image flags.
    ///
    /// Use `ImageFlags::empty()` for default bilinear filtering.
    /// Use `ImageFlags::NEAREST` for pixel-art or 9-patch assets.
    /// Bilinear filtering causes color bleeding across sub-rectangle boundaries.
    pub fn register(
        &mut self,
        tag: &str,
        data: &[u8],
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
        flags: ImageFlags,
    ) -> Option<BitmapId> {
        match self.tag_state(tag) {
            AssetTagState::Resident(id) => Some(id),
            AssetTagState::Suspended(id) => {
                let reservation = self
                    .by_tag
                    .get(tag)
                    .copied()
                    .expect("BUG: suspended bitmap reservation must exist");
                let (image_id, width, height) =
                    match decode_and_upload(data, canvas, reservation.flags) {
                        Ok(bitmap) => bitmap,
                        Err(e) => {
                            tracing::error!("failed to decode/upload bitmap ({tag}): {e}");
                            return None;
                        }
                    };
                self.bitmaps.insert(
                    id,
                    StoredBitmap {
                        image_id,
                        width,
                        height,
                    },
                );
                Some(id)
            }
            AssetTagState::Unknown => {
                let (image_id, width, height) = match decode_and_upload(data, canvas, flags) {
                    Ok(bitmap) => bitmap,
                    Err(e) => {
                        tracing::error!("failed to decode/upload bitmap ({tag}): {e}");
                        return None;
                    }
                };

                let Some(id) = self.reserve(tag, flags) else {
                    canvas.delete_image(image_id);
                    return None;
                };
                self.bitmaps.insert(
                    id,
                    StoredBitmap {
                        image_id,
                        width,
                        height,
                    },
                );
                Some(id)
            }
        }
    }

    /// Upload a pre-decoded RGBA buffer to GPU; no CPU copy.
    /// Replaces any existing bitmap registered under `tag`.
    pub fn register_rgba(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
        flags: ImageFlags,
    ) -> Option<BitmapId> {
        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        if rgba.len() != expected {
            tracing::error!(
                "register_rgba ({tag}): buffer len {} != {width}x{height}x4 ({expected})",
                rgba.len()
            );
            return None;
        }
        let pixels_rgba: &[RGBA8] = rgba.as_rgba();
        let src = ImageSource::Rgba(ImgRef::new(pixels_rgba, width as usize, height as usize));
        crate::gpu_access::assert_gpu_access_authorized();
        let image_id = match canvas.create_image(src, flags) {
            Ok(i) => i,
            Err(e) => {
                tracing::error!("failed to upload bitmap ({tag}): {e}");
                return None;
            }
        };
        if let Some(reservation) = self.by_tag.get_mut(tag) {
            let id = reservation.id;
            if let Some(old) = self.bitmaps.insert(
                id,
                StoredBitmap {
                    image_id,
                    width,
                    height,
                },
            ) {
                canvas.delete_image(old.image_id);
            }
            reservation.flags = flags;
            return Some(id);
        }
        let Some(id) = self.ids.alloc() else {
            canvas.delete_image(image_id);
            tracing::error!("bitmap registry exhausted ({tag})");
            return None;
        };
        self.bitmaps.insert(
            id,
            StoredBitmap {
                image_id,
                width,
                height,
            },
        );
        self.by_tag
            .insert(tag.to_owned(), BitmapReservation { id, flags });
        Some(id)
    }

    /// Return the reservation state for `tag`.
    #[must_use]
    pub fn tag_state(&self, tag: &str) -> AssetTagState<BitmapId> {
        let Some(reservation) = self.by_tag.get(tag) else {
            return AssetTagState::Unknown;
        };
        if self.bitmaps.contains_key(&reservation.id) {
            AssetTagState::Resident(reservation.id)
        } else {
            AssetTagState::Suspended(reservation.id)
        }
    }

    pub fn suspend_exact(
        &mut self,
        tag: &str,
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    ) -> AssetSuspendResult<BitmapId> {
        let Some(reservation) = self.by_tag.get(tag) else {
            return AssetSuspendResult::Unknown;
        };
        let id = reservation.id;
        if let Some(bitmap) = self.bitmaps.remove(&id) {
            crate::gpu_access::assert_gpu_access_authorized();
            canvas.delete_image(bitmap.image_id);
            AssetSuspendResult::Suspended(id)
        } else {
            AssetSuspendResult::AlreadySuspended(id)
        }
    }

    /// Get the FemtoVG `ImageId` for a registered bitmap.
    #[must_use]
    pub fn get(&self, id: BitmapId) -> Option<ImageId> {
        self.bitmaps.get(&id).map(|b| b.image_id)
    }

    /// Total resident texture bytes across registered bitmaps (`w·h·4` each).
    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        self.bitmaps
            .values()
            .map(|b| u64::from(b.width) * u64::from(b.height) * 4)
            .sum()
    }

    #[must_use]
    pub fn has_resident_prefix(&self, prefix: &str) -> bool {
        self.by_tag.iter().any(|(tag, reservation)| {
            bmc_wasm_protocol::tag_matches_prefix(tag, prefix)
                && self.bitmaps.contains_key(&reservation.id)
        })
    }

    /// Get the FemtoVG `ImageId` and source dimensions for a registered bitmap.
    #[must_use]
    pub fn get_with_size(&self, id: BitmapId) -> Option<(ImageId, u32, u32)> {
        self.bitmaps
            .get(&id)
            .map(|b| (b.image_id, b.width, b.height))
    }

    /// Delete all registered FemtoVG images.
    ///
    /// The caller must also invalidate renderer state that caches bitmap IDs.
    pub(super) fn clear(&mut self, canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>) {
        if !self.bitmaps.is_empty() {
            crate::gpu_access::assert_gpu_access_authorized();
        }
        for bitmap in self.bitmaps.drain().map(|(_, bitmap)| bitmap) {
            canvas.delete_image(bitmap.image_id);
        }
        self.clear_reservations();
    }

    fn clear_reservations(&mut self) {
        self.by_tag.clear();
        self.ids = IdPool::new(BITMAP_ID_EXCLUSIVE_CAP);
    }

    /// Evict a tag's bitmap reservation and any resident payload.
    /// Returns `true` when a tag was found and removed.
    fn evict(
        &mut self,
        tag: &str,
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    ) -> bool {
        let Some(reservation) = self.by_tag.remove(tag) else {
            return false;
        };
        if let Some(stored) = self.bitmaps.remove(&reservation.id) {
            crate::gpu_access::assert_gpu_access_authorized();
            canvas.delete_image(stored.image_id);
        }
        self.ids.release(reservation.id);
        true
    }

    /// Evict every tag matching `prefix` at segment boundaries (the tag is
    /// either exactly `prefix` or a descendant under it).
    /// The caller must invalidate renderer state that caches removed bitmap IDs.
    /// Returns the number of tags removed.
    pub(super) fn evict_prefix(
        &mut self,
        prefix: &str,
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    ) -> usize {
        let tags: Vec<String> = self
            .by_tag
            .keys()
            .filter(|k| bmc_wasm_protocol::tag_matches_prefix(k, prefix))
            .cloned()
            .collect();
        let mut n = 0;
        for tag in tags {
            if self.evict(&tag, canvas) {
                n += 1;
            }
        }
        n
    }
}

/// Decode image bytes to RGBA and upload them to the GPU.
fn decode_and_upload(
    data: &[u8],
    canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    flags: ImageFlags,
) -> anyhow::Result<(ImageId, u32, u32)> {
    let rgba = decode_full_to_dynamic(data)?.into_rgba8();

    let (w, h) = (rgba.width(), rgba.height());
    let pixels_rgba: &[RGBA8] = rgba.as_raw().as_rgba();

    let src = ImageSource::Rgba(ImgRef::new(pixels_rgba, w as usize, h as usize));
    crate::gpu_access::assert_gpu_access_authorized();
    let image_id = canvas.create_image(src, flags)?;
    Ok((image_id, w, h))
}

/// Decode PNG/JPEG/etc. bytes into tightly packed RGBA pixels without GPU access.
pub fn decode_bitmap_rgba(data: &[u8]) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let rgba = decode_full_to_dynamic(data)?.into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok((rgba.into_raw(), width, height))
}

/// Decode to RGBA scaled to fit within `max_w`×`max_h` (no upscale), letterboxed
/// at render. JPEG scales on load, others full-decode.
pub fn decode_scaled_to_fit(
    data: &[u8],
    max_w: u32,
    max_h: u32,
) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    Ok(resize_rgba_to_fit(
        decode_to_dynamic(data, max_w, max_h)?,
        max_w,
        max_h,
    ))
}

/// Decode to RGBA scaled to cover `w`×`h` and centre-cropped to exactly that,
/// filled (no letterbox) at render. JPEG scales on load, others full-decode.
pub fn decode_scaled_to_cover(data: &[u8], w: u32, h: u32) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    Ok(resize_rgba_to_cover(&decode_to_dynamic(data, w, h)?, w, h))
}

/// Decode to a `DynamicImage` at least `max_w`×`max_h` where the source allows:
/// JPEG DCT-scales on load near the target, others full-decode (alloc-capped).
fn decode_to_dynamic(data: &[u8], max_w: u32, max_h: u32) -> anyhow::Result<image::DynamicImage> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        decode_jpeg_to_dynamic(data, max_w, max_h)
    } else {
        decode_full_to_dynamic(data)
    }
}

/// DCT scale-on-load near the target; returns the decoded image (un-resampled).
fn decode_jpeg_to_dynamic(
    data: &[u8],
    max_w: u32,
    max_h: u32,
) -> anyhow::Result<image::DynamicImage> {
    panic::catch_unwind(|| {
        let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(&data));
        decoder.scale(
            u16::try_from(max_w).unwrap_or(u16::MAX),
            u16::try_from(max_h).unwrap_or(u16::MAX),
        )?;
        let info = decoder
            .info()
            .ok_or_else(|| anyhow::anyhow!("jpeg has no frame info"))?;
        let pixels = decoder.decode()?;
        let (sw, sh) = (u32::from(info.width), u32::from(info.height));
        let rgba = jpeg_to_rgba(&pixels, info.pixel_format)?;
        let img = image::RgbaImage::from_raw(sw, sh, rgba)
            .ok_or_else(|| anyhow::anyhow!("jpeg pixel buffer size mismatch"))?;
        Ok(image::DynamicImage::ImageRgba8(img))
    })
    .map_err(|_| anyhow::anyhow!("jpeg decoder panicked"))?
}

/// Full decode (pixel- and allocation-capped); returns the decoded image (un-resampled).
///
/// Unlike the JPEG path (`decode_jpeg_to_dynamic` DCT-shrinks on load), this
/// decodes the whole source into memory before the caller downscales — peak
/// memory is the full source (within budget), not the target. A streaming
/// row-wise PNG decode was deferred: it only helps non-interlaced PNGs and is
/// messy for that narrow gain; large sources lean on server-side `{{width}}`.
///
/// The decode runs under `catch_unwind` — third-party decoders
/// (zune-jpeg's AVX2/NEON paths) can panic on certain image dimensions.
fn decode_full_to_dynamic(data: &[u8]) -> anyhow::Result<image::DynamicImage> {
    panic::catch_unwind(|| {
        // Reject oversized sources before the full decode allocates (pixel budget).
        let (w, h) = image::ImageReader::new(Cursor::new(&data))
            .with_guessed_format()
            .map_err(image::ImageError::IoError)?
            .into_dimensions()?;
        anyhow::ensure!(
            u64::from(w) * u64::from(h) <= crate::MAX_DECODE_IMAGE_PIXELS,
            "image exceeds pixel budget ({w}x{h})"
        );
        let mut reader = image::ImageReader::new(Cursor::new(&data));
        let mut limits = image::io::Limits::default();
        limits.max_alloc = Some(crate::MAX_DECODE_IMAGE_ALLOC_BYTES);
        reader.limits(limits);
        reader
            .with_guessed_format()
            .map_err(image::ImageError::IoError)?
            .decode()
            .map_err(anyhow::Error::from)
    })
    .map_err(|_| anyhow::anyhow!("image decoder panicked"))?
}

/// Resample down to fit `max_w`×`max_h` preserving aspect; never upscales.
fn resize_rgba_to_fit(img: image::DynamicImage, max_w: u32, max_h: u32) -> (Vec<u8>, u32, u32) {
    let rgba = if img.width() > max_w || img.height() > max_h {
        img.resize(max_w, max_h, image::imageops::FilterType::Triangle)
            .into_rgba8()
    } else {
        img.into_rgba8()
    };
    let (w, h) = (rgba.width(), rgba.height());
    (rgba.into_raw(), w, h)
}

/// Centre-crop the source to the `w:h` aspect, then resample to exactly `w`×`h`.
/// Crop-first bounds the transient (no oversized resize-to-cover intermediate).
#[expect(
    clippy::integer_division,
    reason = "pixel crop dims and centre offsets — integer truncation is intended"
)]
fn resize_rgba_to_cover(img: &image::DynamicImage, w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (sw, sh) = (img.width(), img.height());
    let (cw, ch) = cover_crop(sw, sh, w, h);
    let rgba = img
        .crop_imm((sw - cw) / 2, (sh - ch) / 2, cw, ch)
        .resize_exact(w, h, image::imageops::FilterType::Triangle)
        .into_rgba8();
    (rgba.into_raw(), w, h)
}

/// Largest centred `w:h`-aspect rect fitting in `sw`×`sh`.
#[expect(
    clippy::integer_division,
    reason = "pixel crop dimensions — integer truncation is intended"
)]
fn cover_crop(sw: u32, sh: u32, w: u32, h: u32) -> (u32, u32) {
    let (sw64, sh64, w64, h64) = (u64::from(sw), u64::from(sh), u64::from(w), u64::from(h));
    if sw64 * h64 >= sh64 * w64 {
        (u32::try_from(sh64 * w64 / h64).unwrap_or(sw).min(sw), sh)
    } else {
        (sw, u32::try_from(sw64 * h64 / w64).unwrap_or(sh).min(sh))
    }
}

/// Expand jpeg-decoder output to RGBA; only RGB and 8-bit grayscale.
fn jpeg_to_rgba(pixels: &[u8], format: jpeg_decoder::PixelFormat) -> anyhow::Result<Vec<u8>> {
    use jpeg_decoder::PixelFormat;
    match format {
        PixelFormat::RGB24 => Ok(pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect()),
        PixelFormat::L8 => Ok(pixels.iter().flat_map(|&g| [g, g, g, 255]).collect()),
        fmt @ (PixelFormat::L16 | PixelFormat::CMYK32) => {
            anyhow::bail!("unsupported jpeg pixel format: {fmt:?}")
        }
    }
}

/// Draw a sub-rectangle of a bitmap: sample from `(sx, sy, sw, sh)`
/// in the source and render it into `(dx, dy, dw, dh)` on the canvas,
/// each texel multiplied by `tint`.
#[expect(clippy::too_many_arguments)]
pub fn draw_bitmap_subrect(
    canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    image_id: ImageId,
    src_w: f32,
    src_h: f32,
    sx: f32,
    sy: f32,
    sw: f32,
    sh: f32,
    dx: f32,
    dy: f32,
    dw: f32,
    dh: f32,
    tint: femtovg::Color,
) {
    if dw <= 0.0 || dh <= 0.0 || sw <= 0.0 || sh <= 0.0 {
        return;
    }
    // Paint::image maps the full image to (ox, oy, ow, oh).
    // To show only the sub-rect (sx, sy, sw, sh) at (dx, dy, dw, dh),
    // compute the virtual full-image placement so the sub-rect aligns.
    let scale_x = dw / sw;
    let scale_y = dh / sh;
    let ox = dx - sx * scale_x;
    let oy = dy - sy * scale_y;
    let ow = src_w * scale_x;
    let oh = src_h * scale_y;

    let paint = Paint::image_tint(image_id, ox, oy, ow, oh, 0.0, tint).with_anti_alias(false);
    let mut path = Path::new();
    path.rect(dx, dy, dw, dh);
    canvas.fill_path(&path, &paint);
}

/// (src_x, src_y, src_w, src_h, dst_x, dst_y, dst_w, dst_h)
type Quad = (f32, f32, f32, f32, f32, f32, f32, f32);

/// Draw a 9-patch bitmap: slice the source into 9 quads using insets and stretch appropriately.
///
/// Corners stay at fixed size, edges stretch in one axis, center stretches both.
#[expect(clippy::too_many_arguments)]
pub fn draw_nine_patch(
    canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    image_id: ImageId,
    src_w: f32,
    src_h: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    tint: femtovg::Color,
) {
    // Source regions
    let src_center_w = src_w - left - right;
    let src_center_h = src_h - top - bottom;

    // Destination regions — corners stay at fixed pixel size, only center stretches
    let dl = left.min(w);
    let dr = right.min(w - dl);
    let dt = top.min(h);
    let db = bottom.min(h - dt);
    let dst_center_w = (w - dl - dr).max(0.0);
    let dst_center_h = (h - dt - db).max(0.0);
    let quads: [Quad; 9] = [
        // Top-left corner
        (0.0, 0.0, left, top, x, y, dl, dt),
        // Top edge
        (left, 0.0, src_center_w, top, x + dl, y, dst_center_w, dt),
        // Top-right corner
        (
            src_w - right,
            0.0,
            right,
            top,
            x + dl + dst_center_w,
            y,
            dr,
            dt,
        ),
        // Left edge
        (0.0, top, left, src_center_h, x, y + dt, dl, dst_center_h),
        // Center
        (
            left,
            top,
            src_center_w,
            src_center_h,
            x + dl,
            y + dt,
            dst_center_w,
            dst_center_h,
        ),
        // Right edge
        (
            src_w - right,
            top,
            right,
            src_center_h,
            x + dl + dst_center_w,
            y + dt,
            dr,
            dst_center_h,
        ),
        // Bottom-left corner
        (
            0.0,
            src_h - bottom,
            left,
            bottom,
            x,
            y + dt + dst_center_h,
            dl,
            db,
        ),
        // Bottom edge
        (
            left,
            src_h - bottom,
            src_center_w,
            bottom,
            x + dl,
            y + dt + dst_center_h,
            dst_center_w,
            db,
        ),
        // Bottom-right corner
        (
            src_w - right,
            src_h - bottom,
            right,
            bottom,
            x + dl + dst_center_w,
            y + dt + dst_center_h,
            dr,
            db,
        ),
    ];

    for &(sx, sy, sw, sh, dx, dy, dw, dh) in &quads {
        draw_bitmap_subrect(
            canvas, image_id, src_w, src_h, sx, sy, sw, sh, dx, dy, dw, dh, tint,
        );
    }
}

/// Render a registered bitmap onto the canvas at the given rect,
/// each texel multiplied by `tint`.
pub fn draw_bitmap(
    canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    image_id: ImageId,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    tint: femtovg::Color,
) {
    let paint = Paint::image_tint(image_id, x, y, w, h, 0.0, tint).with_anti_alias(false);
    let mut path = Path::new();
    path.rect(x, y, w, h);
    canvas.fill_path(&path, &paint);
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests;
