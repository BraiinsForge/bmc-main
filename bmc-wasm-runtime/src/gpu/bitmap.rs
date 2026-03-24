// Copyright (C) 2026  Braiins Systems s.r.o.

//! Bitmap registry — decodes raster images and uploads them as GPU textures.
//!
//! Bitmaps are registered once (on first use from WASM) and persist for the
//! runtime lifetime. Each registered bitmap gets an opaque `u16` ID that maps
//! to a FemtoVG `ImageId` (GPU texture handle).

use std::collections::HashMap;
use std::fmt;
use std::io::Cursor;

use femtovg::{ImageFlags, ImageId, ImageSource, Paint, Path};
use imgref::ImgRef;
use rgb::{FromSlice as _, RGBA8};

/// Registry mapping opaque widget-side IDs to FemtoVG GPU texture handles.
pub struct BitmapRegistry {
    bitmaps: HashMap<u16, ImageId>,
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
            Ok(image_id) => {
                self.bitmaps.insert(id, image_id);
            }
            Err(e) => {
                tracing::error!("failed to decode/upload bitmap: {e}");
            }
        }
        id
    }

    /// Get the FemtoVG ImageId for a registered bitmap.
    #[must_use]
    pub fn get(&self, id: u16) -> Option<ImageId> {
        self.bitmaps.get(&id).copied()
    }
}

/// Decode image bytes to RGBA and upload to the GPU as a FemtoVG texture.
fn decode_and_upload(
    data: &[u8],
    canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
) -> anyhow::Result<ImageId> {
    let img = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()?
        .decode()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);

    let pixels: &[RGBA8] = rgba.as_raw().as_rgba();

    let src = ImageSource::Rgba(ImgRef::new(pixels, w, h));
    let image_id = canvas.create_image(src, ImageFlags::empty())?;
    Ok(image_id)
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
