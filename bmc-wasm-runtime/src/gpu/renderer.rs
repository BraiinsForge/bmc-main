// Copyright (C) 2026  Braiins Systems s.r.o.

//! FemtoVG-based GPU renderer implementing the [`Renderer`] trait.
//!
//! Wraps a `femtovg::Canvas<OpenGl>` for shape/text drawing and a
//! `cosmic_text::FontSystem` + [`ParagraphLayoutCache`] for rich-text paragraph layout.

#![expect(clippy::cast_precision_loss)]

use std::ffi::c_void;
use std::num::NonZeroU32;

use anyhow::Result;
use cosmic_text::fontdb;
use femtovg::renderer::OpenGl;
use femtovg::{Canvas, Color, FontId, Paint, Path, RenderTarget};

use super::bitmap::BitmapRegistry;
use super::icons::IconRegistry;
use super::sphere::SphereRenderer;
use super::text::{ParagraphLayoutCache, to_femtovg_color};
use crate::renderer::Renderer;
use crate::tree::{SpanData, TextStyle};

// Embed BraiinsSans fonts at compile time.
const FONT_REGULAR: &[u8] =
    include_bytes!("../../../bmc-display/ui/assets/fonts/BraiinsSans-Regular.otf");
const FONT_BOLD: &[u8] =
    include_bytes!("../../../bmc-display/ui/assets/fonts/BraiinsSans-Bold.otf");

/// GPU-accelerated renderer backed by FemtoVG (OpenGL ES 2.0+).
///
/// Owns the FemtoVG canvas, font IDs, cosmic-text `FontSystem`, and a
/// paragraph layout cache. Created once per runtime lifetime.
pub struct FemtoVgRenderer {
    gl: glow::Context,
    canvas: Canvas<OpenGl>,
    font_regular: FontId,
    font_bold: FontId,
    font_system: cosmic_text::FontSystem,
    paragraph_cache: ParagraphLayoutCache,
    icon_registry: IconRegistry,
    bitmap_registry: BitmapRegistry,
    sphere: Option<SphereRenderer>,
    width: f32,
    height: f32,
    frame_counter: u64,
}

impl std::fmt::Debug for FemtoVgRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FemtoVgRenderer")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("frame_counter", &self.frame_counter)
            .finish_non_exhaustive()
    }
}

impl FemtoVgRenderer {
    /// Create a new GPU renderer targeting a specific FBO.
    ///
    /// The `fbo_id` is the OpenGL framebuffer object that FemtoVG should render to.
    /// This is typically the staging FBO from the EGL two-FBO pipeline.
    ///
    /// Loads BraiinsSans fonts into both FemtoVG (for GPU glyph rendering) and
    /// cosmic-text (for paragraph shaping/layout). The cosmic-text `FontSystem`
    /// uses an empty DB with only the embedded fonts — no system font discovery.
    ///
    /// # Safety
    /// `load_fn` must return valid OpenGL function pointers for the current GL context.
    pub unsafe fn new<F>(mut load_fn: F, width: u32, height: u32, fbo_id: u32) -> Result<Self>
    where
        F: FnMut(&str) -> *const c_void,
    {
        // Create glow context for direct GL access (globe renderer, etc.)
        let gl = unsafe { glow::Context::from_loader_function(&mut load_fn) };

        // Create FemtoVG OpenGL renderer (shares the same GL context)
        let mut gl_renderer = unsafe { OpenGl::new_from_function(&mut load_fn) }?;

        // Set the FBO as screen target BEFORE creating the Canvas.
        // This is critical for rendering to DMA-BUF exports.
        if let Some(fbo) = NonZeroU32::new(fbo_id) {
            gl_renderer.set_screen_target(Some(glow::NativeFramebuffer(fbo)));
            tracing::info!("FemtoVG screen target set to FBO {fbo_id}");
        } else {
            tracing::info!("FBO id is 0, using default screen target");
        }

        let mut canvas = Canvas::new(gl_renderer)?;
        canvas.set_size(width, height, 1.0);

        // Load fonts into FemtoVG for GPU rendering
        let font_regular = canvas.add_font_mem(FONT_REGULAR)?;
        let font_bold = canvas.add_font_mem(FONT_BOLD)?;

        // Build cosmic-text FontSystem with only our embedded fonts.
        // Loading the same two files keeps glyph advances in sync with FemtoVG.
        let mut db = fontdb::Database::new();
        db.load_font_data(FONT_REGULAR.to_vec());
        db.load_font_data(FONT_BOLD.to_vec());
        let font_system = cosmic_text::FontSystem::new_with_locale_and_db("en-US".into(), db);

        let mut icon_registry = IconRegistry::new();
        icon_registry.register_builtins();

        Ok(Self {
            gl,
            canvas,
            font_regular,
            font_bold,
            font_system,
            paragraph_cache: ParagraphLayoutCache::new(),
            icon_registry,
            bitmap_registry: BitmapRegistry::new(),
            sphere: None,
            width: width as f32,
            height: height as f32,
            frame_counter: 0,
        })
    }
}

// ── Renderer trait implementation ───────────────────────────────────

impl Renderer for FemtoVgRenderer {
    // -- Shapes --

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let mut path = Path::new();
        path.rect(x, y, w, h);
        self.canvas
            .fill_path(&path, &Paint::color(to_femtovg_color(color)));
    }

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u32) {
        let mut path = Path::new();
        path.rounded_rect(x, y, w, h, radius);
        self.canvas
            .fill_path(&path, &Paint::color(to_femtovg_color(color)));
    }

    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: u32) {
        let mut path = Path::new();
        path.circle(cx, cy, r);
        self.canvas
            .fill_path(&path, &Paint::color(to_femtovg_color(color)));
    }

    fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, border_width: f32, color: u32) {
        let mut path = Path::new();
        path.rect(x, y, w, h);
        let mut paint = Paint::color(to_femtovg_color(color));
        paint.set_line_width(border_width);
        self.canvas.stroke_path(&path, &paint);
    }

    fn draw_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: u32) {
        let mut path = Path::new();
        path.move_to(x1, y1);
        path.line_to(x2, y2);
        let mut paint = Paint::color(to_femtovg_color(color));
        paint.set_line_width(width);
        self.canvas.stroke_path(&path, &paint);
    }

    // -- Paths --

    fn stroke_path(
        &mut self,
        points: &[(f32, f32)],
        stroke_width: f32,
        color: u32,
        closed: bool,
        smooth: bool,
    ) {
        if points.len() < 2 {
            return;
        }
        let path = build_femtovg_path(points, closed, smooth);
        let mut paint = Paint::color(to_femtovg_color(color));
        paint.set_line_width(stroke_width);
        paint.set_line_cap(femtovg::LineCap::Round);
        paint.set_line_join(femtovg::LineJoin::Round);
        self.canvas.stroke_path(&path, &paint);
    }

    fn fill_path_points(&mut self, points: &[(f32, f32)], color: u32, smooth: bool) {
        if points.len() < 3 {
            return;
        }
        let path = build_femtovg_path(points, true, smooth);
        self.canvas
            .fill_path(&path, &Paint::color(to_femtovg_color(color)));
    }

    // -- Transform stack --

    fn save(&mut self) {
        self.canvas.save();
    }

    fn restore(&mut self) {
        self.canvas.restore();
    }

    fn translate(&mut self, x: f32, y: f32) {
        self.canvas.translate(x, y);
    }

    fn rotate(&mut self, angle_radians: f32) {
        self.canvas.rotate(angle_radians);
    }

    // -- Scissor clipping --

    fn push_scissor(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.canvas.save();
        self.canvas.scissor(x, y, w, h);
    }

    fn pop_scissor(&mut self) {
        self.canvas.restore();
    }

    // -- Simple text --

    fn draw_text(&mut self, text: &str, x: f32, y: f32, size: f32, color: u32) {
        let mut paint = Paint::color(to_femtovg_color(color));
        paint.set_font(&[self.font_regular]);
        paint.set_font_size(size);
        paint.set_text_baseline(femtovg::Baseline::Top);
        let _ = self.canvas.fill_text(x, y, text, &paint);
    }

    fn measure_text(&mut self, text: &str, size: f32) -> f32 {
        let mut paint = Paint::color(Color::white());
        paint.set_font(&[self.font_regular]);
        paint.set_font_size(size);
        self.canvas
            .measure_text(0.0, 0.0, text, &paint)
            .map_or(0.0, |m| m.width())
    }

    // -- Rich text paragraphs --

    fn measure_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
    ) -> (f32, f32) {
        self.paragraph_cache
            .measure(&mut self.font_system, style, spans, max_width)
    }

    fn draw_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
    ) {
        self.paragraph_cache.draw(
            &mut self.font_system,
            &mut self.canvas,
            self.font_regular,
            self.font_bold,
            style,
            spans,
            x,
            y,
            max_width,
        );
    }

    fn draw_paragraph_clipped(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
        clip_top: f32,
        clip_bottom: f32,
    ) {
        self.push_scissor(x, clip_top, max_width, clip_bottom - clip_top);
        self.draw_paragraph(style, spans, x, y, max_width);
        self.pop_scissor();
    }

    // -- Icons --

    fn register_icon(&mut self, data: &[u8]) -> u16 {
        self.icon_registry.register(data)
    }

    fn draw_icon(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32, icon_id: u16) {
        if let Some(icon) = self.icon_registry.get(icon_id) {
            super::icons::draw_icon(&mut self.canvas, icon, x, y, w, h, color);
        }
    }

    // -- Bitmaps --

    fn register_bitmap(&mut self, data: &[u8]) -> u16 {
        self.bitmap_registry.register(data, &mut self.canvas)
    }

    fn draw_bitmap(&mut self, x: f32, y: f32, w: f32, h: f32, bitmap_id: u16) {
        if let Some(image_id) = self.bitmap_registry.get(bitmap_id) {
            super::bitmap::draw_bitmap(&mut self.canvas, image_id, x, y, w, h);
        }
    }

    #[expect(clippy::many_single_char_names)]
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
    ) {
        // Lazy-init sphere renderer on first call
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        if self.sphere.is_none() {
            match SphereRenderer::new(&self.gl, &mut self.canvas, w as u32, h as u32) {
                Ok(s) => self.sphere = Some(s),
                Err(e) => {
                    tracing::error!("sphere init failed: {e}");
                    self.draw_bitmap(x, y, w, h, bitmap_id);
                    return;
                }
            }
        }
        let sphere = self
            .sphere
            .as_mut()
            .expect("BUG: sphere is initialized above");

        // Lazy-init texture from the registered bitmap
        if !sphere.has_texture()
            && let Some(image_id) = self.bitmap_registry.get(bitmap_id)
            && let Ok(tex) = self.canvas.get_native_texture(image_id)
        {
            sphere.set_texture(tex);
        }

        // When light is NaN, pass zero-vector to disable shading
        let (sl, sn) = if light_lat.is_nan() {
            (0.0, 0.0)
        } else {
            (light_lat, light_lon)
        };

        // Render sphere to offscreen FBO (skips if params unchanged)
        sphere.render(&self.gl, center_lat, center_lon, zoom, sl, sn, atmosphere);

        // Draw the FBO texture via femtovg
        let image_id = sphere.image_id();
        super::bitmap::draw_bitmap(&mut self.canvas, image_id, x, y, w, h);
    }

    // -- Frame lifecycle --

    fn begin_frame(&mut self, width: u32, height: u32) {
        self.width = width as f32;
        self.height = height as f32;
        self.frame_counter += 1;
        // Ensure we render to the configured screen target (the staging FBO)
        self.canvas.set_render_target(RenderTarget::Screen);
        self.canvas.set_size(width, height, 1.0);
        self.canvas
            .clear_rect(0, 0, width, height, Color::rgbf(0.0, 0.0, 0.0));
        self.paragraph_cache.begin_frame(self.frame_counter);
    }

    fn flush(&mut self) {
        self.canvas.flush();
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn height(&self) -> f32 {
        self.height
    }
}

/// Build a FemtoVG `Path` from a sequence of points.
///
/// - `smooth = false`: straight line segments (`move_to` + `line_to`).
/// - `smooth = true`: Catmull-Rom spline converted to cubic Bézier curves.
///   Each segment between `p[i]` and `p[i+1]` uses control points derived
///   from neighboring points, producing a smooth curve through all points.
fn build_femtovg_path(points: &[(f32, f32)], closed: bool, smooth: bool) -> Path {
    let mut path = Path::new();

    if smooth && points.len() >= 2 {
        let n = points.len();
        path.move_to(points[0].0, points[0].1);

        for i in 0..n - 1 {
            let p0 = points[if i == 0 { 0 } else { i - 1 }];
            let p1 = points[i];
            let p2 = points[i + 1];
            let p3 = points[if i + 2 < n { i + 2 } else { n - 1 }];

            // Catmull-Rom → cubic Bézier control points (tension = 1.0)
            let cp1x = p1.0 + (p2.0 - p0.0) / 6.0;
            let cp1y = p1.1 + (p2.1 - p0.1) / 6.0;
            let cp2x = p2.0 - (p3.0 - p1.0) / 6.0;
            let cp2y = p2.1 - (p3.1 - p1.1) / 6.0;

            path.bezier_to(cp1x, cp1y, cp2x, cp2y, p2.0, p2.1);
        }
    } else {
        path.move_to(points[0].0, points[0].1);
        for &(x, y) in &points[1..] {
            path.line_to(x, y);
        }
    }

    if closed {
        path.close();
    }
    path
}
