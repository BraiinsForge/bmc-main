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

//! Test-only headless [`Renderer`] stub shared across the crate's unit tests.

use bmc_render::colors::Color;
use bmc_render::gpu::mesh::MeshDrawArgs;
use bmc_render::renderer::Renderer;
use bmc_render::tree::{AutoFit, SpanData, TextStyle};
use bmc_wasm_protocol::{
    ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, BitmapId, Fill, MeshId, SvgId,
};

#[derive(Default)]
pub(crate) struct TestRenderer {
    filled: bool,
    text: Option<String>,
}

impl Renderer for TestRenderer {
    fn fill_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _color: Color) {
        self.filled = true;
    }

    fn fill_rounded_rect(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        _radius: f32,
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

    fn draw_line(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _width: f32, _color: Color) {}

    fn save(&mut self) {}

    fn restore(&mut self) {}

    fn translate(&mut self, _x: f32, _y: f32) {}

    fn rotate(&mut self, _angle_radians: f32) {}

    fn push_scissor(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {}

    fn pop_scissor(&mut self) {}

    fn draw_text(&mut self, text: &str, _x: f32, _y: f32, _size: f32, _color: Color) {
        self.text = Some(text.to_owned());
    }

    fn measure_text(&mut self, _text: &str, _size: f32) -> f32 {
        0.0
    }

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

    fn bitmap_resident_bytes(&self) -> u64 {
        0
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
        0.0
    }

    fn height(&self) -> f32 {
        0.0
    }

    fn evict_prefix(&mut self, _prefix: &str) -> usize {
        0
    }
}
