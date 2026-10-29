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

use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroU32;

use anyhow::Result;
use bmc_ui::colors::Color;
use bmc_ui::{ArcCap, ArcFill, ArcSegments, BitmapId, Fill};
use femtovg::renderer::OpenGl;
use femtovg::{Canvas, LineCap, Paint, Path, RenderTarget, Solidity};
use glow::HasContext;

use super::bitmap::BitmapRegistry;
use super::glyph_cache::GlyphCache;
use super::mesh::{MESH_ID_EXCLUSIVE_CAP, MeshRenderer, PendingMeshReservations};
use super::sphere::SphereRenderer;
use super::svg::SvgRegistry;
use super::text::{FontTable, ParagraphLayoutCache, build_font_table, to_femtovg_color};
use crate::components::draw::is_full_turn;
use crate::renderer::prelude::*;
use crate::renderer::{FrameBackground, FrameClear};

mod assets;
mod fonts;
mod geometry;
mod render_targets;
pub(crate) use fonts::{FACES, Fonts, build_font_system};
use geometry::{
    ARC_CHUNK_MAX, ARC_SEAM_EPS, arc_color_at, arc_point, arc_spans, arc_to_femtovg_angle,
    build_femtovg_path, chunk_span, paint_for_fill, sweep_fraction,
};
pub use render_targets::RenderTargetProbe;
use render_targets::{PhysicalSize, PoolSlot, RawBlit, ShadowFboPool, StaticLayer};

#[cfg(any(test, feature = "test-support"))]
pub(crate) use text_draw::sans_line_style;

mod text_draw;

#[derive(Default)]
enum SphereRendererState {
    #[default]
    Uninitialized,
    Ready(SphereRenderer),
    Failed,
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
    font_system: cosmic_text::FontSystem,
    paragraph_cache: ParagraphLayoutCache,
    /// The application-owned glyph atlas every text draw goes through,
    /// and the rasterizer filling it. `font_table` maps cosmic-text's
    /// per-glyph face choice onto femtovg's handle for the same binary.
    glyph_cache: GlyphCache<femtovg::ImageId>,
    swash: cosmic_text::SwashCache,
    font_table: FontTable,
    icon_registry: SvgRegistry,
    bitmap_registry: BitmapRegistry,
    sphere: SphereRendererState,
    /// `BitmapId` currently bound as the sphere's source texture. Used to
    /// detect rebind-on-change in `draw_sphere`; a mismatch (incl. the
    /// post-evict case where the registry has dropped the id) re-fetches
    /// the native texture so we never sample a deleted GL name.
    sphere_bitmap_id: Option<BitmapId>,
    mesh_renderer: Option<MeshRenderer>,
    pending_mesh_reservations: PendingMeshReservations,
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
    /// The surface the frame in progress paints on; see [`FrameBackground`].
    background: FrameBackground,
    shadow_fbo_pool: Option<ShadowFboPool>,
    #[cfg(feature = "profiling")]
    glyph_report_every: ii_stopwatch::Every,
    /// Cached static layers, keyed by owning widget instance. One renderer
    /// serves every slot, so these cannot be a single slot-agnostic layer.
    static_layers: HashMap<String, StaticLayer>,
    /// Stencil renderbuffers shared by the static layers of one physical size,
    /// standing in for the one femtovg allocates per render-target image.
    ///
    /// Keyed by size because GLES 2.0 wants every attachment on a framebuffer to share dimensions.
    /// Entries live as long as the renderer,
    /// bounded by the handful of layer sizes a layout produces.
    stencil_pool: HashMap<PhysicalSize, PoolSlot>,
    /// Layers whose framebuffer came back incomplete, and the size it failed at.
    /// `begin_static_layer` refuses a layer at that size but not at any other.
    /// Kept apart from `static_layers`, since the refusal drops the layer's entry there.
    incomplete_static_layers: HashMap<String, PhysicalSize>,
    /// Probed in [`FemtoVgRenderer::new`].
    /// Anything but [`RenderTargetProbe::Working`] makes `begin_static_layer` refuse,
    /// so a driver that cannot render into an RGBA8 texture keeps drawing
    /// full frames instead of blitting a layer it never captured.
    render_to_texture: RenderTargetProbe,
    /// 1x1 opaque white texture, tinted to paint solid rectangles through
    /// femtovg's texture-copy fast path. See [`FemtoVgRenderer::solid_texture`].
    solid_texture: Option<femtovg::ImageId>,
    /// See [`RenderFrame::brightness`]. Applied as each paint is built,
    /// so bitmaps and an SVG's own colours dim with everything else.
    brightness: f32,
    /// Minimal textured-quad program used to composite the static layer
    /// without going through femtovg. See [`FemtoVgRenderer::raw_blit`].
    raw_blit: Option<RawBlit>,
    /// Fails every raw layer copy, as a driver that cannot build the program does.
    #[cfg(test)]
    refuse_raw_blit: bool,
}

impl std::fmt::Debug for FemtoVgRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FemtoVgRenderer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Re-export femtovg's `ImageId` for use by gallery stage rendering.
pub type FemtovgImageId = femtovg::ImageId;

impl FemtoVgRenderer {
    pub fn enable_text_layout_profiling(&mut self) {
        self.paragraph_cache.enable_profiling();
    }

    /// How many textures femtovg's own glyph atlas holds.
    ///
    /// The glyph cache exists so this stays zero: every entry point that could
    /// populate that atlas is either unreachable or above the direct-path
    /// cutoff, where femtovg path-renders instead of atlasing.
    #[cfg(feature = "atlas-inspect")]
    #[must_use]
    pub fn font_atlas_texture_count(&self) -> usize {
        self.canvas.debug_inspector_get_font_textures().len()
    }

    /// The atlas page behind every cached-glyph batch of the last flushed frame,
    /// in submission order.
    ///
    /// One entry per batch, not per page: a page that reappears after another
    /// is a genuine second visit, and under an order-sensitive composite
    /// the pixels depend on that order surviving.
    #[cfg(feature = "atlas-inspect")]
    #[must_use]
    pub fn pages_touched_last_frame(&self) -> &[femtovg::ImageId] {
        self.glyph_cache.pages_drawn_last_frame()
    }

    /// Batches the frame submitted beyond what the record holds.
    /// Anything but zero means the list above stops partway through the frame.
    #[cfg(feature = "atlas-inspect")]
    #[must_use]
    pub fn batches_past_record_last_frame(&self) -> usize {
        self.glyph_cache.batches_over_cap_last_frame()
    }

    /// Rasterize every glyph of every loaded face at `size_px`
    /// into this renderer's own cache.
    ///
    /// Shaped text can never reach an unencoded or alternate glyph,
    /// and the draw entry points pin one family and weight,
    /// so the allocation gate cannot build its pressure through them.
    /// It enumerates the corpus here, inside the renderer,
    /// because a `fontdb::ID` only resolves against the database it came from —
    /// keys built from a second `FontSystem` would name faces
    /// this cache never sees.
    #[cfg(feature = "glyph-alloc-gate")]
    pub fn cache_full_glyph_corpus_for_gate(&mut self, size_px: f32) {
        // Two snapshots: `FaceInfo` carries no glyph count, and asking for one
        // needs `&mut FontSystem` while `db().faces()` still borrows it.
        let faces: Vec<(cosmic_text::fontdb::ID, cosmic_text::fontdb::Weight)> = self
            .font_system
            .db()
            .faces()
            .map(|face| (face.id, face.weight))
            .collect();
        let counted: Vec<(cosmic_text::fontdb::ID, cosmic_text::fontdb::Weight, u16)> = faces
            .into_iter()
            .filter_map(|(id, weight)| {
                let font = self.font_system.get_font(id, weight)?;
                Some((id, weight, font.as_swash().glyph_metrics(&[]).glyph_count()))
            })
            .collect();

        let mut pages = super::text::FemtovgPages {
            canvas: &mut self.canvas,
        };
        for (id, weight, glyph_count) in counted {
            for glyph_id in 0..glyph_count {
                let key = cosmic_text::CacheKey {
                    font_id: id,
                    glyph_id,
                    font_size_bits: size_px.to_bits(),
                    x_bin: cosmic_text::SubpixelBin::Zero,
                    y_bin: cosmic_text::SubpixelBin::Zero,
                    font_weight: weight,
                    flags: cosmic_text::CacheKeyFlags::empty(),
                };
                let _ = self.glyph_cache.get_or_insert(&mut pages, key, |key| {
                    super::text::rasterize_glyph(
                        &mut self.swash,
                        &mut self.font_system,
                        key.inner(),
                    )
                });
            }
        }
    }

    /// The cache the allocation gate inspects after driving a workload.
    #[cfg(feature = "glyph-alloc-gate")]
    pub(crate) fn glyph_cache_for_gate(&self) -> &super::glyph_cache::GlyphCache<femtovg::ImageId> {
        &self.glyph_cache
    }

    /// One aggregated line per interval, never one per pass:
    /// `flush` ends a render pass,
    /// of which a single displayed frame can have several.
    #[cfg(feature = "profiling")]
    fn report_text_profile(&mut self) {
        if !ii_stopwatch::every_expired!(self.glyph_report_every) {
            return;
        }
        let glyphs = *self.glyph_cache.counters();
        let layout = self.paragraph_cache.counters();
        tracing::info!(
            target: crate::profile::TARGET,
            hits = glyphs.hits,
            misses = glyphs.misses,
            negative_cache_hits = glyphs.negative_cache_hits,
            rasterizations = glyphs.rasterizations,
            uploads = glyphs.uploads,
            evictions = glyphs.evictions,
            max_evictions_per_miss = glyphs.max_evictions_per_miss,
            scratch_uses = glyphs.scratch_uses,
            glyphs_dropped = glyphs.glyphs_dropped,
            glyphs_oversized = glyphs.glyphs_oversized,
            cache_invariant_failures = glyphs.cache_invariant_failures,
            page_create_failures = glyphs.page_create_failures,
            upload_transient_failures = glyphs.upload_transient_failures,
            atlas_bytes = self.glyph_cache.resident_atlas_bytes(),
            metadata_capacity_bytes = self.glyph_cache.metadata_capacity_bytes(),
            layout_cache_hits = layout.layout_cache_hits,
            layout_cache_shapes = layout.layout_cache_shapes,
            layout_cache_capacity_evictions = layout.layout_cache_capacity_evictions,
            layout_cache_peak_entries = layout.layout_cache_peak_entries,
            layout_cache_peak_frame_keys = layout.layout_cache_peak_frame_keys,
            layout_cache_repeat_shapes_same_frame = layout.layout_cache_repeat_shapes_same_frame,
            layout_cache_draw_misses_after_measure = layout.layout_cache_draw_misses_after_measure,
            layout_cache_single_line_hits = layout.layout_cache_single_line_hits,
            layout_cache_single_line_shapes = layout.layout_cache_single_line_shapes,
            layout_cache_paragraph_hits = layout.layout_cache_paragraph_hits,
            layout_cache_paragraph_shapes = layout.layout_cache_paragraph_shapes,
            layout_cache_single_line_entries = layout.layout_cache_single_line_entries,
            layout_cache_single_line_peak_entries = layout.layout_cache_single_line_peak_entries,
            layout_cache_paragraph_entries = layout.layout_cache_paragraph_entries,
            layout_cache_paragraph_peak_entries = layout.layout_cache_paragraph_peak_entries,
            layout_cache_resident_glyphs = layout.layout_cache_resident_glyphs,
            layout_cache_peak_resident_glyphs = layout.layout_cache_peak_resident_glyphs,
            layout_cache_single_line_resident_glyphs = layout.layout_cache_single_line_resident_glyphs,
            layout_cache_single_line_peak_resident_glyphs = layout.layout_cache_single_line_peak_resident_glyphs,
            layout_cache_paragraph_resident_glyphs = layout.layout_cache_paragraph_resident_glyphs,
            layout_cache_paragraph_peak_resident_glyphs = layout.layout_cache_paragraph_peak_resident_glyphs,
            layout_cache_peak_frame_glyph_instances = layout.layout_cache_peak_frame_glyph_instances,
            layout_cache_peak_frame_distinct_glyphs = layout.layout_cache_peak_frame_distinct_glyphs,
            "glyph cache"
        );
    }

    /// Release a femtovg image this renderer handed out
    /// through [`Self::create_render_target`].
    pub fn delete_image(&mut self, id: femtovg::ImageId) {
        self.canvas.delete_image(id);
    }

    /// Put the canvas back into logical coordinates for the frame ahead.
    ///
    /// The reset comes first: the transform survives a frame in canvas state,
    /// so premultiplying onto what the last frame left would square the ratio
    /// every frame.
    fn scale_to_logical(&mut self, dpi_scale: f32) {
        self.canvas.reset_transform();
        self.canvas.scale(dpi_scale, dpi_scale);
    }

    /// Discard errors an earlier call left queued, so the next check reports
    /// only its own. Bounded: a lost context returns `GL_CONTEXT_LOST` forever,
    /// so an unbounded drain would hang startup instead of failing it.
    unsafe fn drain_gl_errors(&self) {
        const MAX_DRAIN: usize = 8;
        for _ in 0..MAX_DRAIN {
            let gl_error = unsafe { self.gl.get_error() };
            if gl_error == glow::NO_ERROR {
                return;
            }
            tracing::debug!(
                gl_error,
                "discarding a GL error an earlier call left queued"
            );
        }
    }

    /// Create a new GPU renderer targeting a specific FBO.
    ///
    /// The `fbo_id` is the OpenGL framebuffer object that FemtoVG should render to.
    /// This is typically the staging FBO from the EGL two-FBO pipeline.
    ///
    /// Maps the font faces once and hands the mappings to both FemtoVG
    /// (for glyphs above the cutoff) and cosmic-text (for shaping and the glyph cache).
    /// The cosmic-text `FontSystem` holds only those faces — no system font discovery.
    ///
    /// # Errors
    ///
    /// Fails when GL setup fails, or when a font face cannot be mapped, holds another face
    /// than its file is named for, or is refused by femtovg; a font error names the file.
    ///
    /// # Safety
    /// `load_fn` must return valid OpenGL function pointers for the current GL context.
    /// If `fbo_id`'s framebuffer carries a stencil renderbuffer, it must stay alive
    /// for as long as the renderer captures static layers, which borrow it.
    #[expect(clippy::cast_precision_loss)]
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

        let fonts = Fonts::load()?;
        let text_context = femtovg::TextContext::default();
        let femtovg_fonts = fonts.register_femtovg(&text_context)?;
        let mut canvas = Canvas::new_with_text_context(gl_renderer, text_context)?;
        canvas.set_size(width, height, 1.0);

        let font_system = build_font_system(&fonts);
        let font_table = build_font_table(&font_system, &femtovg_fonts);

        let mut icon_registry = SvgRegistry::new();
        icon_registry.register_builtins();

        let mut renderer = Self {
            gl,
            canvas,
            screen_fbo,
            font_system,
            paragraph_cache: ParagraphLayoutCache::new(),
            glyph_cache: GlyphCache::new(),
            swash: cosmic_text::SwashCache::new(),
            font_table,
            icon_registry,
            bitmap_registry: BitmapRegistry::new(),
            sphere: SphereRendererState::Uninitialized,
            sphere_bitmap_id: None,
            mesh_renderer: None,
            pending_mesh_reservations: PendingMeshReservations::new(MESH_ID_EXCLUSIVE_CAP),
            mesh_msaa_samples,
            width: width as f32,
            height: height as f32,
            dpi_scale: 1.0,
            frame_target: RenderTarget::Screen,
            shadow_fbo_pool: None,
            #[cfg(feature = "profiling")]
            glyph_report_every: ii_stopwatch::Every::new(std::time::Duration::from_secs(5)),
            static_layers: HashMap::new(),
            stencil_pool: HashMap::new(),
            incomplete_static_layers: HashMap::new(),
            render_to_texture: RenderTargetProbe::Working,
            solid_texture: None,
            brightness: 1.0,
            raw_blit: None,
            background: FrameBackground::OpaqueBlack,
            #[cfg(test)]
            refuse_raw_blit: false,
        };
        renderer.render_to_texture = renderer.probe_render_to_texture();
        renderer.adopt_screen_stencil();
        Ok(renderer)
    }

    /// Result of the startup render-target probe.
    pub fn render_to_texture(&self) -> RenderTargetProbe {
        self.render_to_texture
    }

    /// Drop every caller-registered bitmap and icon plus the lazy-init sphere
    /// and mesh renderers, returning the renderer to its post-`new` state.
    /// Shaders, fonts, the screen FBO binding, and the paragraph layout cache
    /// are preserved.
    pub fn drop_all(&mut self) {
        self.release_gpu_assets();
        self.pending_mesh_reservations.clear();
        self.icon_registry = SvgRegistry::new();
        self.icon_registry.register_builtins();
    }

    fn release_gpu_assets(&mut self) {
        self.sphere_bitmap_id = None;
        if let SphereRendererState::Ready(sphere) = std::mem::take(&mut self.sphere) {
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
        // Layer framebuffers go first, so no pooled stencil is deleted while still attached.
        for (_, layer) in self.static_layers.drain() {
            self.canvas.delete_image(layer.image);
        }
        self.release_pooled_stencils();
    }
}

// ── RenderFrame and RenderShapes implementations ────────────────────

impl FemtoVgRenderer {
    /// Paint that fills `(x, y, w, h)` with `color` through femtovg's
    /// texture-copy fast path rather than its general fill shader.
    ///
    /// A plain colour fill runs `scissorMask()`, the edge-AA `strokeMask()` and
    /// a `discard` per fragment, the discard being especially unkind to a
    /// tile-based GPU. Tinting a 1x1 texture instead selects
    /// `ShaderType::TextureCopyUnclipped`, which samples once and returns.
    /// `is_straight_tinted_image` also requires anti-aliasing off, hence both.
    ///
    /// ⚠ **This gives up anti-aliasing, and femtovg's fallback does not give it
    /// back.** AA there is geometry: in `fill_path_internal` (femtovg 0.20.4,
    /// `src/lib.rs:875`) the fringe ribbon the shader samples is emitted from
    /// the AA flag *before* the fast-path test, so a path that then declines
    /// the fast path is filled with no fringe to sample. So a rotated rect —
    /// which fails `path_fill_is_rect`, testing axis-alignment on transformed
    /// vertices — renders with stepped edges, and an axis-aligned rect at a
    /// fractional position takes the fast path with binary coverage and steps
    /// rather than slides as it animates.
    ///
    /// Gating on an axis-aligned transform would close the first; the second
    /// needs whole-device-pixel placement too.
    fn solid_paint(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) -> Paint {
        let tint = self.paint_color(color);
        let Some(texture) = self.solid_texture() else {
            return Paint::color(tint);
        };
        Paint::image_tint(texture, x, y, w, h, 0.0, tint).with_anti_alias(false)
    }

    fn dim(&self, color: Color) -> Color {
        color.brightness(self.brightness)
    }

    fn paint_color(&self, color: Color) -> femtovg::Color {
        to_femtovg_color(self.dim(color).to_u32())
    }

    /// What an image paint multiplies each texel by.
    fn image_tint(&self) -> femtovg::Color {
        femtovg::Color::rgbf(self.brightness, self.brightness, self.brightness)
    }

    /// The shared 1x1 white texture, allocated on first use.
    ///
    /// `NEAREST` because there is nothing to interpolate between. Returns
    /// `None` if allocation fails, leaving the caller to paint the usual way.
    fn solid_texture(&mut self) -> Option<femtovg::ImageId> {
        if self.solid_texture.is_none() {
            let white = [femtovg::rgb::RGBA8::new(255, 255, 255, 255)];
            let source = imgref::ImgRef::new(&white, 1, 1);
            let image = self
                .canvas
                .create_image(
                    femtovg::ImageSource::from(source),
                    femtovg::ImageFlags::NEAREST,
                )
                .ok()?;
            self.solid_texture = Some(image);
        }
        self.solid_texture
    }
}

impl RenderFrame for FemtoVgRenderer {
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

    // -- Brightness --

    fn brightness(&self) -> f32 {
        self.brightness
    }

    fn set_brightness(&mut self, brightness: f32) {
        debug_assert!(
            (0.0..=1.0).contains(&brightness),
            "BUG: brightness only dims, got {brightness}"
        );
        self.brightness = brightness;
    }

    // -- Frame lifecycle --

    fn begin_frame(&mut self, width: u32, height: u32, dpi_scale: f32) {
        self.begin_frame_with_clear(
            width,
            height,
            dpi_scale,
            FrameBackground::OpaqueBlack,
            FrameClear::Clear,
        );
    }

    #[expect(clippy::cast_precision_loss)]
    fn begin_frame_with_clear(
        &mut self,
        width: u32,
        height: u32,
        dpi_scale: f32,
        background: FrameBackground,
        clear: FrameClear,
    ) {
        debug_assert_fringe_within_a_pixel(dpi_scale);
        self.width = width as f32;
        self.height = height as f32;
        self.dpi_scale = dpi_scale;
        self.background = background;
        // FemtoVG's `set_render_target(Screen)` skips the SetRenderTarget command
        // when the Canvas already thinks it's targeting Screen (the default).
        // This means flush() never calls glBindFramebuffer. We must bind the
        // target FBO explicitly so clear_rect and draw commands hit the right FBO.
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, self.screen_fbo);
        }
        self.canvas.set_render_target(RenderTarget::Screen);
        self.frame_target = RenderTarget::Screen;
        // Physical pixels: femtovg sizes its viewport in them, and takes `dpi`
        // only as a tessellation hint.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (pw, ph) = (
            (width as f32 * dpi_scale) as u32,
            (height as f32 * dpi_scale) as u32,
        );
        self.canvas.set_size(pw, ph, dpi_scale);
        match clear {
            FrameClear::Clear => self.clear_frame(),
            // Keeping the target's pixels is the point of a damage-tracked
            // frame; clearing would wipe the regions it deliberately reuses.
            FrameClear::Keep => {}
        }
        self.scale_to_logical(dpi_scale);
        self.paragraph_cache.begin_frame();
        self.brightness = 1.0;
    }

    fn clear_frame(&mut self) {
        // `clear_rect` is untransformed, so it takes the target's physical size.
        let (pw, ph) = (self.canvas.width(), self.canvas.height());
        self.canvas
            .clear_rect(0, 0, pw, ph, background_color(self.background));
    }

    fn flush(&mut self) {
        self.canvas.flush();
        // Paired with the frame's submissions, not with `begin_frame`:
        // the scratch page's allocations are only free once they have been drawn.
        self.glyph_cache.end_frame();
        // Swapped rather than cleared:
        // the record describes the pixels this flush has just produced,
        // so it has to outlive the frame that wrote it.
        #[cfg(feature = "atlas-inspect")]
        self.glyph_cache.retire_pages_drawn();
        #[cfg(feature = "profiling")]
        self.report_text_profile();
    }

    fn flush_and_fence_us(&mut self) -> u32 {
        let started = std::time::Instant::now();
        self.canvas.flush();
        // `glFinish` rather than a fence object: this path exists to stall, and
        // it needs no extension probing on a GLES 2.0 driver.
        unsafe {
            self.gl.finish();
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a single pass never takes an hour"
        )]
        let elapsed = started.elapsed().as_micros() as u32;
        elapsed
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn height(&self) -> f32 {
        self.height
    }
}

impl RenderShapes for FemtoVgRenderer {
    // -- Shapes --

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let mut path = Path::new();
        path.rect(x, y, w, h);
        let paint = self.solid_paint(x, y, w, h, color);
        self.canvas.fill_path(&path, &paint);
    }

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let mut path = Path::new();
        path.rounded_rect(x, y, w, h, radius);
        let paint = Paint::color(self.paint_color(color));
        self.canvas.fill_path(&path, &paint);
    }

    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        let mut path = Path::new();
        path.circle(cx, cy, r);
        let paint = Paint::color(self.paint_color(color));
        self.canvas.fill_path(&path, &paint);
    }

    fn fill_rect_paint(&mut self, x: f32, y: f32, w: f32, h: f32, fill: &Fill) {
        let mut path = Path::new();
        path.rect(x, y, w, h);
        // A solid fill is the same job as `fill_rect`, so it takes the same
        // fast path; gradients have to run the general shader.
        let paint = match fill {
            Fill::Solid(color) => self.solid_paint(x, y, w, h, *color),
            Fill::Linear { .. } | Fill::Radial { .. } => paint_for_fill(
                &fill.brightness(self.brightness),
                (x, y, w, h),
                (x + w / 2.0, y + h / 2.0, (w / 2.0).hypot(h / 2.0)),
            ),
        };
        self.canvas.fill_path(&path, &paint);
    }

    fn fill_circle_paint(&mut self, cx: f32, cy: f32, r: f32, fill: &Fill) {
        let mut path = Path::new();
        path.circle(cx, cy, r);
        let paint = paint_for_fill(
            &fill.brightness(self.brightness),
            (cx - r, cy - r, 2.0 * r, 2.0 * r),
            (cx, cy, r),
        );
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
        // The chunking below exists only to run a gradient along the sweep,
        // and two anti-aliased butt ends never add up to full coverage,
        // so every chunk seam of a solid sweep would be a hairline gap.
        if let (ArcFill::Solid(color), ArcSegments::Continuous) = (fill, segments) {
            let mut path = Path::new();
            let mut paint = Paint::color(self.paint_color(*color));
            paint.set_line_width(width);
            if is_full_turn(start_angle, end_angle) {
                // Closed, so the ends join rather than meet as two caps.
                path.circle(cx, cy, radius);
            } else {
                path.arc(
                    cx,
                    cy,
                    radius,
                    arc_to_femtovg_angle(start_angle),
                    arc_to_femtovg_angle(end_angle),
                    Solidity::Hole,
                );
                paint.set_line_cap(outer_cap);
            }
            self.canvas.stroke_path(&path, &paint);
            return;
        }
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
                    self.paint_color(c0),
                    self.paint_color(c1),
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
        let mut paint = Paint::color(self.paint_color(color));
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
        let mut paint = Paint::color(self.paint_color(color));
        paint.set_line_width(border_width);
        self.canvas.stroke_path(&path, &paint);
    }

    fn draw_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color) {
        let mut path = Path::new();
        path.move_to(x1, y1);
        path.line_to(x2, y2);
        let mut paint = Paint::color(self.paint_color(color));
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
        let mut paint = Paint::color(self.paint_color(color));
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
            &fill.brightness(self.brightness),
            (min_x, min_y, w, h),
            (min_x + w / 2.0, min_y + h / 2.0, radius),
        );
        self.canvas.fill_path(&path, &paint);
    }
}

fn background_color(background: FrameBackground) -> femtovg::Color {
    match background {
        FrameBackground::OpaqueBlack => femtovg::Color::rgbf(0.0, 0.0, 0.0),
        FrameBackground::TransparentBlack => femtovg::Color::rgbaf(0.0, 0.0, 0.0, 0.0),
    }
}

/// Damage bounds femtovg's fringe, `1 / dpi` physical pixels, at one logical pixel.
fn debug_assert_fringe_within_a_pixel(dpi_scale: f32) {
    debug_assert!(
        dpi_scale >= 1.0,
        "BUG: damage bounds femtovg's fringe at one logical pixel, which needs a dpi scale of at least 1, got {dpi_scale}"
    );
}

#[cfg(test)]
mod tests;
