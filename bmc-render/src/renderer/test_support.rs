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

//! A [`Renderer`] double that shapes text exactly as the GPU renderer does.
//!
//! Its [`Renderer::measure_text`] runs the layout
//! [`crate::gpu::FemtoVgRenderer`] runs, rather than returning a canned width:
//! a canned width keeps every geometry assertion passing across a shaper swap,
//! which is what these tests exist to catch. Draws are recorded or discarded;
//! no GL context is involved.

use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{
    ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, BitmapId, Fill, MeshId, SvgId,
};

use crate::gpu::mesh::MeshDrawArgs;
use crate::gpu::renderer::{build_font_system, sans_line_style};
use crate::gpu::text::ParagraphLayoutCache;
use crate::renderer::Renderer;
use crate::tree::{AutoFit, SpanData, TextStyle};

/// One recorded [`Renderer::draw_text`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawnText {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

/// One recorded [`Renderer::fill_rect`] call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawnRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Records draw calls while measuring text through the production shaper.
pub struct ShapingRecorder {
    pub texts: Vec<DrawnText>,
    pub rects: Vec<DrawnRect>,
    pub rounded_rects: Vec<DrawnRect>,
    font_system: cosmic_text::FontSystem,
    layouts: ParagraphLayoutCache,
    width: f32,
    height: f32,
}

impl ShapingRecorder {
    #[must_use]
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            texts: Vec::new(),
            rects: Vec::new(),
            rounded_rects: Vec::new(),
            font_system: build_font_system(),
            layouts: ParagraphLayoutCache::new(),
            width,
            height,
        }
    }

    /// The single recorded text draw, for tests that expect exactly one.
    ///
    /// # Panics
    ///
    /// Panics unless exactly one [`Renderer::draw_text`] call was recorded.
    #[must_use]
    pub fn only_text(&self) -> &DrawnText {
        assert_eq!(
            self.texts.len(),
            1,
            "expected exactly one text draw, got {:?}",
            self.texts
        );
        &self.texts[0]
    }
}

impl std::fmt::Debug for ShapingRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShapingRecorder")
            .field("texts", &self.texts)
            .field("rects", &self.rects)
            .field("rounded_rects", &self.rounded_rects)
            .finish_non_exhaustive()
    }
}

impl Default for ShapingRecorder {
    fn default() -> Self {
        Self::new(0.0, 0.0)
    }
}

impl Renderer for ShapingRecorder {
    fn measure_text(&mut self, text: &str, size: f32) -> f32 {
        self.layouts
            .layout_single_line(&mut self.font_system, sans_line_style(size), text)
            .width
    }

    fn draw_text(&mut self, text: &str, x: f32, y: f32, size: f32, _color: Color) {
        self.texts.push(DrawnText {
            text: text.to_owned(),
            x,
            y,
            size,
        });
    }

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, _color: Color) {
        self.rects.push(DrawnRect { x, y, w, h });
    }

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, _radius: f32, _color: Color) {
        self.rounded_rects.push(DrawnRect { x, y, w, h });
    }

    fn fill_circle(&mut self, _cx: f32, _cy: f32, _r: f32, _color: Color) {}

    fn fill_rect_paint(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _fill: &Fill) {}

    fn fill_circle_paint(&mut self, _cx: f32, _cy: f32, _r: f32, _fill: &Fill) {}

    fn stroke_arc(
        &mut self,
        _cx: f32,
        _cy: f32,
        _radius: f32,
        _start_angle: f32,
        _end_angle: f32,
        _width: f32,
        _fill: &ArcFill,
        _segments: &ArcSegments,
        _cap: ArcCap,
    ) {
    }

    fn stroke_rect(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        _border_width: f32,
        _color: Color,
    ) {
    }

    fn stroke_rounded_rect(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        _radius: f32,
        _border_width: f32,
        _color: Color,
    ) {
    }

    fn draw_line(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _width: f32, _color: Color) {}

    fn save(&mut self) {}

    fn restore(&mut self) {}

    fn translate(&mut self, _x: f32, _y: f32) {}

    fn rotate(&mut self, _angle_radians: f32) {}

    fn push_scissor(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {}

    fn pop_scissor(&mut self) {}

    fn measure_paragraph(
        &mut self,
        _style: &TextStyle,
        _spans: &[SpanData],
        _max_width: Option<f32>,
    ) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn draw_paragraph(
        &mut self,
        _style: &TextStyle,
        _spans: &[SpanData],
        _x: f32,
        _y: f32,
        _max_width: f32,
    ) {
    }

    fn draw_paragraph_clipped(
        &mut self,
        _style: &TextStyle,
        _spans: &[SpanData],
        _x: f32,
        _y: f32,
        _max_width: f32,
        _clip_top: f32,
        _clip_bottom: f32,
    ) {
    }

    fn register_svg(&mut self, _tag: &str, _data: &[u8]) -> Option<SvgId> {
        None
    }

    fn draw_svg(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        _color: Color,
        _icon_id: SvgId,
        _anti_alias: bool,
        _fills: &[(String, Color)],
    ) {
    }

    fn register_bitmap(&mut self, _tag: &str, _data: &[u8]) -> Option<BitmapId> {
        None
    }

    fn register_bitmap_nearest(&mut self, _tag: &str, _data: &[u8]) -> Option<BitmapId> {
        None
    }

    fn register_bitmap_rgba(
        &mut self,
        _tag: &str,
        _rgba: &[u8],
        _width: u32,
        _height: u32,
    ) -> Option<BitmapId> {
        None
    }

    fn register_bitmap_rgba_nearest(
        &mut self,
        _tag: &str,
        _rgba: &[u8],
        _width: u32,
        _height: u32,
    ) -> Option<BitmapId> {
        None
    }

    fn draw_bitmap(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _bitmap_id: BitmapId) {}

    fn draw_nine_patch(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        _bitmap_id: BitmapId,
        _left: u16,
        _top: u16,
        _right: u16,
        _bottom: u16,
    ) {
    }

    fn register_mesh(&mut self, _tag: &str, _data: &[u8]) -> Option<MeshId> {
        None
    }

    fn draw_mesh(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        _slot_index: u8,
        _mesh_id: MeshId,
        _args: MeshDrawArgs,
    ) {
    }

    fn draw_sphere(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        _bitmap_id: BitmapId,
        _center_lat: f32,
        _center_lon: f32,
        _zoom: f32,
        _light_lat: f32,
        _light_lon: f32,
        _atmosphere: bool,
    ) {
    }

    fn draw_canvas_text(&mut self, _text: &str, _x: f32, _y: f32, _style: &TextStyle) {}

    fn draw_curved_text(
        &mut self,
        _cx: f32,
        _cy: f32,
        _radius: f32,
        _angle: f32,
        _anchor: ArcAnchor,
        _facing: ArcTextFacing,
        _text: &str,
        _style: &TextStyle,
    ) {
    }

    fn draw_autofit_text(
        &mut self,
        _x: f32,
        _y: f32,
        _box_width: f32,
        _box_height: f32,
        _text: &str,
        _style: &TextStyle,
        _mode: AutoFit,
        _min_size: u16,
        _max_size: u16,
    ) {
    }

    fn stroke_path(
        &mut self,
        _points: &[(f32, f32)],
        _stroke_width: f32,
        _color: Color,
        _closed: bool,
        _smooth: bool,
    ) {
    }

    fn fill_path_paint(&mut self, _points: &[(f32, f32)], _fill: &Fill, _smooth: bool) {}

    fn drop_shadow(
        &mut self,
        _cx: f32,
        _cy: f32,
        _fbo_w: u32,
        _fbo_h: u32,
        _dx: f32,
        _dy: f32,
        _blur: f32,
        _color: Color,
        inner: &mut dyn FnMut(&mut dyn Renderer),
    ) {
        inner(self);
    }

    fn begin_frame(&mut self, _width: u32, _height: u32, _dpi_scale: f32) {}

    fn flush(&mut self) {}

    fn width(&self) -> f32 {
        self.width
    }

    fn height(&self) -> f32 {
        self.height
    }

    fn evict_prefix(&mut self, _prefix: &str) -> usize {
        0
    }

    fn bitmap_resident_bytes(&self) -> u64 {
        0
    }
}
