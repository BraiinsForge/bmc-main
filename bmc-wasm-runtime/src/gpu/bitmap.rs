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
pub struct BitmapRegistry {
    bitmaps: HashMap<u16, StoredBitmap>,
    next_id: u16,
}

impl fmt::Debug for BitmapRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitmapRegistry")
            .field("count", &self.bitmaps.len())
            .field("next_id", &self.next_id)
            .finish()
    }
}

impl BitmapRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bitmaps: HashMap::new(),
            next_id: 1,
        }
    }

    /// Decode image bytes (PNG, JPEG, etc.) and upload to GPU as a texture.
    /// Returns the assigned bitmap ID.
    pub fn register(
        &mut self,
        data: &[u8],
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
    ) -> u16 {
        let id = self.next_id;
        self.next_id += 1;

        match decode_and_upload(data, canvas) {
            Ok((image_id, pixels, width, height)) => {
                self.bitmaps.insert(
                    id,
                    StoredBitmap {
                        image_id,
                        pixels,
                        width,
                        height,
                    },
                );
            }
            Err(e) => {
                tracing::error!("failed to decode/upload bitmap: {e}");
            }
        }
        id
    }

    /// Get the FemtoVG `ImageId` for a registered bitmap.
    #[must_use]
    pub fn get(&self, id: u16) -> Option<ImageId> {
        self.bitmaps.get(&id).map(|b| b.image_id)
    }

    /// Sample the average RGBA color of a rectangular region within a registered bitmap.
    ///
    /// Coordinates are clamped to the bitmap dimensions. Returns `None` if the bitmap ID
    /// is not registered or the region is empty after clamping.
    #[must_use]
    #[expect(clippy::many_single_char_names)]
    pub fn sample(&self, id: u16, x: u32, y: u32, w: u32, h: u32) -> Option<u32> {
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
    let image_id = canvas.create_image(src, ImageFlags::empty())?;
    Ok((image_id, rgba.into_raw(), w, h))
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
    let paint = Paint::image(image_id, x, y, w, h, 0.0, 1.0);
    let mut path = Path::new();
    path.rect(x, y, w, h);
    canvas.fill_path(&path, &paint);
}
