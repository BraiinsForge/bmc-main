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

use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{
    ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, BitmapId, Fill, MeshId, SvgId,
};

use crate::gpu::mesh::MeshDrawArgs;
use crate::tree::{AutoFit, SpanData, TextStyle};

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

    /// Register icon data (compact binary from proc macro) under a stable
    /// `tag`. Idempotent: re-registering the same tag returns the cached ID.
    fn register_svg(&mut self, tag: &str, data: &[u8]) -> Option<SvgId>;

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

    /// Register bitmap data (PNG/JPEG bytes), decode and upload to GPU. Idempotent
    /// by `tag` — re-registering the same tag returns the cached ID.
    fn register_bitmap(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId>;

    /// Register bitmap with nearest-neighbor filtering (no bilinear interpolation).
    /// Use for pixel-art assets (9-patch skins) where bilinear filtering would
    /// cause color bleeding across sub-rect boundaries. Idempotent by `tag`.
    fn register_bitmap_nearest(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId>;

    /// Upload a pre-decoded RGBA buffer (decoded off the render thread). Idempotent by `tag`.
    fn register_bitmap_rgba(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId>;

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

    /// Register mesh binary data, upload VBO/IBO/texture to GPU. Idempotent
    /// by `tag` — re-registering the same tag returns the cached ID.
    fn register_mesh(&mut self, tag: &str, data: &[u8]) -> Option<MeshId>;

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
}
