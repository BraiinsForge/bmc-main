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

//! FemtoVG-based GPU renderer implementing the [`Renderer`] trait.
//!
//! Wraps a `femtovg::Canvas<OpenGl>` for shape/text drawing and a
//! `cosmic_text::FontSystem` + [`ParagraphLayoutCache`] for rich-text paragraph layout.

#![expect(clippy::cast_precision_loss)]

use std::ffi::c_void;
use std::num::NonZeroU32;

use anyhow::Result;
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{
    ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, BitmapId, Fill, MeshId, SvgId, WHITE,
};
use cosmic_text::fontdb;
use femtovg::renderer::OpenGl;
use femtovg::{Canvas, FontId, LineCap, Paint, Path, RenderTarget, Solidity};
use glow::HasContext;

use super::bitmap::BitmapRegistry;
use super::curved_text::arc_glyph_layout;
use super::mesh::{MeshDrawArgs, MeshRenderer, MeshReservations};
use super::sphere::SphereRenderer;
use super::svg::SvgRegistry;
use super::text::{
    Fonts, ParagraphLayoutCache, WeightedFonts, autofit_bounds, search_fit_size, to_femtovg_color,
};
use crate::renderer::{FrameClear, Renderer};
use crate::tree::{AutoFit, SpanData, TextAlign, TextStyle, VerticalAlign};

// Embed BraiinsSans fonts at compile time from the top-level assets directory.
const FONT_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/BraiinsSans-Regular.otf");
const FONT_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/BraiinsSans-SemiBold.otf");
const FONT_BOLD: &[u8] = include_bytes!("../../../assets/fonts/BraiinsSans-Bold.otf");
// BraiinsDeckSans is the display face used by the legacy slint deck;
// widgets opt in via `FontFamily::DeckSans` in their `TextStyle`.
const FONT_DECK_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/BraiinsDeckSans-Regular.otf");
const FONT_DECK_SEMIBOLD: &[u8] =
    include_bytes!("../../../assets/fonts/BraiinsDeckSans-SemiBold.otf");
const FONT_DECK_BOLD: &[u8] = include_bytes!("../../../assets/fonts/BraiinsDeckSans-Bold.otf");
/// Fallback font for glyphs not covered by the Braiins faces (Greek, symbols, etc.).
const FONT_FALLBACK: &[u8] = include_bytes!("../../../assets/fonts/NotoSans-Regular.ttf");

/// cosmic-text `FontSystem` holding the embedded faces and nothing else.
///
/// Paragraphs request "Braiins Sans" or "Braiins Deck Sans" by family name (see
/// [`super::text::build_attrs`]), so cosmic-text always prefers them; Noto Sans
/// covers only the glyphs the Braiins faces don't (Greek, Cyrillic, …).
///
/// Deliberately not `FontSystem::new()`, which scans the host's installed fonts:
/// shaping would then vary with whatever the machine happens to have, and the
/// Nix build sandbox has nothing at all — cosmic-text panics there with "no
/// default font found". Tests shape through this for the same reason.
pub(crate) fn build_font_system() -> cosmic_text::FontSystem {
    let mut db = fontdb::Database::new();
    db.load_font_data(FONT_REGULAR.to_vec());
    db.load_font_data(FONT_SEMIBOLD.to_vec());
    db.load_font_data(FONT_BOLD.to_vec());
    db.load_font_data(FONT_DECK_REGULAR.to_vec());
    db.load_font_data(FONT_DECK_SEMIBOLD.to_vec());
    db.load_font_data(FONT_DECK_BOLD.to_vec());
    db.load_font_data(FONT_FALLBACK.to_vec());
    cosmic_text::FontSystem::new_with_locale_and_db("en-US".into(), db)
}

/// Offscreen textures for `drop_shadow`, kept alive across frames
/// so the two textures aren't reallocated every frame;
/// resized only on a size change.
struct ShadowFboPool {
    width: u32,
    height: u32,
    unblurred: femtovg::ImageId,
    blurred: femtovg::ImageId,
}

/// GPU-accelerated renderer backed by FemtoVG (OpenGL ES 2.0+).
///
/// Owns the FemtoVG canvas, font IDs, cosmic-text `FontSystem`, and a
/// paragraph layout cache. Created once per runtime lifetime.
pub struct FemtoVgRenderer {
    gl: glow::Context,
    canvas: Canvas<OpenGl>,
    /// The FBO that FemtoVG should render to (stored separately because FemtoVG's
    /// `Canvas::set_render_target(Screen)` skips the `SetRenderTarget` command when
    /// `current_render_target` is already `Screen` — which is always true since it's
    /// the default. We must explicitly bind the FBO before each flush.)
    screen_fbo: Option<glow::NativeFramebuffer>,
    fonts: Fonts,
    font_fallback: FontId,
    font_system: cosmic_text::FontSystem,
    paragraph_cache: ParagraphLayoutCache,
    icon_registry: SvgRegistry,
    bitmap_registry: BitmapRegistry,
    sphere: Option<SphereRenderer>,
    /// `BitmapId` currently bound as the sphere's source texture. Used to
    /// detect rebind-on-change in `draw_sphere`; a mismatch (incl. the
    /// post-evict case where the registry has dropped the id) re-fetches
    /// the native texture so we never sample a deleted GL name.
    sphere_bitmap_id: Option<BitmapId>,
    mesh_renderer: Option<MeshRenderer>,
    pending_mesh_reservations: MeshReservations,
    mesh_msaa_samples: u32,
    width: f32,
    height: f32,
    /// Device-pixel ratio from the last `begin_frame`;
    /// shadow FBOs are sized in physical pixels off it.
    dpi_scale: f32,
    /// What the frame in progress draws into, so a pass that borrows the target
    /// can hand it back. `drop_shadow` renders its own offscreens mid-frame,
    /// and restoring `Screen` blindly would strand the rest of an image-backed frame
    /// on the window.
    frame_target: RenderTarget,
    shadow_fbo_pool: Option<ShadowFboPool>,
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

/// Re-export femtovg's `ImageId` for use by gallery stage rendering.
pub type FemtovgImageId = femtovg::ImageId;

impl FemtoVgRenderer {
    /// Direct access to the underlying femtovg canvas (for testbed-only effects).
    pub fn canvas_mut(&mut self) -> &mut Canvas<OpenGl> {
        &mut self.canvas
    }

    /// Create a femtovg-managed render target image.
    ///
    /// Returns `(ImageId, raw_gl_texture_name)`. femtovg owns the texture and
    /// its internal FBO+stencil. The raw GL name can be registered with egui
    /// for display.
    pub fn create_render_target(&mut self, width: u32, height: u32) -> (femtovg::ImageId, u32) {
        let image_id = self
            .canvas
            .create_image_empty(
                width as usize,
                height as usize,
                femtovg::PixelFormat::Rgba8,
                femtovg::ImageFlags::empty(),
            )
            .expect("BUG: failed to create femtovg render target image");
        let native = self
            .canvas
            .get_native_texture(image_id)
            .expect("BUG: failed to get native texture from femtovg image");
        let gl_name = native.0.get();
        tracing::debug!(?image_id, gl_name, width, height, "created render target");
        (image_id, gl_name)
    }

    /// Begin a frame targeting a femtovg Image (offscreen render target).
    ///
    /// Like [`Renderer::begin_frame`] but renders to the given Image instead of
    /// the screen FBO. femtovg manages its own FBO with stencil for the Image.
    ///
    /// `set_size` sets the canvas device-pixel ratio (logical → physical scaling,
    /// canvas-level) but queues `SetRenderTarget(Screen)`, so bind the Image after
    /// it. The image must be `width·dpi_scale × height·dpi_scale` texels.
    pub fn begin_frame_to_image(
        &mut self,
        image_id: femtovg::ImageId,
        width: u32,
        height: u32,
        dpi_scale: f32,
    ) {
        self.width = width as f32;
        self.height = height as f32;
        self.dpi_scale = dpi_scale;
        self.frame_counter += 1;
        self.canvas.set_size(width, height, dpi_scale);
        // femtovg 0.20.4: `set_size` queues a switch to the screen without updating the
        // target it remembers, so `set_render_target` skips the switch back as redundant
        // and a second frame into the same image lands on the screen, freezing the image.
        self.canvas.set_render_target(RenderTarget::Screen);
        self.canvas.set_render_target(RenderTarget::Image(image_id));
        self.frame_target = RenderTarget::Image(image_id);
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (pw, ph) = (
            (width as f32 * dpi_scale) as u32,
            (height as f32 * dpi_scale) as u32,
        );
        self.canvas
            .clear_rect(0, 0, pw, ph, femtovg::Color::rgbaf(0.0, 0.0, 0.0, 0.0));
        self.paragraph_cache.begin_frame(self.frame_counter);
    }

    /// Run `inner` translated to the canvas origin `(cx, cy)`.
    /// Used by the `drop_shadow` fallbacks,
    /// since the closure draws at FBO-local `(0, 0)`.
    fn render_inner_translated(
        &mut self,
        cx: f32,
        cy: f32,
        inner: &mut dyn FnMut(&mut dyn Renderer),
    ) {
        self.canvas.save();
        self.canvas.translate(cx, cy);
        inner(self);
        self.canvas.restore();
    }

    /// Borrow the pooled FBO pair at `width × height`,
    /// allocating or resizing as needed.
    /// `None` only if the GPU refuses the allocation.
    ///
    /// Single pair: two `drop_shadow` calls in one frame
    /// at different sizes would realloc each call.
    ///
    /// Clock shadows are all canvas-sized, so they don't;
    /// a multi-entry pool would be needed otherwise.
    fn acquire_shadow_fbos(
        &mut self,
        width: u32,
        height: u32,
    ) -> Option<(femtovg::ImageId, femtovg::ImageId)> {
        if let Some(pool) = &self.shadow_fbo_pool
            && pool.width == width
            && pool.height == height
        {
            return Some((pool.unblurred, pool.blurred));
        }
        if let Some(stale) = self.shadow_fbo_pool.take() {
            self.canvas.delete_image(stale.unblurred);
            self.canvas.delete_image(stale.blurred);
        }
        let unblurred = self
            .canvas
            .create_image_empty(
                width as usize,
                height as usize,
                femtovg::PixelFormat::Rgba8,
                femtovg::ImageFlags::empty(),
            )
            .ok()?;
        let Ok(blurred) = self.canvas.create_image_empty(
            width as usize,
            height as usize,
            femtovg::PixelFormat::Rgba8,
            femtovg::ImageFlags::empty(),
        ) else {
            self.canvas.delete_image(unblurred);
            return None;
        };
        self.shadow_fbo_pool = Some(ShadowFboPool {
            width,
            height,
            unblurred,
            blurred,
        });
        Some((unblurred, blurred))
    }

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
    pub unsafe fn new<F>(
        mut load_fn: F,
        width: u32,
        height: u32,
        fbo_id: u32,
        mesh_msaa_samples: u32,
    ) -> Result<Self>
    where
        F: FnMut(&str) -> *const c_void,
    {
        // Create glow context for direct GL access (globe renderer, etc.)
        let gl = unsafe { glow::Context::from_loader_function(&mut load_fn) };

        // Create FemtoVG OpenGL renderer (shares the same GL context)
        let mut gl_renderer = unsafe { OpenGl::new_from_function(&mut load_fn) }?;

        // Store FBO for explicit binding in begin_frame (FemtoVG's
        // set_render_target(Screen) skips the GL bind when already targeting Screen).
        let screen_fbo = NonZeroU32::new(fbo_id).map(glow::NativeFramebuffer);
        if let Some(fbo) = screen_fbo {
            gl_renderer.set_screen_target(Some(fbo));
            tracing::info!("FemtoVG screen target set to FBO {fbo_id}");
        } else {
            tracing::info!("FBO id is 0, using default screen target");
        }

        let mut canvas = Canvas::new(gl_renderer)?;
        canvas.set_size(width, height, 1.0);

        // Load fonts into FemtoVG for GPU rendering
        let fonts = Fonts {
            sans: WeightedFonts {
                regular: canvas.add_font_mem(FONT_REGULAR)?,
                semibold: canvas.add_font_mem(FONT_SEMIBOLD)?,
                bold: canvas.add_font_mem(FONT_BOLD)?,
            },
            deck_sans: WeightedFonts {
                regular: canvas.add_font_mem(FONT_DECK_REGULAR)?,
                semibold: canvas.add_font_mem(FONT_DECK_SEMIBOLD)?,
                bold: canvas.add_font_mem(FONT_DECK_BOLD)?,
            },
        };
        let font_fallback = canvas.add_font_mem(FONT_FALLBACK)?;

        let font_system = build_font_system();

        let mut icon_registry = SvgRegistry::new();
        icon_registry.register_builtins();

        Ok(Self {
            gl,
            canvas,
            screen_fbo,
            fonts,
            font_fallback,
            font_system,
            paragraph_cache: ParagraphLayoutCache::new(),
            icon_registry,
            bitmap_registry: BitmapRegistry::new(),
            sphere: None,
            sphere_bitmap_id: None,
            mesh_renderer: None,
            pending_mesh_reservations: MeshReservations::default(),
            mesh_msaa_samples,
            width: width as f32,
            height: height as f32,
            dpi_scale: 1.0,
            frame_target: RenderTarget::Screen,
            shadow_fbo_pool: None,
            frame_counter: 0,
        })
    }

    /// Lazy-initialise the mesh renderer on first use. Logs and leaves
    /// `self.mesh_renderer` as `None` if creation fails — callers must
    /// observe the `None` and bail out gracefully rather than relying on
    /// "init succeeded just now" invariants.
    fn lazy_init_mesh_renderer(&mut self) {
        if self.mesh_renderer.is_some() {
            return;
        }
        match MeshRenderer::new_with_reservations(
            &self.gl,
            &mut self.canvas,
            self.mesh_msaa_samples,
            self.pending_mesh_reservations.clone(),
        ) {
            Ok(r) => {
                self.pending_mesh_reservations = MeshReservations::default();
                self.mesh_renderer = Some(r);
            }
            Err(e) => tracing::error!("mesh renderer init failed: {e}"),
        }
    }

    /// Drop every caller-registered bitmap and icon plus the lazy-init sphere
    /// and mesh renderers, returning the renderer to its post-`new` state.
    /// Shaders, fonts, the screen FBO binding, and the paragraph layout cache
    /// are preserved.
    pub fn drop_all(&mut self) {
        self.release_gpu_assets();
        self.sphere_bitmap_id = None;
        self.pending_mesh_reservations = MeshReservations::default();
        self.icon_registry = SvgRegistry::new();
        self.icon_registry.register_builtins();
    }

    fn release_gpu_assets(&mut self) {
        if let Some(sphere) = self.sphere.take() {
            sphere.destroy(&self.gl, &mut self.canvas);
        }
        if let Some(mesh) = self.mesh_renderer.take() {
            mesh.destroy(&self.gl, &mut self.canvas);
        }
        if let Some(pool) = self.shadow_fbo_pool.take() {
            self.canvas.delete_image(pool.unblurred);
            self.canvas.delete_image(pool.blurred);
        }
        self.bitmap_registry.clear(&mut self.canvas);
    }
}

impl Drop for FemtoVgRenderer {
    fn drop(&mut self) {
        self.release_gpu_assets();
    }
}

// ── Renderer trait implementation ───────────────────────────────────

impl Renderer for FemtoVgRenderer {
    // -- Shapes --

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let mut path = Path::new();
        path.rect(x, y, w, h);
        self.canvas
            .fill_path(&path, &Paint::color(to_femtovg_color(color.to_u32())));
    }

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let mut path = Path::new();
        path.rounded_rect(x, y, w, h, radius);
        self.canvas
            .fill_path(&path, &Paint::color(to_femtovg_color(color.to_u32())));
    }

    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        let mut path = Path::new();
        path.circle(cx, cy, r);
        self.canvas
            .fill_path(&path, &Paint::color(to_femtovg_color(color.to_u32())));
    }

    fn fill_rect_paint(&mut self, x: f32, y: f32, w: f32, h: f32, fill: &Fill) {
        let mut path = Path::new();
        path.rect(x, y, w, h);
        let paint = paint_for_fill(
            fill,
            (x, y, w, h),
            (x + w / 2.0, y + h / 2.0, (w / 2.0).hypot(h / 2.0)),
        );
        self.canvas.fill_path(&path, &paint);
    }

    fn fill_circle_paint(&mut self, cx: f32, cy: f32, r: f32, fill: &Fill) {
        let mut path = Path::new();
        path.circle(cx, cy, r);
        let paint = paint_for_fill(fill, (cx - r, cy - r, 2.0 * r, 2.0 * r), (cx, cy, r));
        self.canvas.fill_path(&path, &paint);
    }

    fn stroke_arc(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        width: f32,
        fill: &ArcFill,
        segments: &ArcSegments,
        cap: ArcCap,
    ) {
        let outer_cap = match cap {
            ArcCap::Round => LineCap::Round,
            ArcCap::Butt => LineCap::Butt,
        };
        let spans = arc_spans(segments, start_angle, end_angle);
        let last_span = spans.len().saturating_sub(1);
        for (si, &(s0, s1)) in spans.iter().enumerate() {
            let chunks = chunk_span(s0, s1, ARC_CHUNK_MAX);
            let last_chunk = chunks.len().saturating_sub(1);
            for (ci, &(a0, a1)) in chunks.iter().enumerate() {
                let ea0 = if ci == 0 { a0 } else { a0 - ARC_SEAM_EPS };
                let ea1 = if ci == last_chunk {
                    a1
                } else {
                    a1 + ARC_SEAM_EPS
                };

                let fa0 = arc_to_femtovg_angle(ea0);
                let fa1 = arc_to_femtovg_angle(ea1);

                let mut path = Path::new();
                path.arc(cx, cy, radius, fa0, fa1, Solidity::Hole);

                let p0 = arc_point(cx, cy, radius, ea0);
                let p1 = arc_point(cx, cy, radius, ea1);
                let c0 = arc_color_at(fill, sweep_fraction(ea0, start_angle, end_angle));
                let c1 = arc_color_at(fill, sweep_fraction(ea1, start_angle, end_angle));

                let mut paint = Paint::linear_gradient(
                    p0.0,
                    p0.1,
                    p1.0,
                    p1.1,
                    to_femtovg_color(c0.to_u32()),
                    to_femtovg_color(c1.to_u32()),
                );
                paint.set_line_width(width);
                paint.set_line_cap(LineCap::Butt);
                if si == 0 && ci == 0 {
                    paint.set_line_cap_start(outer_cap);
                }
                if si == last_span && ci == last_chunk {
                    paint.set_line_cap_end(outer_cap);
                }
                self.canvas.stroke_path(&path, &paint);
            }
        }
    }

    fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, border_width: f32, color: Color) {
        let mut path = Path::new();
        path.rect(x, y, w, h);
        let mut paint = Paint::color(to_femtovg_color(color.to_u32()));
        paint.set_line_width(border_width);
        self.canvas.stroke_path(&path, &paint);
    }

    fn stroke_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        border_width: f32,
        color: Color,
    ) {
        let mut path = Path::new();
        path.rounded_rect(x, y, w, h, radius);
        let mut paint = Paint::color(to_femtovg_color(color.to_u32()));
        paint.set_line_width(border_width);
        self.canvas.stroke_path(&path, &paint);
    }

    fn draw_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color) {
        let mut path = Path::new();
        path.move_to(x1, y1);
        path.line_to(x2, y2);
        let mut paint = Paint::color(to_femtovg_color(color.to_u32()));
        paint.set_line_width(width);
        self.canvas.stroke_path(&path, &paint);
    }

    // -- Paths --

    fn stroke_path(
        &mut self,
        points: &[(f32, f32)],
        stroke_width: f32,
        color: Color,
        closed: bool,
        smooth: bool,
    ) {
        if points.len() < 2 {
            return;
        }
        let path = build_femtovg_path(points, closed, smooth);
        let mut paint = Paint::color(to_femtovg_color(color.to_u32()));
        paint.set_line_width(stroke_width);
        paint.set_line_cap(femtovg::LineCap::Round);
        paint.set_line_join(femtovg::LineJoin::Round);
        self.canvas.stroke_path(&path, &paint);
    }

    fn fill_path_paint(&mut self, points: &[(f32, f32)], fill: &Fill, smooth: bool) {
        if points.len() < 3 {
            return;
        }
        let path = build_femtovg_path(points, true, smooth);
        let (min_x, min_y, max_x, max_y) = points.iter().fold(
            (
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ),
            |(mnx, mny, mxx, mxy), &(px, py)| (mnx.min(px), mny.min(py), mxx.max(px), mxy.max(py)),
        );
        let (w, h) = (max_x - min_x, max_y - min_y);
        let radius = (w / 2.0).hypot(h / 2.0);
        let paint = paint_for_fill(
            fill,
            (min_x, min_y, w, h),
            (min_x + w / 2.0, min_y + h / 2.0, radius),
        );
        self.canvas.fill_path(&path, &paint);
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
        // Use intersect_scissor so nested clips (e.g. canvas inside scroll) work correctly.
        // When no scissor is active, intersect_scissor acts like scissor.
        self.canvas.intersect_scissor(x, y, w, h);
    }

    fn pop_scissor(&mut self) {
        self.canvas.restore();
    }

    // -- Simple text --

    fn draw_text(&mut self, text: &str, x: f32, y: f32, size: f32, color: Color) {
        let mut paint = Paint::color(to_femtovg_color(color.to_u32()));
        paint.set_font(&[self.fonts.sans.regular, self.font_fallback]);
        paint.set_font_size(size);
        paint.set_text_baseline(femtovg::Baseline::Top);
        let _ = self.canvas.fill_text(x, y, text, &paint);
    }

    fn measure_text(&mut self, text: &str, size: f32) -> f32 {
        let mut paint = Paint::color(femtovg::Color::white());
        paint.set_font(&[self.fonts.sans.regular, self.font_fallback]);
        paint.set_font_size(size);
        self.canvas
            .measure_text(0.0, 0.0, text, &paint)
            .map_or(0.0, |m| m.width())
    }

    // -- Canvas text --

    fn draw_canvas_text(&mut self, text: &str, x: f32, y: f32, style: &TextStyle) {
        let font = self.fonts.select(style.family, style.weight);
        let size = style.size as f32;
        let mut paint = Paint::color(to_femtovg_color(style.color.to_u32()));
        paint.set_font(&[font, self.font_fallback]);
        paint.set_font_size(size);
        paint.set_text_baseline(femtovg_baseline(style.vertical_align));

        let measured_width = match style.align {
            TextAlign::Left => 0.0,
            TextAlign::Center | TextAlign::Right => self
                .canvas
                .measure_text(0.0, 0.0, text, &paint)
                .map_or(0.0, |m| m.width()),
        };

        // Alignment: measure text width and offset x for Center/Right
        let draw_x = match style.align {
            TextAlign::Left => x,
            TextAlign::Center => x - measured_width / 2.0,
            TextAlign::Right => x - measured_width,
        };

        // Text outline via 8-direction fill_text at each 1px ring up to outline_width.
        // 8 textured-quad draws per ring — cheap on GPU even on embedded.
        if style.outline_color != crate::colors::TRANSPARENT && style.outline_width > 0.0 {
            let mut outline_paint = Paint::color(to_femtovg_color(style.outline_color.to_u32()));
            outline_paint.set_font(&[font, self.font_fallback]);
            outline_paint.set_font_size(size);
            outline_paint.set_text_baseline(femtovg_baseline(style.vertical_align));
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let rings = style.outline_width.ceil() as u32;
            for ring in 1..=rings {
                let d = ring as f32;
                for &(dx, dy) in &[
                    (d, 0.0),
                    (-d, 0.0),
                    (0.0, d),
                    (0.0, -d),
                    (d, d),
                    (-d, -d),
                    (d, -d),
                    (-d, d),
                ] {
                    let _ = self
                        .canvas
                        .fill_text(draw_x + dx, y + dy, text, &outline_paint);
                }
            }
        }

        let _ = self.canvas.fill_text(draw_x, y, text, &paint);

        // Decorations
        if style.underline || style.strikethrough {
            let width = self
                .canvas
                .measure_text(0.0, 0.0, text, &paint)
                .map_or(0.0, |m| m.width());
            let thickness = (size / 14.0).max(1.0);

            if style.underline {
                let uy = y + size * 0.1;
                self.fill_rect(draw_x, uy, width, thickness, style.color);
            }
            if style.strikethrough {
                let sy = y - size * 0.3;
                self.fill_rect(draw_x, sy, width, thickness, style.color);
            }
        }
    }

    fn draw_curved_text(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        angle: f32,
        anchor: ArcAnchor,
        facing: ArcTextFacing,
        text: &str,
        style: &TextStyle,
    ) {
        if radius <= 0.0 || text.is_empty() {
            return;
        }

        let font = self.fonts.select(style.family, style.weight);
        let size = style.size as f32;
        let mut paint = Paint::color(to_femtovg_color(style.color.to_u32()));
        paint.set_font(&[font, self.font_fallback]);
        paint.set_font_size(size);
        paint.set_text_baseline(femtovg::Baseline::Middle);

        let widths: Vec<f32> = text
            .chars()
            .map(|ch| {
                let mut buf = [0_u8; 4];
                let glyph = ch.encode_utf8(&mut buf);
                self.canvas
                    .measure_text(0.0, 0.0, glyph, &paint)
                    .map_or(0.0, |metrics| metrics.width())
            })
            .collect();

        for (ch, (width, placement)) in text.chars().zip(
            widths
                .iter()
                .zip(arc_glyph_layout(&widths, radius, angle, anchor, facing)),
        ) {
            let px = cx + radius * placement.theta.sin();
            let py = cy - radius * placement.theta.cos();
            let mut buf = [0_u8; 4];
            let glyph = ch.encode_utf8(&mut buf);
            self.canvas.save();
            self.canvas.translate(px, py);
            self.canvas.rotate(placement.rotation);
            let _ = self.canvas.fill_text(-width / 2.0, 0.0, glyph, &paint);
            self.canvas.restore();
        }
    }

    fn draw_autofit_text(
        &mut self,
        x: f32,
        y: f32,
        box_width: f32,
        box_height: f32,
        text: &str,
        style: &TextStyle,
        mode: AutoFit,
        min_size: u16,
        max_size: u16,
    ) {
        if text.is_empty() || box_width <= 0.0 || box_height <= 0.0 {
            return;
        }
        let spans = [SpanData {
            text: text.to_string(),
            weight: None,
            color: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }];

        let (lower, upper) = autofit_bounds(
            style.size,
            u32::from(min_size),
            u32::from(max_size),
            mode,
            Some(box_height),
            style.line_height,
        );
        let fitted = search_fit_size(lower, upper, Some(box_width), Some(box_height), |size| {
            // Performance enhancement: Normalize the color, so the cache is not invalidated on repeated measurements e.g., during fade.
            let probe = TextStyle {
                size,
                color: WHITE,
                ..*style
            };
            self.paragraph_cache
                .measure(&mut self.font_system, &probe, &spans, Some(box_width))
        });

        let sized = TextStyle {
            size: fitted,
            ..*style
        };
        let (_, block_h) =
            self.paragraph_cache
                .measure(&mut self.font_system, &sized, &spans, Some(box_width));

        let draw_y = match style.vertical_align {
            VerticalAlign::Top => y,
            // Currently the Baseline behaves the same as the Center option.
            VerticalAlign::Center | VerticalAlign::Baseline => {
                y + ((box_height - block_h) / 2.0).max(0.0)
            }
            VerticalAlign::Bottom => y + (box_height - block_h).max(0.0),
        };

        self.paragraph_cache.draw(
            &mut self.font_system,
            &mut self.canvas,
            self.fonts,
            &sized,
            &spans,
            x,
            draw_y,
            box_width,
        );
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
            self.fonts,
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

    fn register_svg(&mut self, tag: &str, data: &[u8]) -> Option<SvgId> {
        self.icon_registry.register(tag, data)
    }

    fn draw_svg(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
        icon_id: SvgId,
        anti_alias: bool,
        fills: &[(String, Color)],
    ) {
        if let Some(icon) = self.icon_registry.get(icon_id) {
            super::svg::draw_svg(&mut self.canvas, icon, x, y, w, h, color, anti_alias, fills);
        }
    }

    // -- Bitmaps --

    fn register_bitmap(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId> {
        self.bitmap_registry
            .register(tag, data, &mut self.canvas, femtovg::ImageFlags::empty())
    }

    fn register_bitmap_nearest(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId> {
        self.bitmap_registry
            .register(tag, data, &mut self.canvas, femtovg::ImageFlags::NEAREST)
    }

    fn register_bitmap_rgba(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId> {
        self.bitmap_registry.register_rgba(
            tag,
            rgba,
            width,
            height,
            &mut self.canvas,
            femtovg::ImageFlags::empty(),
        )
    }

    fn draw_bitmap(&mut self, x: f32, y: f32, w: f32, h: f32, bitmap_id: BitmapId) {
        if let Some(image_id) = self.bitmap_registry.get(bitmap_id) {
            super::bitmap::draw_bitmap(&mut self.canvas, image_id, x, y, w, h);
        }
    }

    fn draw_nine_patch(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: BitmapId,
        left: u16,
        top: u16,
        right: u16,
        bottom: u16,
    ) {
        if let Some((image_id, src_w, src_h)) = self.bitmap_registry.get_with_size(bitmap_id) {
            super::bitmap::draw_nine_patch(
                &mut self.canvas,
                image_id,
                src_w as f32,
                src_h as f32,
                x,
                y,
                w,
                h,
                f32::from(left),
                f32::from(top),
                f32::from(right),
                f32::from(bottom),
            );
        }
    }

    fn register_mesh(&mut self, tag: &str, data: &[u8]) -> Option<MeshId> {
        self.lazy_init_mesh_renderer();
        let renderer = self.mesh_renderer.as_mut()?;
        renderer.register_mesh(&self.gl, tag, data)
    }

    fn reserve_mesh(&mut self, tag: &str) -> Option<MeshId> {
        if let Some(renderer) = self.mesh_renderer.as_mut() {
            renderer.reserve(tag)
        } else {
            self.pending_mesh_reservations.reserve(tag)
        }
    }

    fn suspend_mesh(&mut self, tag: &str) -> AssetSuspendResult<MeshId> {
        if let Some(renderer) = self.mesh_renderer.as_mut() {
            renderer.suspend_exact(&self.gl, tag)
        } else {
            self.pending_mesh_reservations.suspend_exact(tag)
        }
    }

    fn mesh_tag_state(&self, tag: &str) -> AssetTagState<MeshId> {
        if let Some(renderer) = self.mesh_renderer.as_ref() {
            renderer.tag_state(tag)
        } else {
            self.pending_mesh_reservations.tag_state(tag)
        }
    }

    fn draw_mesh(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        slot_index: u8,
        mesh_id: MeshId,
        args: MeshDrawArgs,
    ) {
        self.lazy_init_mesh_renderer();
        let Some(renderer) = self.mesh_renderer.as_mut() else {
            return;
        };

        // Render mesh to atlas slot (skips if params unchanged).
        // `None` means out-of-range slot or evicted mesh
        //  — skip the draw so we don't sample stale slot pixels.
        let Some((image_id, sx, sy, sw, sh)) =
            renderer.render(&self.gl, slot_index, mesh_id, &args)
        else {
            return;
        };

        // Draw the atlas sub-rect via femtovg
        let (atlas_w, atlas_h) = renderer.atlas_size();
        super::bitmap::draw_bitmap_subrect(
            &mut self.canvas,
            image_id,
            atlas_w,
            atlas_h,
            sx,
            sy,
            sw,
            sh,
            x,
            y,
            w,
            h,
        );
    }

    #[expect(clippy::many_single_char_names)]
    fn draw_sphere(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: BitmapId,
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

        // Resolve the bitmap each frame so an evict+re-register cycle is
        // observed before we touch GL. Skip the draw on registry miss so a
        // stale FBO never gets sampled.
        let Some(image_id) = self.bitmap_registry.get(bitmap_id) else {
            self.sphere_bitmap_id = None;
            return;
        };

        // Rebind the GL texture when the source bitmap changed. `BitmapId`
        // is not recycled across evict+re-register (see `BitmapRegistry`),
        // so equal ids guarantee the underlying texture is still live.
        if self.sphere_bitmap_id != Some(bitmap_id) {
            let Ok(tex) = self.canvas.get_native_texture(image_id) else {
                self.sphere_bitmap_id = None;
                return;
            };
            sphere.set_texture(tex);
            self.sphere_bitmap_id = Some(bitmap_id);
        }

        // When light is NaN, pass zero-vector to disable shading
        let (sl, sn) = if light_lat.is_nan() {
            (0.0, 0.0)
        } else {
            (light_lat, light_lon)
        };

        // Render sphere to offscreen FBO (skips if params unchanged). `None`
        // means no texture has been bound yet — skip the draw so we don't
        // sample stale FBO pixels.
        let Some(image_id) =
            sphere.render(&self.gl, center_lat, center_lon, zoom, sl, sn, atmosphere)
        else {
            return;
        };

        // Draw the FBO texture via femtovg
        super::bitmap::draw_bitmap(&mut self.canvas, image_id, x, y, w, h);
    }

    // -- Drop shadow --

    fn drop_shadow(
        &mut self,
        cx: f32,
        cy: f32,
        fbo_w: u32,
        fbo_h: u32,
        dx: f32,
        dy: f32,
        blur: f32,
        color: Color,
        inner: &mut dyn FnMut(&mut dyn Renderer),
    ) {
        if fbo_w == 0 || fbo_h == 0 {
            self.render_inner_translated(cx, cy, inner);
            return;
        }

        // Physical-pixel FBO size, so the offscreen pass matches screen DPI
        // and the composite doesn't upscale a half-resolution buffer.
        let dpi = self.dpi_scale.max(1.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "FBO dimensions are small positive pixel counts after ceil()"
        )]
        let (pw, ph) = (
            ((fbo_w as f32) * dpi).ceil() as u32,
            ((fbo_h as f32) * dpi).ceil() as u32,
        );

        let Some((unblurred, blurred)) = self.acquire_shadow_fbos(pw, ph) else {
            tracing::warn!("drop_shadow: FBO pool allocation failed; falling back to no shadow");
            self.render_inner_translated(cx, cy, inner);
            return;
        };

        // Rasterise the inner draw into `unblurred` at FBO-local coordinates.
        // `reset` drops transform + scissor; the device-pixel ratio is canvas-level
        // and survives, so logical coords still scale to physical.
        self.canvas.save();
        self.canvas.reset();
        self.canvas
            .set_render_target(RenderTarget::Image(unblurred));
        self.canvas
            .clear_rect(0, 0, pw, ph, femtovg::Color::rgbaf(0.0, 0.0, 0.0, 0.0));

        inner(self);

        // Sigma is logical pixels; scale to physical.
        // Clamp mirrors the SDK cap so a malformed
        // wire value can't request an unbounded blur.
        let sigma = blur.clamp(0.0, bmc_wasm_protocol::DROP_SHADOW_BLUR_MAX) * dpi;
        if sigma > 0.0 {
            self.canvas.filter_image(
                blurred,
                femtovg::ImageFilter::GaussianBlur { sigma },
                unblurred,
            );
        }

        // No mid-frame flush: femtovg replays its queue in submission order
        // and the target switch is an ordering barrier, so the composites
        // below read fully-rendered textures. That same ordering lets one
        // pooled pair serve every shadow in a frame — draw N's composites
        // are queued before draw N+1's `clear_rect`. A flush here,
        // or out-of-order shadow rendering, would break that.
        // Back to whatever the frame draws into rather than `Screen`.
        // Under `begin_frame_to_image` that is a stage's image,
        // and assuming the window strands everything queued after this shadow.
        self.canvas.set_render_target(self.frame_target);
        self.canvas.restore();

        let composite_src = if sigma > 0.0 { blurred } else { unblurred };
        let tint = to_femtovg_color(color.to_u32());
        let fbo_w_f = fbo_w as f32;
        let fbo_h_f = fbo_h as f32;

        // Each composite flips Y so the GL FBO's bottom-left-origin texture
        // lands right-side-up; paint and path anchor at (0, 0) in flipped space.
        self.canvas.save();
        self.canvas.translate(cx + dx, cy + dy + fbo_h_f);
        self.canvas.scale(1.0, -1.0);
        let shadow_paint = Paint::image_tint(composite_src, 0.0, 0.0, fbo_w_f, fbo_h_f, 0.0, tint);
        let mut shadow_path = Path::new();
        shadow_path.rect(0.0, 0.0, fbo_w_f, fbo_h_f);
        self.canvas.fill_path(&shadow_path, &shadow_paint);
        self.canvas.restore();

        self.canvas.save();
        self.canvas.translate(cx, cy + fbo_h_f);
        self.canvas.scale(1.0, -1.0);
        let asset_paint = Paint::image(unblurred, 0.0, 0.0, fbo_w_f, fbo_h_f, 0.0, 1.0);
        let mut asset_path = Path::new();
        asset_path.rect(0.0, 0.0, fbo_w_f, fbo_h_f);
        self.canvas.fill_path(&asset_path, &asset_paint);
        self.canvas.restore();
    }

    // -- Frame lifecycle --

    fn begin_frame(&mut self, width: u32, height: u32, dpi_scale: f32) {
        self.begin_frame_with_clear(width, height, dpi_scale, FrameClear::OpaqueBlack);
    }

    fn begin_frame_with_clear(
        &mut self,
        width: u32,
        height: u32,
        dpi_scale: f32,
        clear: FrameClear,
    ) {
        self.width = width as f32;
        self.height = height as f32;
        self.dpi_scale = dpi_scale;
        self.frame_counter += 1;
        // FemtoVG's `set_render_target(Screen)` skips the SetRenderTarget command
        // when the Canvas already thinks it's targeting Screen (the default).
        // This means flush() never calls glBindFramebuffer. We must bind the
        // target FBO explicitly so clear_rect and draw commands hit the right FBO.
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, self.screen_fbo);
        }
        self.canvas.set_render_target(RenderTarget::Screen);
        self.frame_target = RenderTarget::Screen;
        self.canvas.set_size(width, height, dpi_scale);
        // clear_rect uses physical pixels when dpi_scale > 1.0
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (pw, ph) = (
            (width as f32 * dpi_scale) as u32,
            (height as f32 * dpi_scale) as u32,
        );
        let clear = match clear {
            FrameClear::OpaqueBlack => femtovg::Color::rgbf(0.0, 0.0, 0.0),
            FrameClear::TransparentBlack => femtovg::Color::rgbaf(0.0, 0.0, 0.0, 0.0),
        };
        self.canvas.clear_rect(0, 0, pw, ph, clear);
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

    fn evict_prefix(&mut self, prefix: &str) -> usize {
        let mut n = self.icon_registry.evict_prefix(prefix);
        n += self.bitmap_registry.evict_prefix(prefix, &mut self.canvas);
        if let Some(mesh) = self.mesh_renderer.as_mut() {
            n += mesh.evict_prefix(&self.gl, prefix);
        } else {
            n += self.pending_mesh_reservations.evict_prefix(prefix);
        }
        n
    }

    fn bitmap_resident_bytes(&self) -> u64 {
        self.bitmap_registry.resident_bytes()
    }

    fn svg_resident_path_bytes(&self) -> u64 {
        self.icon_registry.resident_path_bytes()
    }

    fn mesh_resident_bytes(&self) -> u64 {
        self.mesh_renderer
            .as_ref()
            .map_or(0, MeshRenderer::resident_bytes)
    }
}

fn femtovg_baseline(vertical_align: VerticalAlign) -> femtovg::Baseline {
    match vertical_align {
        VerticalAlign::Top => femtovg::Baseline::Top,
        VerticalAlign::Center => femtovg::Baseline::Middle,
        VerticalAlign::Bottom => femtovg::Baseline::Bottom,
        VerticalAlign::Baseline => femtovg::Baseline::Alphabetic,
    }
}

/// Build a femtovg [`Paint`] for `fill`. `lin_bbox` is the bounding box used
/// for linear endpoint projection; `radial` is `(cx, cy, radius)` for the
/// radial gradient (a circle passes its own radius, not the bbox diagonal).
fn paint_for_fill(fill: &Fill, lin_bbox: (f32, f32, f32, f32), radial: (f32, f32, f32)) -> Paint {
    match fill {
        Fill::Solid(c) => Paint::color(to_femtovg_color(c.to_u32())),
        Fill::Linear { angle, start, end } => {
            let (bx, by, bw, bh) = lin_bbox;
            let ((sx, sy), (ex, ey)) = linear_endpoints(bx, by, bw, bh, *angle);
            Paint::linear_gradient(
                sx,
                sy,
                ex,
                ey,
                to_femtovg_color(start.to_u32()),
                to_femtovg_color(end.to_u32()),
            )
        }
        Fill::Radial { inner, outer } => {
            let (cx, cy, radius) = radial;
            Paint::radial_gradient(
                cx,
                cy,
                0.0,
                radius,
                to_femtovg_color(inner.to_u32()),
                to_femtovg_color(outer.to_u32()),
            )
        }
    }
}

const ARC_CHUNK_MAX: f32 = 0.4;
const ARC_SEAM_EPS: f32 = 0.002;

// Explicit segments hold absolute angular positions; clipping them to the draw
// sweep lets the sweep reveal or hide whole/partial segments without moving
// them, so a sweep transition animates the arc's length in place. The gradient
// still maps over the draw sweep, so the colour at a revealed angle converges to
// its resting value as the sweep reaches the requested end.
fn arc_spans(segments: &ArcSegments, start: f32, end: f32) -> Vec<(f32, f32)> {
    match segments {
        ArcSegments::Continuous => vec![(start, end)],
        ArcSegments::Explicit(spans) => {
            let (lo, hi) = (start.min(end), start.max(end));
            spans
                .iter()
                .filter_map(|&(s, e)| {
                    let clipped = (s.max(lo), e.min(hi));
                    (clipped.0 < clipped.1).then_some(clipped)
                })
                .collect()
        }
    }
}

fn chunk_span(a0: f32, a1: f32, max: f32) -> Vec<(f32, f32)> {
    let span = a1 - a0;
    let max = max.abs();
    if span.abs() <= max || max <= f32::EPSILON {
        return vec![(a0, a1)];
    }
    let direction = span.signum();
    let step = max * direction;
    let mut chunks = Vec::new();
    let mut s = a0;
    while ((a1 - s) * direction) > max {
        let e = s + step;
        chunks.push((s, e));
        s = e;
    }
    chunks.push((s, a1));
    chunks
}

fn sweep_fraction(theta: f32, start: f32, end: f32) -> f32 {
    let span = end - start;
    if span.abs() < f32::EPSILON {
        0.0
    } else {
        ((theta - start) / span).clamp(0.0, 1.0)
    }
}

fn arc_color_at(fill: &ArcFill, t: f32) -> Color {
    match fill {
        ArcFill::Solid(c) => *c,
        ArcFill::Gradient { start, end } => start.mix(*end, t),
    }
}

fn arc_to_femtovg_angle(angle: f32) -> f32 {
    angle - std::f32::consts::FRAC_PI_2
}

fn arc_point(cx: f32, cy: f32, radius: f32, angle: f32) -> (f32, f32) {
    let femtovg_angle = arc_to_femtovg_angle(angle);
    (
        cx + radius * femtovg_angle.cos(),
        cy + radius * femtovg_angle.sin(),
    )
}

/// Compute the two linear-gradient endpoints for a bounding box `(x, y, w, h)`
/// at `angle` degrees (`0` = top→bottom, `90` = left→right, clockwise).
///
/// The gradient axis passes through the box centre; `start` sits at the
/// minimum projection of the corners onto the angle vector and `end` at the
/// maximum, so the gradient always spans the box (CSS-style).
#[expect(
    clippy::many_single_char_names,
    reason = "x/y/w/h/t are conventional bounding-box and projection names"
)]
fn linear_endpoints(x: f32, y: f32, w: f32, h: f32, angle: f32) -> ((f32, f32), (f32, f32)) {
    let rad = angle.to_radians();
    let (dx, dy) = (rad.sin(), rad.cos());
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let corners = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)];
    let mut t_min = f32::INFINITY;
    let mut t_max = f32::NEG_INFINITY;
    for (px, py) in corners {
        let t = (px - cx) * dx + (py - cy) * dy;
        t_min = t_min.min(t);
        t_max = t_max.max(t);
    }
    (
        (cx + t_min * dx, cy + t_min * dy),
        (cx + t_max * dx, cy + t_max * dy),
    )
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

// ── Tests ──────────────────────────────────────────────────────────
#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::{FemtoVgRenderer, femtovg_baseline};
    use crate::renderer::Renderer;
    use crate::test_harness::GlHarness;
    use crate::tree::VerticalAlign;
    use glow::HasContext;
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use std::io::Cursor;

    #[test]
    fn canvas_text_vertical_align_maps_to_femtovg_baselines() {
        assert_eq!(femtovg_baseline(VerticalAlign::Top), femtovg::Baseline::Top);
        assert_eq!(
            femtovg_baseline(VerticalAlign::Center),
            femtovg::Baseline::Middle
        );
        assert_eq!(
            femtovg_baseline(VerticalAlign::Bottom),
            femtovg::Baseline::Bottom
        );
        assert_eq!(
            femtovg_baseline(VerticalAlign::Baseline),
            femtovg::Baseline::Alphabetic
        );
    }

    #[test]
    fn mesh_reservation_does_not_initialize_gpu_renderer() {
        let harness = GlHarness::new().expect("BUG: headless GL setup failed");
        let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, 0, 0) }
            .expect("BUG: renderer init failed");

        let id = renderer
            .reserve_mesh("widget:mesh")
            .expect("BUG: first mesh reservation must allocate an ID");

        assert!(
            renderer.mesh_renderer.is_none(),
            "a reservation without payload must not allocate the mesh atlas"
        );
        assert_eq!(
            renderer.mesh_tag_state("widget:mesh"),
            AssetTagState::Suspended(id)
        );
    }

    /// Encode a 1×1 RGBA PNG with the given pixel; minimum payload that
    /// rides through `BitmapRegistry::register`'s decode+upload path.
    fn one_px_png(rgba: [u8; 4]) -> Vec<u8> {
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgba(rgba));
        let mut buf = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, ImageFormat::Png)
            .expect("BUG: PNG encode should succeed");
        buf.into_inner()
    }

    /// Drain any buffered GL errors so a subsequent `gl.get_error()` reflects
    /// only the operation under test.
    fn drain_gl_errors(gl: &glow::Context) {
        loop {
            let err = unsafe { gl.get_error() };
            if err == glow::NO_ERROR {
                break;
            }
        }
    }

    /// Regression for the use-after-delete documented on MR !324: when
    /// `BitmapSlot::set` evicts+re-registers the sphere's source bitmap,
    /// the next `draw_sphere` must rebind to the fresh GL texture instead
    /// of sampling the deleted one.
    #[test]
    fn sphere_rebind_after_bitmap_evict_does_not_use_deleted_texture() {
        let harness = GlHarness::new().expect("BUG: headless GL setup failed");
        let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, 0, 0) }
            .expect("BUG: renderer init failed");

        let png_a = one_px_png([255, 0, 0, 255]);
        let png_b = one_px_png([0, 255, 0, 255]);

        let id_a = renderer
            .register_bitmap("sphere:test", &png_a)
            .expect("BUG: register A");
        drain_gl_errors(&harness.gl);

        renderer.draw_sphere(
            0.0,
            0.0,
            64.0,
            64.0,
            id_a,
            0.0,
            0.0,
            2.0,
            f32::NAN,
            f32::NAN,
            false,
        );
        let err = unsafe { harness.gl.get_error() };
        assert_eq!(
            err,
            glow::NO_ERROR,
            "first draw_sphere produced GL 0x{err:04X}"
        );
        assert_eq!(renderer.sphere_bitmap_id, Some(id_a));

        // Evict A and register B under the same tag — mirrors the
        // `BitmapSlot::set` host path that destroys A's GL texture.
        assert!(
            renderer
                .bitmap_registry
                .evict("sphere:test", &mut renderer.canvas),
            "BUG: evict A",
        );
        let id_b = renderer
            .register_bitmap("sphere:test", &png_b)
            .expect("BUG: register B");
        assert_ne!(id_a, id_b, "BUG: re-register should mint a fresh BitmapId");

        drain_gl_errors(&harness.gl);
        renderer.draw_sphere(
            0.0,
            0.0,
            64.0,
            64.0,
            id_b,
            0.0,
            0.0,
            2.0,
            f32::NAN,
            f32::NAN,
            false,
        );
        let err = unsafe { harness.gl.get_error() };
        assert_eq!(
            err,
            glow::NO_ERROR,
            "draw_sphere after evict+rebind produced GL 0x{err:04X} (use-after-delete)",
        );
        assert_eq!(
            renderer.sphere_bitmap_id,
            Some(id_b),
            "BUG: rebind tracker must follow the new BitmapId",
        );
    }

    /// `draw_sphere` with an unregistered (or already-evicted) `BitmapId`
    /// must skip the GL path entirely rather than sampling whatever
    /// texture was last bound.
    #[test]
    fn sphere_skips_draw_on_registry_miss() {
        let harness = GlHarness::new().expect("BUG: headless GL setup failed");
        let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, 0, 0) }
            .expect("BUG: renderer init failed");

        let png = one_px_png([255, 255, 255, 255]);
        let id = renderer
            .register_bitmap("sphere:gone", &png)
            .expect("BUG: register");
        assert!(
            renderer
                .bitmap_registry
                .evict("sphere:gone", &mut renderer.canvas),
            "BUG: evict",
        );

        drain_gl_errors(&harness.gl);
        renderer.draw_sphere(
            0.0,
            0.0,
            64.0,
            64.0,
            id,
            0.0,
            0.0,
            2.0,
            f32::NAN,
            f32::NAN,
            false,
        );
        let err = unsafe { harness.gl.get_error() };
        assert_eq!(
            err,
            glow::NO_ERROR,
            "draw_sphere on missing bitmap produced GL 0x{err:04X}"
        );
        assert_eq!(
            renderer.sphere_bitmap_id, None,
            "BUG: registry miss must invalidate the cached binding",
        );
    }
}

/// Regressions for hard-separated multiline text. cosmic-text reports each
/// glyph's byte offset relative to its own hard line, so a later line's offsets
/// must be shifted by that line's start before they can index the concatenated
/// span text or the span-style table — otherwise a later line slices a
/// multi-byte char boundary (panic), renders the first line's characters, or
/// inherits an earlier span's style. These exercise the real `FemtoVgRenderer`
/// GL path end to end (shape → draw → read back pixels).
#[cfg(test)]
#[cfg(target_os = "linux")]
mod multiline_text_tests {
    use super::FemtoVgRenderer;
    use crate::renderer::Renderer;
    use crate::test_harness::{GlHarness, create_readback_fbo, read_pixels_top_down};
    use crate::tree::{SpanData, TextStyle};
    use bmc_wasm_protocol::{AutoFit, Color};

    const W: u32 = 240;
    const H: u32 = 110;
    /// Text origin of every render below. The box is inset by this much on all
    /// four sides, giving `MAX_W` × `BOX_H`.
    const TEXT_INSET: f32 = 2.0;
    const MAX_W: f32 = 236.0;
    const BOX_H: f32 = 106.0;

    fn span(text: &str, color: Option<Color>) -> SpanData {
        SpanData {
            text: text.to_owned(),
            weight: None,
            color,
            italic: false,
            underline: false,
            strikethrough: false,
        }
    }

    fn white_style() -> TextStyle {
        TextStyle {
            size: 28,
            color: Color::from_rgb(255, 255, 255),
            ..TextStyle::default()
        }
    }

    /// Row bands that isolate each line's ink: rows `0..line1_max` carry only
    /// the first line, `line2_min..H` only the second.
    ///
    /// Derived from the style the tests actually draw with rather than
    /// hand-computed, so a change to `TextStyle::default().line_height` moves
    /// the bands instead of leaving every assertion below silently measuring
    /// the wrong rows. cosmic-text centres the glyph box inside the line box,
    /// so the blank gap between the two lines' ink is exactly the leading —
    /// half above the line boundary and half below.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn line_bands() -> (usize, usize) {
        let style = white_style();
        let font = style.size as f32;
        let line_height = font * style.line_height;
        let half_leading = (line_height - font) / 2.0;
        (
            (TEXT_INSET + half_leading + font) as usize,
            (TEXT_INSET + line_height + half_leading) as usize,
        )
    }

    /// Run `draw` against a fresh headless renderer targeting an offscreen FBO
    /// and return the rendered pixels (row 0 = top). Each call is a full,
    /// independent GL context — deterministic under Mesa llvmpipe.
    fn render(draw: impl FnOnce(&mut FemtoVgRenderer)) -> Vec<[u8; 4]> {
        let harness = GlHarness::new().expect("BUG: headless GL setup failed");
        let (fbo, fbo_id) = create_readback_fbo(&harness.gl, W, H);
        let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
            .expect("BUG: renderer init failed");
        renderer.begin_frame(W, H, 1.0);
        draw(&mut renderer);
        renderer.flush();
        let pixels = read_pixels_top_down(&harness.gl, fbo, W, H);
        drop(renderer);
        pixels
    }

    fn lit(px: [u8; 4]) -> bool {
        u16::from(px[0]) + u16::from(px[1]) + u16::from(px[2]) > 96
    }

    fn is_reddish(px: [u8; 4]) -> bool {
        px[0] > 150 && px[1] < 100 && px[2] < 100
    }

    fn is_greenish(px: [u8; 4]) -> bool {
        px[1] > 140 && px[0] < 120 && px[2] < 120
    }

    /// Count pixels matching `pred` within rows `[y0, y1)`.
    fn count_rows(px: &[[u8; 4]], y0: usize, y1: usize, pred: impl Fn([u8; 4]) -> bool) -> usize {
        let w = W as usize;
        px[y0 * w..y1 * w].iter().filter(|p| pred(**p)).count()
    }

    /// Both text rows must carry real glyph coverage. Guards the pixel-equality
    /// and colour-absence assertions below, which all hold trivially if a line
    /// renders nothing at all. Measured coverage is 468–1019 lit px per line, so
    /// this floor sits ~5× below the real values.
    fn assert_both_lines_rendered(px: &[[u8; 4]]) {
        const MIN_LIT: usize = 100;
        let (line1_max, line2_min) = line_bands();
        let first = count_rows(px, 0, line1_max, lit);
        let second = count_rows(px, line2_min, H as usize, lit);
        assert!(
            first > MIN_LIT,
            "BUG: first line rendered nothing ({first} lit px)",
        );
        assert!(
            second > MIN_LIT,
            "BUG: second line rendered nothing ({second} lit px)",
        );
    }

    /// Count pixels that differ between two renders within rows `[y0, y1)`.
    fn diff_rows(a: &[[u8; 4]], b: &[[u8; 4]], y0: usize, y1: usize) -> usize {
        let w = W as usize;
        a[y0 * w..y1 * w]
            .iter()
            .zip(&b[y0 * w..y1 * w])
            .filter(|(pa, pb)| pa != pb)
            .count()
    }

    /// A multi-byte grapheme near the start, followed by a hard break, once made
    /// a later line's glyph offsets slice the concatenated span text on a
    /// non-char boundary — a panic that tears down the widget slot. Both the
    /// paragraph and autofit paths must survive it.
    #[test]
    fn multibyte_hard_break_does_not_panic() {
        let style = white_style();
        let spans = [span("“Fact”\nSecond line", None)];
        let px = render(|r| r.draw_paragraph(&style, &spans, TEXT_INSET, TEXT_INSET, MAX_W));
        // Per line, not per frame: a whole-frame check passes when only the first
        // line survives, which is the very regression this guards.
        assert_both_lines_rendered(&px);

        // Autofit funnels through the same paragraph draw; must not panic either.
        let _ = render(|r| {
            r.draw_autofit_text(
                TEXT_INSET,
                TEXT_INSET,
                MAX_W,
                BOX_H,
                "“Fact”\nSecond line",
                &style,
                AutoFit::Shrink,
                8,
                28,
            );
        });
    }

    /// The second line must render its own characters, not the bytes sitting at
    /// the same offsets in the first line. Two paragraphs share an identical
    /// second line but differ on the first; with the bug the second line copies
    /// the first line's glyphs, so the bottom region diverges.
    #[test]
    fn later_line_renders_its_own_characters() {
        let style = white_style();
        let first = [span("AAAA\nZZZZ", None)];
        let second = [span("MMMM\nZZZZ", None)];
        let a = render(|r| r.draw_paragraph(&style, &first, TEXT_INSET, TEXT_INSET, MAX_W));
        let b = render(|r| r.draw_paragraph(&style, &second, TEXT_INSET, TEXT_INSET, MAX_W));

        // Sanity: both lines actually rendered, and the first lines really do
        // differ. Without this the equality check below passes vacuously if the
        // second line disappears from both renders.
        assert_both_lines_rendered(&a);
        assert_both_lines_rendered(&b);
        let (line1_max, line2_min) = line_bands();
        assert!(
            diff_rows(&a, &b, 0, line1_max) > 20,
            "BUG: test setup — first lines should differ",
        );
        // The shared second line must be pixel-identical (small AA slack).
        let diffs = diff_rows(&a, &b, line2_min, H as usize);
        assert!(
            diffs < 20,
            "BUG: second line depends on the first line's text ({diffs} px differ)",
        );
    }

    /// Span styling must follow the text across a hard break. First span is red
    /// and ends with the newline; the second span (green) owns the second line.
    /// With the bug the second line inherits the first span's red.
    #[test]
    fn span_style_survives_hard_break() {
        let style = white_style();
        let red = Color::from_rgb(255, 40, 40);
        let green = Color::from_rgb(40, 220, 40);
        let spans = [span("Red\n", Some(red)), span("Green", Some(green))];
        let px = render(|r| r.draw_paragraph(&style, &spans, TEXT_INSET, TEXT_INSET, MAX_W));

        let (line1_max, line2_min) = line_bands();
        assert!(
            count_rows(&px, 0, line1_max, is_reddish) > 10,
            "BUG: first line should be red",
        );
        assert!(
            count_rows(&px, line2_min, H as usize, is_greenish) > 10,
            "BUG: second line should render its own (green) span",
        );
        let leaked = count_rows(&px, line2_min, H as usize, is_reddish);
        assert_eq!(
            leaked, 0,
            "BUG: second line leaked the first span's red color ({leaked} px)",
        );
    }
}

#[cfg(test)]
mod gradient_geometry_tests {
    use super::linear_endpoints;

    fn approx(a: (f32, f32), b: (f32, f32)) {
        assert!(
            (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01,
            "{a:?} != {b:?}"
        );
    }

    #[test]
    fn zero_degrees_is_top_to_bottom() {
        let (start, end) = linear_endpoints(0.0, 0.0, 100.0, 100.0, 0.0);
        approx(start, (50.0, 0.0));
        approx(end, (50.0, 100.0));
    }

    #[test]
    fn ninety_degrees_is_left_to_right() {
        let (start, end) = linear_endpoints(0.0, 0.0, 100.0, 100.0, 90.0);
        approx(start, (0.0, 50.0));
        approx(end, (100.0, 50.0));
    }

    #[test]
    fn forty_five_degrees_spans_the_diagonal() {
        let (start, end) = linear_endpoints(0.0, 0.0, 100.0, 100.0, 45.0);
        approx(start, (0.0, 0.0));
        approx(end, (100.0, 100.0));
    }

    #[test]
    fn non_square_box_does_not_land_on_corners() {
        // 200x100 box at 45 deg: endpoints fall off the corners, exercising
        // the general projection path rather than the square special case.
        let (start, end) = linear_endpoints(0.0, 0.0, 200.0, 100.0, 45.0);
        approx(start, (25.0, -25.0));
        approx(end, (175.0, 125.0));
    }
}

#[cfg(test)]
mod arc_geometry_tests {
    use super::{ARC_CHUNK_MAX, arc_color_at, arc_point, arc_spans, chunk_span, sweep_fraction};
    use bmc_wasm_protocol::{ArcFill, ArcSegments, Color};

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "{a} != {b}");
    }

    #[test]
    fn continuous_resolves_to_full_sweep() {
        assert_eq!(
            arc_spans(&ArcSegments::Continuous, 0.2, 1.4),
            vec![(0.2, 1.4)]
        );
    }

    #[test]
    fn explicit_within_sweep_passes_unchanged() {
        let spans = vec![(0.0, 0.5), (0.6, 1.0)];
        assert_eq!(
            arc_spans(&ArcSegments::Explicit(spans.clone()), 0.0, 1.0),
            spans
        );
    }

    #[test]
    fn explicit_clips_to_sweep_dropping_and_truncating() {
        // Sweep ends mid-second-segment: the first survives whole, the second is
        // truncated at the sweep end, anything past it is dropped.
        let spans = vec![(0.0, 0.5), (0.6, 1.0), (1.2, 1.5)];
        assert_eq!(
            arc_spans(&ArcSegments::Explicit(spans), 0.0, 0.8),
            vec![(0.0, 0.5), (0.6, 0.8)]
        );
    }

    #[test]
    fn explicit_zero_sweep_clips_to_nothing() {
        let spans = vec![(0.0, 0.5), (0.6, 1.0)];
        assert!(arc_spans(&ArcSegments::Explicit(spans), 0.0, 0.0).is_empty());
    }

    #[test]
    fn chunking_subdivides_and_lands_on_end() {
        let chunks = chunk_span(0.0, 1.0, 0.2);
        assert_eq!(chunks.len(), 5);
        approx(chunks[0].0, 0.0);
        approx(
            chunks
                .last()
                .expect("BUG: chunk_span must return at least one chunk")
                .1,
            1.0,
        );
        for w in chunks.windows(2) {
            approx(w[0].1, w[1].0);
        }
    }

    #[test]
    fn tiny_span_yields_one_chunk() {
        assert_eq!(chunk_span(0.0, 0.05, 0.2).len(), 1);
    }

    #[test]
    fn mining_clock_tick_span_fits_in_one_arc_chunk() {
        let mining_tick_slot = std::f32::consts::TAU / 28.0;
        let mining_tick_span = mining_tick_slot * 0.96;

        assert_eq!(chunk_span(0.0, mining_tick_span, ARC_CHUNK_MAX).len(), 1);
    }

    #[test]
    fn sweep_fraction_is_independent_of_gaps() {
        approx(sweep_fraction(0.0, 0.0, 1.0), 0.0);
        approx(sweep_fraction(1.0, 0.0, 1.0), 1.0);
        approx(sweep_fraction(0.5, 0.0, 1.0), 0.5);
        approx(sweep_fraction(0.75, 0.0, 1.0), 0.75);
    }

    #[test]
    fn color_at_hits_endpoints() {
        let red = Color::from_rgb(0xFF, 0x00, 0x00);
        let blue = Color::from_rgb(0x00, 0x00, 0xFF);
        let g = ArcFill::gradient(red, blue);
        assert_eq!(arc_color_at(&g, 0.0), red);
        assert_eq!(arc_color_at(&g, 1.0), blue);
        assert_eq!(arc_color_at(&ArcFill::Solid(red), 0.5), red);
    }

    #[test]
    fn arc_point_uses_twelve_oclock_zero() {
        let center = (10.0, 20.0);
        let radius = 5.0;
        let top = arc_point(center.0, center.1, radius, 0.0);
        approx(top.0, 10.0);
        approx(top.1, 15.0);

        let right = arc_point(center.0, center.1, radius, std::f32::consts::FRAC_PI_2);
        approx(right.0, 15.0);
        approx(right.1, 20.0);
    }
}
