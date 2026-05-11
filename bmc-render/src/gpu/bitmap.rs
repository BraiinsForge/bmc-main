// Copyright (C) 2026  Braiins Systems s.r.o.

//! Bitmap registry — decodes raster images and uploads them as GPU textures.
//!
//! Bitmaps are registered once (on first use from WASM) and persist for the
//! runtime lifetime. Each registered bitmap gets an opaque `u16` ID that maps
//! to a FemtoVG `ImageId` (GPU texture handle).

use std::collections::HashMap;
use std::fmt;
use std::io::Cursor;
use std::panic;

use bmc_wasm_protocol::BitmapId;
use femtovg::{ImageFlags, ImageId, ImageSource, Paint, Path};
use imgref::ImgRef;
use rgb::{FromSlice as _, RGBA8};

/// Decoded bitmap pixels retained for host-side sampling.
struct StoredBitmap {
    image_id: ImageId,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

/// Registry mapping opaque widget-side IDs to FemtoVG GPU texture handles
/// and retained RGBA pixel data for host-side sampling.
///
/// Registrations are deduped by tag: re-registering the same tag returns the
/// cached ID without re-decoding or re-uploading.
pub struct BitmapRegistry {
    bitmaps: HashMap<BitmapId, StoredBitmap>,
    by_tag: HashMap<String, BitmapId>,
    next_id: u16,
}

impl fmt::Debug for BitmapRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitmapRegistry")
            .field("count", &self.bitmaps.len())
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl BitmapRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bitmaps: HashMap::new(),
            by_tag: HashMap::new(),
            next_id: 1,
        }
    }

    /// Decode image bytes (PNG, JPEG, etc.) and upload to GPU as a texture
    /// under `tag`. Idempotent: a second call with the same tag returns the
    /// cached ID without re-decoding. The `flags` from the first successful
    /// call win.
    ///
    /// Pass `ImageFlags::empty()` for default bilinear filtering, or
    /// `ImageFlags::NEAREST` for pixel-art / 9-patch assets where bilinear
    /// filtering would cause color bleeding across sub-rect boundaries.
    pub fn register(
        &mut self,
        tag: &str,
        data: &[u8],
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
        flags: ImageFlags,
    ) -> Option<BitmapId> {
        if let Some(&id) = self.by_tag.get(tag) {
            return Some(id);
        }
        let (image_id, pixels, width, height) = match decode_and_upload(data, canvas, flags) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("failed to decode/upload bitmap ({tag}): {e}");
                return None;
            }
        };

        let id = BitmapId::alloc(&mut self.next_id);
        self.bitmaps.insert(
            id,
            StoredBitmap {
                image_id,
                pixels,
                width,
                height,
            },
        );
        self.by_tag.insert(tag.to_owned(), id);
        Some(id)
    }

    /// Get the FemtoVG `ImageId` for a registered bitmap.
    #[must_use]
    pub fn get(&self, id: BitmapId) -> Option<ImageId> {
        self.bitmaps.get(&id).map(|b| b.image_id)
    }

    /// Get the FemtoVG `ImageId` and source dimensions for a registered bitmap.
    #[must_use]
    pub fn get_with_size(&self, id: BitmapId) -> Option<(ImageId, u32, u32)> {
        self.bitmaps
            .get(&id)
            .map(|b| (b.image_id, b.width, b.height))
    }

    /// Delete all registered FemtoVG images.
    pub fn clear(&mut self, canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>) {
        for bitmap in self.bitmaps.drain().map(|(_, bitmap)| bitmap) {
            canvas.delete_image(bitmap.image_id);
        }
        self.by_tag.clear();
    }

    /// Evict a single tag's bitmap: drop the FemtoVG image and remove the
    /// entry. Returns `true` if a tag was found and removed.
    ///
    /// IDs are not recycled — registering a fresh tag after eviction
    /// allocates a new ID via `next_id`.
    pub fn evict(
        &mut self,
        tag: &str,
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    ) -> bool {
        let Some(id) = self.by_tag.remove(tag) else {
            return false;
        };
        let Some(stored) = self.bitmaps.remove(&id) else {
            return false;
        };
        canvas.delete_image(stored.image_id);
        true
    }

    /// Evict every tag whose key starts with `prefix`. Returns the number of
    /// tags removed.
    pub fn evict_prefix(
        &mut self,
        prefix: &str,
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    ) -> usize {
        let tags: Vec<String> = self
            .by_tag
            .keys()
            .filter(|k| k.starts_with(prefix))
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

    /// Sample the average RGBA color of a rectangular region within a registered bitmap.
    ///
    /// Coordinates are clamped to the bitmap dimensions. Returns `None` if the bitmap ID
    /// is not registered or the region is empty after clamping.
    #[must_use]
    #[expect(clippy::many_single_char_names)]
    pub fn sample(&self, id: BitmapId, x: u32, y: u32, w: u32, h: u32) -> Option<u32> {
        let bmp = self.bitmaps.get(&id)?;

        let x0 = x.min(bmp.width);
        let y0 = y.min(bmp.height);
        let x1 = x.saturating_add(w).min(bmp.width);
        let y1 = y.saturating_add(h).min(bmp.height);

        let region_w = x1 - x0;
        let region_h = y1 - y0;
        if region_w == 0 || region_h == 0 {
            return None;
        }

        let (mut r_sum, mut g_sum, mut b_sum, mut a_sum) = (0_u64, 0_u64, 0_u64, 0_u64);
        for row in y0..y1 {
            let row_start = (row * bmp.width + x0) as usize * 4;
            for col in 0..region_w {
                let idx = row_start + col as usize * 4;
                r_sum += u64::from(bmp.pixels[idx]);
                g_sum += u64::from(bmp.pixels[idx + 1]);
                b_sum += u64::from(bmp.pixels[idx + 2]);
                a_sum += u64::from(bmp.pixels[idx + 3]);
            }
        }

        let count = u64::from(region_w) * u64::from(region_h);
        #[expect(clippy::cast_possible_truncation, clippy::integer_division)]
        let (r, g, b, a) = (
            (r_sum / count) as u32,
            (g_sum / count) as u32,
            (b_sum / count) as u32,
            (a_sum / count) as u32,
        );
        Some((r << 24) | (g << 16) | (b << 8) | a)
    }
}

/// Decode image bytes to RGBA, upload to the GPU, and return the pixel data.
///
/// The decode step is wrapped in `catch_unwind` because third-party JPEG decoders
/// (zune-jpeg AVX2/NEON paths) can panic on certain image dimensions.
fn decode_and_upload(
    data: &[u8],
    canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    flags: ImageFlags,
) -> anyhow::Result<(ImageId, Vec<u8>, u32, u32)> {
    // Decode on a owned copy so the closure is UnwindSafe (no &mut references).
    let data = data.to_vec();
    let rgba = panic::catch_unwind(|| {
        image::ImageReader::new(Cursor::new(&data))
            .with_guessed_format()
            .map_err(image::ImageError::IoError)
            .and_then(image::ImageReader::decode)
    })
    .map_err(|_| anyhow::anyhow!("image decoder panicked"))?
    .map_err(|e| anyhow::anyhow!("{e}"))?
    .to_rgba8();

    let (w, h) = (rgba.width(), rgba.height());
    let pixels_rgba: &[RGBA8] = rgba.as_raw().as_rgba();

    let src = ImageSource::Rgba(ImgRef::new(pixels_rgba, w as usize, h as usize));
    let image_id = canvas.create_image(src, flags)?;
    Ok((image_id, rgba.into_raw(), w, h))
}

/// Draw a sub-rectangle of a bitmap: sample from `(sx, sy, sw, sh)` in the source
/// and render it into `(dx, dy, dw, dh)` on the canvas.
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

    let paint = Paint::image(image_id, ox, oy, ow, oh, 0.0, 1.0).with_anti_alias(false);
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
            canvas, image_id, src_w, src_h, sx, sy, sw, sh, dx, dy, dw, dh,
        );
    }
}

/// Render a registered bitmap onto the canvas at the given rect.
pub fn draw_bitmap(
    canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    image_id: ImageId,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    let paint = Paint::image(image_id, x, y, w, h, 0.0, 1.0).with_anti_alias(false);
    let mut path = Path::new();
    path.rect(x, y, w, h);
    canvas.fill_path(&path, &paint);
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::*;
    use crate::test_harness::GlHarness;

    /// 1×1 transparent RGBA PNG, encoded each call to keep the test
    /// self-contained (no embedded fixture bytes to drift).
    fn minimal_png() -> Vec<u8> {
        use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
        use std::io::Cursor;

        let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgba([0, 0, 0, 0]));
        let mut buf = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, ImageFormat::Png)
            .expect("BUG: PNG encode should succeed");
        buf.into_inner()
    }

    #[test]
    fn evict_removes_tag_id_and_image() {
        let harness = GlHarness::new().expect("BUG: headless GL setup failed");
        let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
        let mut reg = BitmapRegistry::new();
        let png = minimal_png();

        let id = reg
            .register("crate::bmp", &png, &mut canvas, ImageFlags::empty())
            .expect("BUG: register should succeed");
        assert!(reg.get(id).is_some());

        assert!(reg.evict("crate::bmp", &mut canvas));
        assert!(reg.get(id).is_none());
        // Idempotent: a second evict on the same tag is a no-op.
        assert!(!reg.evict("crate::bmp", &mut canvas));
    }

    #[test]
    fn evict_prefix_only_touches_matching_tags() {
        let harness = GlHarness::new().expect("BUG: headless GL setup failed");
        let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
        let mut reg = BitmapRegistry::new();
        let png = minimal_png();

        let _ = reg
            .register("a::1", &png, &mut canvas, ImageFlags::empty())
            .expect("BUG: a::1");
        let _ = reg
            .register("a::2", &png, &mut canvas, ImageFlags::empty())
            .expect("BUG: a::2");
        let id_b = reg
            .register("b::1", &png, &mut canvas, ImageFlags::empty())
            .expect("BUG: b::1");

        assert_eq!(reg.evict_prefix("a::", &mut canvas), 2);
        assert!(reg.get(id_b).is_some());
    }

    #[test]
    fn register_after_evict_uses_fresh_id() {
        let harness = GlHarness::new().expect("BUG: headless GL setup failed");
        let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
        let mut reg = BitmapRegistry::new();
        let png = minimal_png();

        let id1 = reg
            .register("ephemeral", &png, &mut canvas, ImageFlags::empty())
            .expect("BUG: first register");
        assert!(reg.evict("ephemeral", &mut canvas));
        let id2 = reg
            .register("ephemeral", &png, &mut canvas, ImageFlags::empty())
            .expect("BUG: re-register");
        // IDs are not recycled — eviction frees resources, not slots.
        assert_ne!(id1, id2);
    }
}
