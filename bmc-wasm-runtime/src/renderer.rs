// Copyright (C) 2026  Braiins Systems s.r.o.

//! Abstract rendering backend for the widget tree.
//!
//! All coordinates are f32. Implementations may round to integer
//! internally if the backend requires it.

use crate::tree::{SpanData, TextStyle};

/// Rendering backend trait.
///
/// Implemented by [`crate::gpu::FemtoVgRenderer`] for GPU-accelerated rendering.
/// The trait is object-safe so `process_tree` can accept `&mut dyn Renderer`.
pub trait Renderer {
    // -- Shapes --

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32);

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u32);

    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: u32);

    fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, border_width: f32, color: u32);

    fn draw_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: u32);

    // -- Transform stack --

    fn save(&mut self);
    fn restore(&mut self);
    fn translate(&mut self, x: f32, y: f32);
    fn rotate(&mut self, angle_radians: f32);

    // -- Scissor clipping --

    fn push_scissor(&mut self, x: f32, y: f32, w: f32, h: f32);
    fn pop_scissor(&mut self);

    // -- Simple text --

    fn draw_text(&mut self, text: &str, x: f32, y: f32, size: f32, color: u32);
    fn measure_text(&mut self, text: &str, size: f32) -> f32;

    // -- Rich text paragraphs --

    fn measure_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
    ) -> (f32, f32);

    fn draw_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
    );

    #[expect(clippy::too_many_arguments)]
    fn draw_paragraph_clipped(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
        clip_top: f32,
        clip_bottom: f32,
    );

    // -- Icons --

    /// Register icon data (compact binary from proc macro), returns opaque ID.
    fn register_icon(&mut self, data: &[u8]) -> u16;

    /// Draw a registered icon at the given position and size.
    fn draw_icon(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32, icon_id: u16);

    // -- Bitmaps --

    /// Register bitmap data (PNG/JPEG bytes), decode and upload to GPU. Returns opaque ID.
    fn register_bitmap(&mut self, data: &[u8]) -> u16;

    /// Draw a registered bitmap at the given position and size.
    fn draw_bitmap(&mut self, x: f32, y: f32, w: f32, h: f32, bitmap_id: u16);

    // -- Frame lifecycle --

    fn begin_frame(&mut self, width: u32, height: u32);
    fn flush(&mut self);
    fn width(&self) -> f32;
    fn height(&self) -> f32;
}
