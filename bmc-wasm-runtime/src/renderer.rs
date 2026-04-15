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
    #[expect(clippy::too_many_arguments)]
    fn draw_icon(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
        icon_id: u16,
        anti_alias: bool,
    );

    // -- Bitmaps --

    /// Register bitmap data (PNG/JPEG bytes), decode and upload to GPU. Returns opaque ID.
    fn register_bitmap(&mut self, data: &[u8]) -> u16;

    /// Register bitmap with nearest-neighbor filtering (no bilinear interpolation).
    /// Use for pixel-art assets (9-patch skins) where bilinear filtering would
    /// cause color bleeding across sub-rect boundaries.
    fn register_bitmap_nearest(&mut self, data: &[u8]) -> u16;

    /// Draw a registered bitmap at the given position and size.
    fn draw_bitmap(&mut self, x: f32, y: f32, w: f32, h: f32, bitmap_id: u16);

    /// Draw a 9-patch bitmap: slice into 9 quads using insets and stretch appropriately.
    ///
    /// Corners stay fixed, edges stretch in one axis, center stretches both.
    /// Insets define the distance from each edge to the stretchable region boundary.
    #[expect(clippy::too_many_arguments)]
    fn draw_nine_patch(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: u16,
        left: u16,
        top: u16,
        right: u16,
        bottom: u16,
    );

    /// Sample the average color of a rectangular region within a registered bitmap.
    ///
    /// Returns the average RGBA as a packed `u32` (`0xRRGGBBAA`), or `None` if the
    /// bitmap ID is invalid or the region is empty.
    fn bitmap_sample(&self, bitmap_id: u16, x: u32, y: u32, w: u32, h: u32) -> Option<u32>;

    // -- Meshes --

    /// Register mesh binary data, upload VBO/IBO/texture to GPU. Returns opaque ID.
    fn register_mesh(&mut self, data: &[u8]) -> u16;

    /// Draw a 3D mesh with quaternion-based orientation and optional directional light.
    ///
    /// `slot_index` selects which atlas slot to render into (0..8 for 3×3 grid).
    /// When `light_pitch` is `f32::NAN`, lighting is disabled.
    #[expect(clippy::too_many_arguments)]
    fn draw_mesh(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        slot_index: u8,
        mesh_id: u16,
        fov: f32,
        distance: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
        px: f32,
        py: f32,
        pz: f32,
        scale: f32,
        light_pitch: f32,
        light_yaw: f32,
        ambient: f32,
        specular: f32,
        hl_u_min: f32,
        hl_v_min: f32,
        hl_u_max: f32,
        hl_v_max: f32,
        hl_r: f32,
        hl_g: f32,
        hl_b: f32,
    );

    // -- Sphere --

    /// Draw an equirectangular texture mapped onto a 3D sphere with optional light shading.
    ///
    /// When `light_lat` is `f32::NAN`, shading is disabled (full brightness).
    /// When `atmosphere` is true, adds limb darkening and bluish edge glow.
    #[expect(clippy::too_many_arguments)]
    fn draw_sphere(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: u16,
        center_lat: f32,
        center_lon: f32,
        zoom: f32,
        light_lat: f32,
        light_lon: f32,
        atmosphere: bool,
    );

    // -- Canvas text --

    /// Draw styled text on a canvas at an explicit position.
    ///
    /// Alignment is handled by the caller via `TextStyle.align`:
    /// Left = text starts at x, Center = centered on x, Right = text ends at x.
    fn draw_canvas_text(&mut self, text: &str, x: f32, y: f32, style: &TextStyle);

    // -- Paths --

    /// Stroke a path through the given points.
    /// If `smooth` is true, use Catmull-Rom spline interpolation.
    /// If `closed` is true, join last point to first.
    fn stroke_path(
        &mut self,
        points: &[(f32, f32)],
        stroke_width: f32,
        color: u32,
        closed: bool,
        smooth: bool,
    );

    /// Fill a closed path through the given points.
    /// If `smooth` is true, use Catmull-Rom spline interpolation.
    fn fill_path_points(&mut self, points: &[(f32, f32)], color: u32, smooth: bool);

    // -- Frame lifecycle --

    /// Begin a frame with DPI scaling.
    ///
    /// `dpi_scale` > 1.0 renders at higher internal resolution for sharper text.
    /// The coordinate system stays at `width × height` logical pixels.
    fn begin_frame(&mut self, width: u32, height: u32, dpi_scale: f32);
    fn flush(&mut self);
    fn width(&self) -> f32;
    fn height(&self) -> f32;
}
