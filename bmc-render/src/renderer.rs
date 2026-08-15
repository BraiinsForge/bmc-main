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

//! Abstract rendering backend for the widget tree.
//!
//! All coordinates are f32. Implementations may round to integer
//! internally if the backend requires it.

use std::cell::RefCell;

use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{
    ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, BitmapId, Fill, MeshId, SvgId,
};

use crate::gpu::mesh::MeshDrawArgs;
use crate::tree::{AutoFit, SpanData, TextStyle};

/// State of an asset tag's registry reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetTagState<Id> {
    /// The ID has a drawable payload without further registration work.
    Resident(Id),
    /// The ID and tag remain reserved, but it has no drawable payload.
    /// Successful re-registration must restore its payload.
    Suspended(Id),
    /// No reservation is known for the tag.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetSuspendResult<Id> {
    Suspended(Id),
    AlreadySuspended(Id),
    Unknown,
}

/// Base clear policy for a render frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameClear {
    /// Opaque black frame base, used by normal widget surfaces.
    OpaqueBlack,
    /// Transparent black frame base, used by composited overlays.
    TransparentBlack,
}

/// Rendering backend trait.
///
/// Implemented by [`crate::gpu::FemtoVgRenderer`] for GPU-accelerated rendering.
/// The trait is object-safe so `process_tree` can accept `&mut dyn Renderer`.
pub trait Renderer {
    // -- Shapes --

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color);

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color);

    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Color);

    /// Fill a rectangle with a [`Fill`] paint (solid or gradient).
    fn fill_rect_paint(&mut self, x: f32, y: f32, w: f32, h: f32, fill: &Fill);

    /// Fill a circle with a [`Fill`] paint (solid or gradient).
    fn fill_circle_paint(&mut self, cx: f32, cy: f32, r: f32, fill: &Fill);

    /// Stroke a circular arc with an along-arc paint and optional segmentation.
    ///
    /// Angles are radians, `0` at 12 o'clock, increasing clockwise. The gradient
    /// is parameterised over the full `[start_angle, end_angle]` sweep, so it
    /// flows continuously across segment gaps.
    #[expect(clippy::too_many_arguments, reason = "arc geometry is irreducible")]
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
    );

    fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, border_width: f32, color: Color);

    /// Stroke a rounded rectangle outline.
    #[expect(
        clippy::too_many_arguments,
        reason = "rect + radius + stroke geometry is irreducible"
    )]
    fn stroke_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        border_width: f32,
        color: Color,
    );

    fn draw_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color);

    // -- Transform stack --

    fn save(&mut self);
    fn restore(&mut self);
    fn translate(&mut self, x: f32, y: f32);
    fn rotate(&mut self, angle_radians: f32);

    // -- Scissor clipping --

    fn push_scissor(&mut self, x: f32, y: f32, w: f32, h: f32);
    fn pop_scissor(&mut self);

    // -- Simple text --

    fn draw_text(&mut self, text: &str, x: f32, y: f32, size: f32, color: Color);
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

    /// Register icon data under a stable `tag`.
    /// A resident tag returns its ID; a suspended tag restores that reservation.
    fn register_svg(&mut self, tag: &str, data: &[u8]) -> Option<SvgId>;

    fn reserve_svg(&mut self, _tag: &str) -> Option<SvgId> {
        None
    }

    fn suspend_svg(&mut self, _tag: &str) -> AssetSuspendResult<SvgId> {
        AssetSuspendResult::Unknown
    }

    /// Return the SVG reservation state for `tag`.
    ///
    /// The default does not inspect backend state and returns [`AssetTagState::Unknown`].
    fn svg_tag_state(&self, _tag: &str) -> AssetTagState<SvgId> {
        AssetTagState::Unknown
    }

    /// Draw a registered SVG at the given position and size.
    ///
    /// `fills` carries optional per-path colour overrides keyed
    /// by the path's `id` attribute (see `Draw::svg(...).fill(id, color)`).
    /// Pass an empty slice when no per-path recolouring is needed;
    /// the renderer falls back to the whole-icon `color` tint or the SVG's own colours.
    #[expect(clippy::too_many_arguments)]
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
    );

    // -- Bitmaps --

    /// Register bitmap data under a stable `tag`.
    /// A resident tag returns its ID; a suspended tag restores that reservation.
    fn register_bitmap(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId>;

    /// Register bitmap with nearest-neighbor filtering (no bilinear interpolation).
    /// Use for pixel-art assets (9-patch skins) where bilinear filtering would
    /// cause color bleeding across sub-rect boundaries. Resident and suspended
    /// tags follow [`Self::register_bitmap`]'s reservation semantics.
    fn register_bitmap_nearest(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId>;

    fn reserve_bitmap(&mut self, _tag: &str) -> Option<BitmapId> {
        None
    }

    fn reserve_bitmap_nearest(&mut self, _tag: &str) -> Option<BitmapId> {
        None
    }

    fn suspend_bitmap(&mut self, _tag: &str) -> AssetSuspendResult<BitmapId> {
        AssetSuspendResult::Unknown
    }

    /// Upload a pre-decoded RGBA buffer, replacing a tag's resident payload in place.
    fn register_bitmap_rgba(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId>;

    /// Return the bitmap reservation state for `tag`.
    ///
    /// The default does not inspect backend state and returns [`AssetTagState::Unknown`].
    fn bitmap_tag_state(&self, _tag: &str) -> AssetTagState<BitmapId> {
        AssetTagState::Unknown
    }

    /// Draw a registered bitmap at the given position and size.
    fn draw_bitmap(&mut self, x: f32, y: f32, w: f32, h: f32, bitmap_id: BitmapId);

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
        bitmap_id: BitmapId,
        left: u16,
        top: u16,
        right: u16,
        bottom: u16,
    );

    // -- Meshes --

    /// Register mesh data under a stable `tag`.
    /// A resident tag returns its ID; a suspended tag restores that reservation.
    fn register_mesh(&mut self, tag: &str, data: &[u8]) -> Option<MeshId>;

    fn reserve_mesh(&mut self, _tag: &str) -> Option<MeshId> {
        None
    }

    fn suspend_mesh(&mut self, _tag: &str) -> AssetSuspendResult<MeshId> {
        AssetSuspendResult::Unknown
    }

    /// Return the mesh reservation state for `tag`.
    ///
    /// The default does not inspect backend state and returns [`AssetTagState::Unknown`].
    fn mesh_tag_state(&self, _tag: &str) -> AssetTagState<MeshId> {
        AssetTagState::Unknown
    }

    /// Draw a 3D mesh with quaternion-based orientation and optional directional light.
    ///
    /// `slot_index` selects which atlas slot to render into (0..8 for 3×3 grid).
    /// `args` bundles transform, lighting and highlight parameters; see
    /// `MeshDrawArgs` for sentinel conventions (`MeshLighting.pitch == NaN`
    /// disables lighting, `MeshHighlight.u_min == NaN` disables highlight).
    #[expect(clippy::too_many_arguments)]
    fn draw_mesh(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        slot_index: u8,
        mesh_id: MeshId,
        args: MeshDrawArgs,
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
        bitmap_id: BitmapId,
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

    /// Draw `text` scaled to fit the `(box_width, box_height)` rectangle whose
    /// top-left is `(x, y)`. The font size is searched per `mode` within
    /// `[min_size, max_size]` (0 = use defaults: floor 12 / bounded by box).
    #[expect(clippy::too_many_arguments)]
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
    );

    /// Draw styled text with glyph centers placed on a circular arc.
    #[expect(clippy::too_many_arguments)]
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
    );

    // -- Paths --

    /// Stroke a path through the given points.
    /// If `smooth` is true, use Catmull-Rom spline interpolation.
    /// If `closed` is true, join last point to first.
    fn stroke_path(
        &mut self,
        points: &[(f32, f32)],
        stroke_width: f32,
        color: Color,
        closed: bool,
        smooth: bool,
    );

    /// Fill a closed polygon through `points` with a [`Fill`] paint.
    /// If `smooth` is true, use Catmull-Rom spline interpolation.
    fn fill_path_paint(&mut self, points: &[(f32, f32)], fill: &Fill, smooth: bool);

    // -- Drop shadow --

    /// Backend hook for `DrawCommand::Shadow`. `(cx, cy)` is the canvas origin;
    /// `(fbo_w, fbo_h)` an offscreen size holding the inner content;
    /// `inner` rasterises the wrapped draw at FBO-local coordinates.
    ///
    /// Backends without offscreen targets may stub this to just run `inner`.
    #[expect(clippy::too_many_arguments)]
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
    );

    // -- Frame lifecycle --

    /// Begin a frame with DPI scaling.
    ///
    /// `dpi_scale` > 1.0 renders at higher internal resolution for sharper text.
    /// The coordinate system stays at `width × height` logical pixels.
    fn begin_frame(&mut self, width: u32, height: u32, dpi_scale: f32);

    /// Begin a frame with an explicit clear policy.
    fn begin_frame_with_clear(
        &mut self,
        width: u32,
        height: u32,
        dpi_scale: f32,
        _clear: FrameClear,
    ) {
        self.begin_frame(width, height, dpi_scale);
    }

    fn flush(&mut self);
    fn width(&self) -> f32;
    fn height(&self) -> f32;

    // -- Eviction --

    /// Drop every icon, bitmap, and mesh registered under a tag that starts
    /// with `prefix`, releasing the associated GPU resources.
    /// Returns the total count of evicted entries across all three registries.
    fn evict_prefix(&mut self, prefix: &str) -> usize;

    /// Total resident texture bytes across registered bitmaps.
    fn bitmap_resident_bytes(&self) -> u64;

    /// Logical bytes held by resident SVG path commands across the renderer.
    fn svg_resident_path_bytes(&self) -> u64 {
        0
    }

    /// Nominal bytes held by resident mesh buffers and textures.
    fn mesh_resident_bytes(&self) -> u64 {
        0
    }
}

/// Restores suspended renderer assets at the point where a draw first needs them.
pub trait RendererAssetResolver {
    /// Prepare an SVG reservation for drawing. Return `false` to skip the draw.
    fn resolve_svg(&mut self, renderer: &mut dyn Renderer, id: SvgId) -> bool;
    /// Prepare a bitmap reservation for drawing. Return `false` to skip the draw.
    fn resolve_bitmap(&mut self, renderer: &mut dyn Renderer, id: BitmapId) -> bool;
    /// Prepare a mesh reservation for drawing. Return `false` to skip the draw.
    fn resolve_mesh(&mut self, renderer: &mut dyn Renderer, id: MeshId) -> bool;
}

pub(crate) struct RenderTarget<'renderer, 'cell, 'resolver> {
    renderer: &'renderer mut dyn Renderer,
    resolver: Option<&'cell RefCell<&'resolver mut dyn RendererAssetResolver>>,
}

impl<'renderer, 'cell, 'resolver> RenderTarget<'renderer, 'cell, 'resolver> {
    pub(crate) fn new(
        renderer: &'renderer mut dyn Renderer,
        resolver: Option<&'cell RefCell<&'resolver mut dyn RendererAssetResolver>>,
    ) -> Self {
        Self { renderer, resolver }
    }

    #[expect(clippy::too_many_arguments)]
    pub(crate) fn drop_shadow(
        &mut self,
        cx: f32,
        cy: f32,
        fbo_w: u32,
        fbo_h: u32,
        dx: f32,
        dy: f32,
        blur: f32,
        color: Color,
        inner: &mut dyn FnMut(&mut RenderTarget<'_, '_, '_>),
    ) {
        let resolver = self.resolver;
        self.renderer
            .drop_shadow(cx, cy, fbo_w, fbo_h, dx, dy, blur, color, &mut |renderer| {
                let mut target = RenderTarget::new(renderer, resolver);
                inner(&mut target);
            });
    }
}

impl Renderer for RenderTarget<'_, '_, '_> {
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.renderer.fill_rect(x, y, w, h, color);
    }

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        self.renderer.fill_rounded_rect(x, y, w, h, radius, color);
    }

    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        self.renderer.fill_circle(cx, cy, r, color);
    }

    fn fill_rect_paint(&mut self, x: f32, y: f32, w: f32, h: f32, fill: &Fill) {
        self.renderer.fill_rect_paint(x, y, w, h, fill);
    }

    fn fill_circle_paint(&mut self, cx: f32, cy: f32, r: f32, fill: &Fill) {
        self.renderer.fill_circle_paint(cx, cy, r, fill);
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
        self.renderer.stroke_arc(
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            fill,
            segments,
            cap,
        );
    }

    fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, border_width: f32, color: Color) {
        self.renderer.stroke_rect(x, y, w, h, border_width, color);
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
        self.renderer
            .stroke_rounded_rect(x, y, w, h, radius, border_width, color);
    }

    fn draw_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color) {
        self.renderer.draw_line(x1, y1, x2, y2, width, color);
    }

    fn save(&mut self) {
        self.renderer.save();
    }

    fn restore(&mut self) {
        self.renderer.restore();
    }

    fn translate(&mut self, x: f32, y: f32) {
        self.renderer.translate(x, y);
    }

    fn rotate(&mut self, angle_radians: f32) {
        self.renderer.rotate(angle_radians);
    }

    fn push_scissor(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.renderer.push_scissor(x, y, w, h);
    }

    fn pop_scissor(&mut self) {
        self.renderer.pop_scissor();
    }

    fn draw_text(&mut self, text: &str, x: f32, y: f32, size: f32, color: Color) {
        self.renderer.draw_text(text, x, y, size, color);
    }

    fn measure_text(&mut self, text: &str, size: f32) -> f32 {
        self.renderer.measure_text(text, size)
    }

    fn measure_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
    ) -> (f32, f32) {
        self.renderer.measure_paragraph(style, spans, max_width)
    }

    fn draw_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
    ) {
        self.renderer.draw_paragraph(style, spans, x, y, max_width);
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
        self.renderer
            .draw_paragraph_clipped(style, spans, x, y, max_width, clip_top, clip_bottom);
    }

    fn register_svg(&mut self, tag: &str, data: &[u8]) -> Option<SvgId> {
        self.renderer.register_svg(tag, data)
    }

    fn reserve_svg(&mut self, tag: &str) -> Option<SvgId> {
        self.renderer.reserve_svg(tag)
    }

    fn suspend_svg(&mut self, tag: &str) -> AssetSuspendResult<SvgId> {
        self.renderer.suspend_svg(tag)
    }

    fn svg_tag_state(&self, tag: &str) -> AssetTagState<SvgId> {
        self.renderer.svg_tag_state(tag)
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
        if self
            .resolver
            .is_none_or(|resolver| resolver.borrow_mut().resolve_svg(self.renderer, icon_id))
        {
            self.renderer
                .draw_svg(x, y, w, h, color, icon_id, anti_alias, fills);
        }
    }

    fn register_bitmap(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId> {
        self.renderer.register_bitmap(tag, data)
    }

    fn register_bitmap_nearest(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId> {
        self.renderer.register_bitmap_nearest(tag, data)
    }

    fn reserve_bitmap(&mut self, tag: &str) -> Option<BitmapId> {
        self.renderer.reserve_bitmap(tag)
    }

    fn reserve_bitmap_nearest(&mut self, tag: &str) -> Option<BitmapId> {
        self.renderer.reserve_bitmap_nearest(tag)
    }

    fn suspend_bitmap(&mut self, tag: &str) -> AssetSuspendResult<BitmapId> {
        self.renderer.suspend_bitmap(tag)
    }

    fn register_bitmap_rgba(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId> {
        self.renderer.register_bitmap_rgba(tag, rgba, width, height)
    }

    fn bitmap_tag_state(&self, tag: &str) -> AssetTagState<BitmapId> {
        self.renderer.bitmap_tag_state(tag)
    }

    fn draw_bitmap(&mut self, x: f32, y: f32, w: f32, h: f32, bitmap_id: BitmapId) {
        if self.resolver.is_none_or(|resolver| {
            resolver
                .borrow_mut()
                .resolve_bitmap(self.renderer, bitmap_id)
        }) {
            self.renderer.draw_bitmap(x, y, w, h, bitmap_id);
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
        if self.resolver.is_none_or(|resolver| {
            resolver
                .borrow_mut()
                .resolve_bitmap(self.renderer, bitmap_id)
        }) {
            self.renderer
                .draw_nine_patch(x, y, w, h, bitmap_id, left, top, right, bottom);
        }
    }

    fn register_mesh(&mut self, tag: &str, data: &[u8]) -> Option<MeshId> {
        self.renderer.register_mesh(tag, data)
    }

    fn reserve_mesh(&mut self, tag: &str) -> Option<MeshId> {
        self.renderer.reserve_mesh(tag)
    }

    fn suspend_mesh(&mut self, tag: &str) -> AssetSuspendResult<MeshId> {
        self.renderer.suspend_mesh(tag)
    }

    fn mesh_tag_state(&self, tag: &str) -> AssetTagState<MeshId> {
        self.renderer.mesh_tag_state(tag)
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
        if self
            .resolver
            .is_none_or(|resolver| resolver.borrow_mut().resolve_mesh(self.renderer, mesh_id))
        {
            self.renderer
                .draw_mesh(x, y, w, h, slot_index, mesh_id, args);
        }
    }

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
        if self.resolver.is_none_or(|resolver| {
            resolver
                .borrow_mut()
                .resolve_bitmap(self.renderer, bitmap_id)
        }) {
            self.renderer.draw_sphere(
                x, y, w, h, bitmap_id, center_lat, center_lon, zoom, light_lat, light_lon,
                atmosphere,
            );
        }
    }

    fn draw_canvas_text(&mut self, text: &str, x: f32, y: f32, style: &TextStyle) {
        self.renderer.draw_canvas_text(text, x, y, style);
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
        self.renderer.draw_autofit_text(
            x, y, box_width, box_height, text, style, mode, min_size, max_size,
        );
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
        self.renderer
            .draw_curved_text(cx, cy, radius, angle, anchor, facing, text, style);
    }

    fn stroke_path(
        &mut self,
        points: &[(f32, f32)],
        stroke_width: f32,
        color: Color,
        closed: bool,
        smooth: bool,
    ) {
        self.renderer
            .stroke_path(points, stroke_width, color, closed, smooth);
    }

    fn fill_path_paint(&mut self, points: &[(f32, f32)], fill: &Fill, smooth: bool) {
        self.renderer.fill_path_paint(points, fill, smooth);
    }

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
        let resolver = self.resolver;
        self.renderer
            .drop_shadow(cx, cy, fbo_w, fbo_h, dx, dy, blur, color, &mut |renderer| {
                let mut target = RenderTarget::new(renderer, resolver);
                inner(&mut target);
            });
    }

    fn begin_frame(&mut self, width: u32, height: u32, dpi_scale: f32) {
        self.renderer.begin_frame(width, height, dpi_scale);
    }

    fn begin_frame_with_clear(
        &mut self,
        width: u32,
        height: u32,
        dpi_scale: f32,
        clear: FrameClear,
    ) {
        self.renderer
            .begin_frame_with_clear(width, height, dpi_scale, clear);
    }

    fn flush(&mut self) {
        self.renderer.flush();
    }

    fn width(&self) -> f32 {
        self.renderer.width()
    }

    fn height(&self) -> f32 {
        self.renderer.height()
    }

    fn evict_prefix(&mut self, prefix: &str) -> usize {
        self.renderer.evict_prefix(prefix)
    }

    fn bitmap_resident_bytes(&self) -> u64 {
        self.renderer.bitmap_resident_bytes()
    }

    fn svg_resident_path_bytes(&self) -> u64 {
        self.renderer.svg_resident_path_bytes()
    }

    fn mesh_resident_bytes(&self) -> u64 {
        self.renderer.mesh_resident_bytes()
    }
}
