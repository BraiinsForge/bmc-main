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

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroU32;

use anyhow::Result;
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{
    ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, BitmapId, Fill, MeshId, SvgId, WHITE,
};
use femtovg::renderer::OpenGl;
use femtovg::{Canvas, LineCap, Paint, Path, RenderTarget, Solidity};
use glow::HasContext;

use super::bitmap::BitmapRegistry;
use super::curved_text::arc_glyph_layout;
use super::glyph_cache::GlyphCache;
use super::mesh::{MeshDrawArgs, MeshRenderer, MeshReservations};
use super::sphere::SphereRenderer;
use super::svg::SvgRegistry;
use super::text::{
    DIRECT_PATH_CUTOFF_PX, FemtovgPages, FontTable, LineGlyphs, LineStyle, ParagraphLayoutCache,
    autofit_bounds, baseline_to_alphabetic, build_cached_curved_glyph_commands, build_font_table,
    build_glyph_commands, curved_glyph_origin_x, draw_line_glyphs, outline_glyph_commands,
    search_fit_size, submit_glyph_commands, to_femtovg_color,
};
use crate::renderer::{
    AssetSuspendResult, AssetTagState, FrameClear, GlyphCacheCounters, Renderer, TextLayoutCounters,
};
use crate::tree::{
    AutoFit, FontFamily, FontWeight, SpanData, TextAlign, TextOverflow, TextStyle, VerticalAlign,
};

use crate::interaction::Rect;

mod fonts;
mod geometry;
mod render_targets;

pub(crate) use fonts::build_font_system;
use fonts::{
    FONT_BOLD, FONT_DECK_BOLD, FONT_DECK_REGULAR, FONT_DECK_SEMIBOLD, FONT_FALLBACK, FONT_REGULAR,
    FONT_SEMIBOLD,
};
use geometry::{
    ARC_CHUNK_MAX, ARC_SEAM_EPS, arc_color_at, arc_point, arc_spans, arc_to_femtovg_angle,
    build_femtovg_path, chunk_span, paint_for_fill, scissor_box, sweep_fraction,
};
pub use render_targets::RenderTargetProbe;
use render_targets::{
    F32_BYTES, PhysicalSize, PoolSlot, RawBlit, ShadowFboPool, StaticLayer, physical_size,
};

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
    /// See [`Renderer::brightness`]. Applied as each paint is built,
    /// so bitmaps and an SVG's own colours dim with everything else.
    brightness: f32,
    /// Minimal textured-quad program used to composite the static layer
    /// without going through femtovg. See [`FemtoVgRenderer::raw_blit`].
    raw_blit: Option<RawBlit>,
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
    /// If `fbo_id`'s framebuffer carries a stencil renderbuffer, it must stay alive
    /// for as long as the renderer captures static layers, which borrow it.
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

        // Load fonts into FemtoVG for the delegated path above the cutoff,
        // in the order `build_font_table` pairs them with cosmic-text's faces.
        let femtovg_fonts = [
            canvas.add_font_mem(FONT_REGULAR)?,
            canvas.add_font_mem(FONT_SEMIBOLD)?,
            canvas.add_font_mem(FONT_BOLD)?,
            canvas.add_font_mem(FONT_DECK_REGULAR)?,
            canvas.add_font_mem(FONT_DECK_SEMIBOLD)?,
            canvas.add_font_mem(FONT_DECK_BOLD)?,
            canvas.add_font_mem(FONT_FALLBACK)?,
        ];

        let font_system = build_font_system();
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
            pending_mesh_reservations: MeshReservations::default(),
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
        };
        renderer.render_to_texture = renderer.probe_render_to_texture();
        renderer.adopt_screen_stencil();
        Ok(renderer)
    }

    /// Result of the startup render-target probe.
    pub fn render_to_texture(&self) -> RenderTargetProbe {
        self.render_to_texture
    }

    #[cfg(test)]
    fn layout_line(&mut self, style: LineStyle, text: &str) -> (Vec<LineGlyphs>, f32) {
        let entry = self
            .paragraph_cache
            .layout_single_line(&mut self.font_system, style, text);
        (entry.lines.clone(), entry.width)
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
        self.pending_mesh_reservations = MeshReservations::default();
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

/// Draw shaped lines anchored at `(x, y)` by `baseline`.
///
/// The anchor resolves against each line's own metrics, and the first line's
/// layout advance is removed so a lone line lands exactly on the anchor.
#[expect(clippy::too_many_arguments, reason = "one line's full draw state")]
fn draw_anchored_lines(
    canvas: &mut Canvas<OpenGl>,
    glyph_cache: &mut GlyphCache<femtovg::ImageId>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut cosmic_text::FontSystem,
    font_table: &FontTable,
    lines: &[LineGlyphs],
    x: f32,
    y: f32,
    baseline: femtovg::Baseline,
    paint: &Paint,
    font_size: f32,
) {
    let Some(first) = lines.first() else {
        return;
    };
    let first_advance = first.baseline_y;
    for line in lines {
        let alphabetic_y = baseline_to_alphabetic(y, baseline, line.max_ascent, line.max_descent)
            + line.baseline_y
            - first_advance;
        draw_line_glyphs(
            canvas,
            glyph_cache,
            swash,
            font_system,
            font_table,
            &line.glyphs,
            x,
            alphabetic_y,
            paint,
            font_size,
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one outlined line's full draw state"
)]
fn draw_anchored_lines_with_outline(
    canvas: &mut Canvas<OpenGl>,
    glyph_cache: &mut GlyphCache<femtovg::ImageId>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut cosmic_text::FontSystem,
    font_table: &FontTable,
    lines: &[LineGlyphs],
    x: f32,
    y: f32,
    baseline: femtovg::Baseline,
    outline_paint: &Paint,
    paint: &Paint,
    font_size: f32,
    rings: u32,
) {
    if font_size > DIRECT_PATH_CUTOFF_PX {
        for ring in 1..=rings {
            let d = ring as f32;
            for (dx, dy) in [
                (d, 0.0),
                (-d, 0.0),
                (0.0, d),
                (0.0, -d),
                (d, d),
                (-d, -d),
                (d, -d),
                (-d, d),
            ] {
                draw_anchored_lines(
                    canvas,
                    glyph_cache,
                    swash,
                    font_system,
                    font_table,
                    lines,
                    x + dx,
                    y + dy,
                    baseline,
                    outline_paint,
                    font_size,
                );
            }
        }
        draw_anchored_lines(
            canvas,
            glyph_cache,
            swash,
            font_system,
            font_table,
            lines,
            x,
            y,
            baseline,
            paint,
            font_size,
        );
        return;
    }

    let Some(first) = lines.first() else {
        return;
    };
    let first_advance = first.baseline_y;
    let mut prepared = Vec::with_capacity(lines.len());
    for line in lines {
        let alphabetic_y = baseline_to_alphabetic(y, baseline, line.max_ascent, line.max_descent)
            + line.baseline_y
            - first_advance;
        prepared.push(build_glyph_commands(
            &mut FemtovgPages { canvas },
            glyph_cache,
            swash,
            font_system,
            font_table,
            &line.glyphs,
            x,
            alphabetic_y,
            font_size,
        ));
    }

    for commands in &prepared {
        submit_glyph_commands(
            canvas,
            glyph_cache,
            outline_glyph_commands(commands, rings),
            outline_paint,
            font_size,
        );
    }
    for commands in prepared {
        submit_glyph_commands(canvas, glyph_cache, commands, paint, font_size);
    }
}

// ── Renderer trait implementation ───────────────────────────────────

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

    fn dim_style(&self, style: &TextStyle) -> TextStyle {
        TextStyle {
            color: self.dim(style.color),
            outline_color: self.dim(style.outline_color),
            ..*style
        }
    }

    /// Borrowed while nothing dims, so a lit paragraph clones no spans.
    fn dim_spans<'s>(&self, spans: &'s [SpanData]) -> Cow<'s, [SpanData]> {
        if self.brightness >= 1.0 {
            return Cow::Borrowed(spans);
        }
        spans
            .iter()
            .map(|span| SpanData {
                color: span.color.map(|c| self.dim(c)),
                ..span.clone()
            })
            .collect()
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

impl Renderer for FemtoVgRenderer {
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

    // -- Simple text --

    fn draw_text(&mut self, text: &str, x: f32, y: f32, size: f32, color: Color) {
        let paint = Paint::color(self.paint_color(color));
        let Self {
            canvas,
            font_system,
            paragraph_cache,
            glyph_cache,
            swash,
            font_table,
            ..
        } = self;
        let entry = paragraph_cache.layout_single_line(font_system, sans_line_style(size), text);
        draw_anchored_lines(
            canvas,
            glyph_cache,
            swash,
            font_system,
            font_table,
            &entry.lines,
            x,
            y,
            femtovg::Baseline::Top,
            &paint,
            size,
        );
    }

    fn measure_text(&mut self, text: &str, size: f32) -> f32 {
        self.paragraph_cache
            .measure_single_line(&mut self.font_system, sans_line_style(size), text)
            .width
    }

    // -- Canvas text --

    fn draw_canvas_text(&mut self, text: &str, x: f32, y: f32, style: &TextStyle) {
        let size = style.size as f32;
        let baseline = femtovg_baseline(style.vertical_align);
        let paint = Paint::color(self.paint_color(style.color));
        let outline_paint = Paint::color(self.paint_color(style.outline_color));
        let (width, draw_x) = {
            let Self {
                canvas,
                font_system,
                paragraph_cache,
                glyph_cache,
                swash,
                font_table,
                ..
            } = self;
            let entry =
                paragraph_cache.layout_single_line(font_system, line_style(style, size), text);
            let width = entry.width;
            let draw_x = match style.align {
                TextAlign::Left => x,
                TextAlign::Center => x - width / 2.0,
                TextAlign::Right => x - width,
            };

            if style.outline_color != crate::colors::TRANSPARENT && style.outline_width > 0.0 {
                #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let rings = style.outline_width.ceil() as u32;
                draw_anchored_lines_with_outline(
                    canvas,
                    glyph_cache,
                    swash,
                    font_system,
                    font_table,
                    &entry.lines,
                    draw_x,
                    y,
                    baseline,
                    &outline_paint,
                    &paint,
                    size,
                    rings,
                );
            } else {
                draw_anchored_lines(
                    canvas,
                    glyph_cache,
                    swash,
                    font_system,
                    font_table,
                    &entry.lines,
                    draw_x,
                    y,
                    baseline,
                    &paint,
                    size,
                );
            }
            (width, draw_x)
        };

        // Decorations
        if style.underline || style.strikethrough {
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

        let size = style.size as f32;
        let paint = Paint::color(self.paint_color(style.color));
        // Shaped as one string, never glyph by glyph: per-character layouts
        // would break clusters and ligatures and lose every kern between
        // neighbours, which is exactly what the arc advances are made of.
        let Self {
            canvas,
            font_system,
            paragraph_cache,
            glyph_cache,
            swash,
            font_table,
            ..
        } = self;
        let entry = paragraph_cache.layout_single_line(font_system, line_style(style, size), text);
        let Some(line) = entry.lines.first() else {
            return;
        };

        let widths: Vec<f32> = line.glyphs.iter().map(|glyph| glyph.w).collect();
        let alphabetic_y = baseline_to_alphabetic(
            0.0,
            femtovg::Baseline::Middle,
            line.max_ascent,
            line.max_descent,
        );

        if size > DIRECT_PATH_CUTOFF_PX {
            for (glyph, placement) in line
                .glyphs
                .iter()
                .zip(arc_glyph_layout(&widths, radius, angle, anchor, facing))
            {
                let px = cx + radius * placement.theta.sin();
                let py = cy - radius * placement.theta.cos();
                canvas.save();
                canvas.translate(px, py);
                canvas.rotate(placement.rotation);
                draw_line_glyphs(
                    canvas,
                    glyph_cache,
                    swash,
                    font_system,
                    font_table,
                    std::slice::from_ref(glyph),
                    curved_glyph_origin_x(glyph),
                    alphabetic_y,
                    &paint,
                    size,
                );
                canvas.restore();
            }
            return;
        }

        let commands = build_cached_curved_glyph_commands(
            &mut FemtovgPages { canvas },
            glyph_cache,
            swash,
            font_system,
            &line.glyphs,
            alphabetic_y,
        );
        for (command, placement) in commands
            .into_iter()
            .zip(arc_glyph_layout(&widths, radius, angle, anchor, facing))
        {
            let Some(command) = command else {
                continue;
            };
            let px = cx + radius * placement.theta.sin();
            let py = cy - radius * placement.theta.cos();
            canvas.save();
            canvas.translate(px, py);
            canvas.rotate(placement.rotation);
            submit_glyph_commands(canvas, glyph_cache, std::iter::once(command), &paint, size);
            canvas.restore();
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
        // Autofit fits by wrapping, and canvas text ignores the overflow mode.
        let style = &TextStyle {
            text_overflow: TextOverflow::Wrap,
            ..*style
        };
        let spans = [SpanData {
            text: text.to_string(),
            weight: None,
            color: None,
            size: None,
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
            ..self.dim_style(style)
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
            &mut self.glyph_cache,
            &mut self.swash,
            &self.font_table,
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
        let style = self.dim_style(style);
        let spans = self.dim_spans(spans);
        self.paragraph_cache.draw(
            &mut self.font_system,
            &mut self.canvas,
            &mut self.glyph_cache,
            &mut self.swash,
            &self.font_table,
            &style,
            &spans,
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

    fn reserve_svg(&mut self, tag: &str) -> Option<SvgId> {
        self.icon_registry.reserve(tag)
    }

    fn suspend_svg(&mut self, tag: &str) -> AssetSuspendResult<SvgId> {
        self.icon_registry.suspend_exact(tag)
    }

    fn svg_tag_state(&self, tag: &str) -> AssetTagState<SvgId> {
        self.icon_registry.tag_state(tag)
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
            super::svg::draw_svg(
                &mut self.canvas,
                icon,
                x,
                y,
                w,
                h,
                color,
                anti_alias,
                fills,
                self.brightness,
            );
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

    fn reserve_bitmap(&mut self, tag: &str) -> Option<BitmapId> {
        self.bitmap_registry
            .reserve(tag, femtovg::ImageFlags::empty())
    }

    fn reserve_bitmap_nearest(&mut self, tag: &str) -> Option<BitmapId> {
        self.bitmap_registry
            .reserve(tag, femtovg::ImageFlags::NEAREST)
    }

    fn suspend_bitmap(&mut self, tag: &str) -> AssetSuspendResult<BitmapId> {
        let result = self.bitmap_registry.suspend_exact(tag, &mut self.canvas);
        if let AssetSuspendResult::Suspended(id) = result
            && self.sphere_bitmap_id == Some(id)
        {
            self.sphere_bitmap_id = None;
        }
        result
    }

    fn register_bitmap_rgba(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId> {
        let bitmap_id = self.bitmap_registry.register_rgba(
            tag,
            rgba,
            width,
            height,
            &mut self.canvas,
            femtovg::ImageFlags::empty(),
        )?;
        if self.sphere_bitmap_id == Some(bitmap_id) {
            self.sphere_bitmap_id = None;
        }
        Some(bitmap_id)
    }

    fn register_bitmap_rgba_nearest(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId> {
        let bitmap_id = self.bitmap_registry.register_rgba(
            tag,
            rgba,
            width,
            height,
            &mut self.canvas,
            femtovg::ImageFlags::NEAREST,
        )?;
        if self.sphere_bitmap_id == Some(bitmap_id) {
            self.sphere_bitmap_id = None;
        }
        Some(bitmap_id)
    }

    fn bitmap_tag_state(&self, tag: &str) -> AssetTagState<BitmapId> {
        self.bitmap_registry.tag_state(tag)
    }

    fn draw_bitmap(&mut self, x: f32, y: f32, w: f32, h: f32, bitmap_id: BitmapId) {
        let tint = self.image_tint();
        if let Some(image_id) = self.bitmap_registry.get(bitmap_id) {
            super::bitmap::draw_bitmap(&mut self.canvas, image_id, x, y, w, h, tint);
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
        let tint = self.image_tint();
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
                tint,
            );
        }
    }

    fn register_mesh(&mut self, tag: &str, data: &[u8]) -> Option<MeshId> {
        if !matches!(self.mesh_tag_state(tag), AssetTagState::Resident(_)) {
            crate::gpu_access::assert_gpu_access_authorized();
        }
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
        if matches!(self.mesh_tag_state(tag), AssetTagState::Resident(_)) {
            crate::gpu_access::assert_gpu_access_authorized();
        }
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
        let tint = self.image_tint();
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
            tint,
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
        let tint = self.image_tint();
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (width, height) = (w as u32, h as u32);
        if width == 0 || height == 0 {
            return;
        }

        if matches!(&self.sphere, SphereRendererState::Failed) {
            self.draw_bitmap(x, y, w, h, bitmap_id);
            return;
        }

        // Lazy-init sphere renderer on first call
        if matches!(&self.sphere, SphereRendererState::Uninitialized) {
            match SphereRenderer::new(&self.gl, &mut self.canvas, width, height) {
                Ok(sphere) => self.sphere = SphereRendererState::Ready(sphere),
                Err(e) => {
                    self.sphere = SphereRendererState::Failed;
                    tracing::error!("sphere init failed: {e}");
                    self.draw_bitmap(x, y, w, h, bitmap_id);
                    return;
                }
            }
        }
        let SphereRendererState::Ready(sphere) = &mut self.sphere else {
            unreachable!("BUG: sphere is initialized or returned above");
        };

        // Resolve the bitmap each frame so an evict+re-register cycle is
        // observed before we touch GL. Skip the draw on registry miss so a
        // stale FBO never gets sampled.
        let Some(image_id) = self.bitmap_registry.get(bitmap_id) else {
            self.sphere_bitmap_id = None;
            return;
        };

        // RGBA replacement, exact suspension, and eviction clear the binding,
        // so an equal BitmapId still refers to the bound native texture.
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
        super::bitmap::draw_bitmap(&mut self.canvas, image_id, x, y, w, h, tint);
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
        // `reset` drops transform and scissor, and the scale has to go back on:
        // the FBO is sized in physical pixels while the inner draw is logical.
        self.canvas.save();
        self.canvas.reset();
        self.scale_to_logical(dpi);
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
        // Not dimmed: the tint multiplies the inner draw, which already was.
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
        // only as a tessellation hint. `clear_rect` is untransformed too.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (pw, ph) = (
            (width as f32 * dpi_scale) as u32,
            (height as f32 * dpi_scale) as u32,
        );
        self.canvas.set_size(pw, ph, dpi_scale);
        match clear {
            FrameClear::OpaqueBlack => {
                self.canvas
                    .clear_rect(0, 0, pw, ph, femtovg::Color::rgbf(0.0, 0.0, 0.0));
            }
            FrameClear::TransparentBlack => {
                self.canvas
                    .clear_rect(0, 0, pw, ph, femtovg::Color::rgbaf(0.0, 0.0, 0.0, 0.0));
            }
            // Keeping the target's pixels is the point of a damage-tracked
            // frame; clearing would wipe the regions it deliberately reuses.
            FrameClear::Keep => {}
        }
        self.scale_to_logical(dpi_scale);
        self.paragraph_cache.begin_frame();
        self.brightness = 1.0;
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

    fn evict_prefix(&mut self, prefix: &str) -> usize {
        let mut n = self.icon_registry.evict_prefix(prefix);
        n += self.bitmap_registry.evict_prefix(prefix, &mut self.canvas);
        if let Some(mesh) = self.mesh_renderer.as_mut() {
            n += mesh.evict_prefix(&self.gl, prefix);
        } else {
            n += self.pending_mesh_reservations.evict_prefix(prefix);
        }
        if self
            .sphere_bitmap_id
            .is_some_and(|id| self.bitmap_registry.get(id).is_none())
        {
            self.sphere_bitmap_id = None;
        }
        // Static layers share the namespace, so a departing slot reclaims its
        // layer along with its icons, bitmaps and meshes. Deliberately not
        // counted: the total reports what the guest's own eviction removed, and
        // a layer is renderer scratch the guest never registered.
        let canvas = &mut self.canvas;
        self.static_layers.retain(|key, layer| {
            if key.starts_with(prefix) {
                canvas.delete_image(layer.image);
                false
            } else {
                true
            }
        });
        self.incomplete_static_layers
            .retain(|key, _| !key.starts_with(prefix));
        n
    }

    fn evict_prefix_requires_gpu_access(&self, prefix: &str) -> bool {
        self.bitmap_registry.has_resident_prefix(prefix)
            || self
                .mesh_renderer
                .as_ref()
                .is_some_and(|mesh| mesh.has_resident_prefix(prefix))
            // Reclaiming a layer deletes a femtovg image, so the eviction needs
            // the GPU lock.
            || self
                .static_layers
                .keys()
                .any(|key| key.starts_with(prefix))
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

    fn text_atlas_resident_bytes(&self) -> u64 {
        as_u64(self.glyph_cache.resident_atlas_bytes())
    }

    fn glyph_cache_counters(&self) -> GlyphCacheCounters {
        *self.glyph_cache.counters()
    }

    fn glyph_cache_metadata_capacity_bytes(&self) -> u64 {
        as_u64(self.glyph_cache.metadata_capacity_bytes())
    }

    fn text_layout_counters(&self) -> TextLayoutCounters {
        self.paragraph_cache.counters()
    }
    fn begin_static_layer(&mut self, key: &str, width: u32, height: u32) -> bool {
        if self.render_to_texture != RenderTargetProbe::Working {
            return false;
        }
        let dpi_scale = self.dpi_scale;
        let Some(size) = physical_size(width, height, dpi_scale) else {
            // No layer can have this size, so whatever the key held is stale.
            self.invalidate_static_layer(key);
            return false;
        };
        if self.incomplete_static_layers.get(key) == Some(&size) {
            // A layer captured at another size will never be blitted again.
            self.invalidate_static_layer(key);
            return false;
        }
        let first_retarget = !self.static_layers.get(key).is_some_and(|layer| {
            layer.width == width
                && layer.height == height
                && (layer.dpi_scale - dpi_scale).abs() < f32::EPSILON
        });
        if first_retarget {
            // A geometry change makes the existing texture the wrong shape.
            self.invalidate_static_layer(key);
            // FLIP_Y, not a canvas transform: femtovg's cheap
            // `is_straight_tinted_image` path ignores the canvas transform,
            // while `FLIP_Y` folds into the paint's inverse transform and so
            // survives it.
            let Ok(image) = self.canvas.create_image_empty(
                usize::from(size.width.get()),
                usize::from(size.height.get()),
                femtovg::PixelFormat::Rgba8,
                femtovg::ImageFlags::FLIP_Y,
            ) else {
                return false;
            };
            self.static_layers.insert(
                key.to_owned(),
                StaticLayer {
                    image,
                    width,
                    height,
                    dpi_scale,
                    captured: false,
                },
            );
        }
        let Some(layer) = self.static_layers.get(key) else {
            return false;
        };
        let image = layer.image;
        // Deliberately not `begin_frame_to_image`: that opens a new frame on
        // the paragraph cache, and a capture is a second pass over the frame
        // already in progress rather than a new one — the double step would
        // charge this frame's shaping twice over. Canvas size and dpi are
        // already the frame's, so only the target and the clear change here.
        self.canvas.set_render_target(RenderTarget::Image(image));

        // femtovg reports no failed retarget,
        // and a skipped one leaves the caller's export buffer bound.
        // The static pass would then paint over the frame,
        // `end_static_layer` would mark the layer captured anyway,
        // and every later blit would copy an uninitialised texture over the widget.
        // Ask GL what is bound instead of trusting the call.
        //
        // The flush is what makes the question answerable: `set_render_target`
        // only queues the command, so without it GL still reports the frame's
        // own target and the check reads as a failure on every capture.
        self.canvas.flush();
        // SAFETY: the renderer's GL context is current for the whole frame.
        let bound = unsafe { self.gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING) };
        // femtovg never renders a layer into the default framebuffer,
        // so finding that bound is as much a failed retarget as finding the screen's.
        if bound.is_none_or(|fbo| Some(fbo) == self.screen_fbo) {
            tracing::error!(
                key,
                "static layer retarget failed; falling back to a full pass"
            );
            self.canvas.set_render_target(RenderTarget::Screen);
            self.invalidate_static_layer(key);
            return false;
        }
        // A non-screen binding is not yet a usable one:
        // a failing `Framebuffer::new` leaves its incomplete framebuffer bound.
        // The probe has settled the format, so what fails here is this layer,
        // most likely its size; it is refused at that size rather than retried every frame.
        // SAFETY: the renderer's GL context is current for the whole frame.
        let status = unsafe { self.gl.check_framebuffer_status(glow::FRAMEBUFFER) };
        if status != glow::FRAMEBUFFER_COMPLETE {
            tracing::error!(
                key,
                status,
                width,
                height,
                "static layer framebuffer incomplete; not caching it at this size"
            );
            self.incomplete_static_layers.insert(key.to_owned(), size);
            self.canvas.set_render_target(RenderTarget::Screen);
            // SAFETY: the context is current for the frame,
            // and on a first retarget the framebuffer `Framebuffer::new` failed on is still bound.
            unsafe {
                if first_retarget {
                    self.delete_orphaned_framebuffer();
                }
                // Rebind now rather than at the next flush,
                // so raw passes before it do not draw into the incomplete framebuffer.
                self.gl.bind_framebuffer(glow::FRAMEBUFFER, self.screen_fbo);
                // The failed allocation leaves its error queued,
                // where a later `get_error` would pin it on an unrelated draw.
                self.drain_gl_errors();
            }
            self.invalidate_static_layer(key);
            return false;
        }
        // femtovg builds the layer's framebuffer during the retarget above,
        // so the swap has nothing to act on any earlier than here.
        // SAFETY: the context is current for the frame, and the check above
        // found the layer's framebuffer bound.
        unsafe { self.share_layer_stencil(size) };
        // `drop_shadow` restores `frame_target` after its offscreen pass, so the
        // capture has to own it: a shadowed *static* draw sits in `Band::Below`
        // and is emitted here, and leaving this on `Screen` would send that
        // shadow — and every static draw queued after it — to the export buffer
        // while the layer is still marked captured.
        debug_assert!(
            matches!(self.frame_target, RenderTarget::Screen),
            "a static-layer capture may only borrow a screen-backed frame; \
             `end_static_layer` hands the target back as `Screen`",
        );
        self.frame_target = RenderTarget::Image(image);

        // Opaque, because the frame itself starts opaque black: rasterising
        // static content against transparency and compositing it later blends
        // those edges twice, which shows as antialiasing drift on thin geometry
        // and text. Opaque makes the blit a copy rather than a blend.
        self.canvas.clear_rect(
            0,
            0,
            u32::from(size.width.get()),
            u32::from(size.height.get()),
            femtovg::Color::rgbf(0.0, 0.0, 0.0),
        );
        true
    }

    fn end_static_layer(&mut self, key: &str) {
        // The capture is still only queued, and femtovg binds a target when it
        // *executes* a command rather than at flush start. Draining here, while
        // the layer's framebuffer is still bound, is what puts the static half
        // into the layer; without it the rebind below sends the whole capture
        // to the screen and leaves the layer undefined.
        self.canvas.flush();
        self.frame_target = RenderTarget::Screen;
        self.canvas.set_render_target(RenderTarget::Screen);
        // Keep GL agreeing with the retarget just queued, so raw passes before
        // the next flush do not land in the layer.
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, self.screen_fbo);
        }
        if let Some(layer) = self.static_layers.get_mut(key) {
            layer.captured = true;
        }
    }

    fn blit_static_layer(&mut self, key: &str) -> bool {
        let Some(layer) = self.static_layers.get(key) else {
            return false;
        };
        if !layer.captured {
            return false;
        }
        let (image, w, h) = (layer.image, layer.width as f32, layer.height as f32);
        if self.blit_static_layer_raw(image) {
            return true;
        }

        // No canvas transform: the fast path below ignores it, and the Y-flip
        // comes from the `ImageFlags::FLIP_Y` set at allocation.
        self.canvas.save();
        let paint = Paint::image(image, 0.0, 0.0, w, h, 0.0, 1.0).with_anti_alias(false);
        let mut path = Path::new();
        path.rect(0.0, 0.0, w, h);
        self.canvas.fill_path(&path, &paint);
        self.canvas.restore();
        true
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "damage rectangles are small positive values within the surface"
    )]
    fn blit_static_layer_rects(&mut self, key: &str, rects: &[Rect]) -> bool {
        if rects.is_empty() {
            return false;
        }
        let Some(layer) = self.static_layers.get(key) else {
            return false;
        };
        if !layer.captured {
            return false;
        }
        let image = layer.image;
        let Ok(texture) = self.canvas.get_native_texture(image) else {
            return false;
        };
        self.canvas.flush();
        if self.raw_blit.is_none() {
            self.raw_blit = RawBlit::new(&self.gl);
        }
        let Some(blit) = self.raw_blit.as_ref() else {
            return false;
        };

        let dpi = self.dpi_scale;
        let (width_px, height_px) = ((self.width * dpi) as i32, (self.height * dpi) as i32);
        // SAFETY: the renderer's GL context is current for the whole frame.
        unsafe {
            self.gl.disable(glow::BLEND);
            self.gl.disable(glow::STENCIL_TEST);
            self.gl.enable(glow::SCISSOR_TEST);
            self.gl.use_program(Some(blit.program));
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            if let Some(vao) = blit.vao {
                self.gl.bind_vertex_array(Some(vao));
            }
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(blit.vbo));
            let stride = 4 * F32_BYTES;
            self.gl.enable_vertex_attrib_array(blit.pos_loc);
            self.gl
                .vertex_attrib_pointer_f32(blit.pos_loc, 2, glow::FLOAT, false, stride, 0);
            self.gl.enable_vertex_attrib_array(blit.uv_loc);
            self.gl.vertex_attrib_pointer_f32(
                blit.uv_loc,
                2,
                glow::FLOAT,
                false,
                stride,
                2 * F32_BYTES,
            );

            for &rect in rects {
                // The quad covers the whole surface and the scissor restricts
                // what is written, so cost tracks damage area, not rect count.
                let (px, py, pw, ph) = scissor_box(rect, dpi, width_px, height_px);
                self.gl.scissor(px, py, pw, ph);
                self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }

            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.disable_vertex_attrib_array(blit.pos_loc);
            self.gl.disable_vertex_attrib_array(blit.uv_loc);
            if blit.vao.is_some() {
                self.gl.bind_vertex_array(None);
            }
        }
        true
    }

    fn paragraph_cache_stats(&self) -> (u32, usize) {
        self.paragraph_cache.stats()
    }

    fn invalidate_static_layer(&mut self, key: &str) {
        if let Some(layer) = self.static_layers.remove(key) {
            self.canvas.delete_image(layer.image);
        }
    }
}

fn as_u64(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// The line style behind [`Renderer::draw_text`] and [`Renderer::measure_text`],
/// whose signatures carry no family, weight or slant.
pub(crate) fn sans_line_style(size: f32) -> LineStyle {
    LineStyle {
        family: FontFamily::Sans,
        weight: FontWeight::REGULAR,
        italic: false,
        size,
    }
}

/// The line style of a canvas [`TextStyle`], whose `size` is whole-pixel `u32`
/// while animated callers pass the fractional size separately.
fn line_style(style: &TextStyle, size: f32) -> LineStyle {
    LineStyle {
        family: style.family,
        weight: style.weight,
        italic: style.italic,
        size,
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

#[cfg(test)]
mod tests;
