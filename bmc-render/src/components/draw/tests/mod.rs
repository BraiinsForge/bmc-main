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

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::transition::{interpolate_draw_values, normalize_angle_deg, shortest_angle_delta_deg};
use super::*;
use crate::renderer::RendererAssetResolver;
use crate::tree::{AutoFit, SpanData};
use crate::{PrevDrawValues, TransitionStateKey};
use bmc_wasm_protocol::*;

mod dash;
mod dispatch;
mod transition;

#[derive(Debug)]
enum RenderEvent {
    Save,
    Restore,
    Translate(f32, f32),
    Rotate(f32),
    Arc {
        cx: f32,
        cy: f32,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        width: f32,
        fill: ArcFill,
        segments: ArcSegments,
        cap: ArcCap,
    },
    Sphere {
        center_lon: f32,
    },
    CurvedText {
        cx: f32,
        cy: f32,
        radius: f32,
        angle: f32,
        anchor: ArcAnchor,
        facing: ArcTextFacing,
        text: String,
        style: TextStyle,
    },
    AutofitText {
        x: f32,
        y: f32,
        box_width: f32,
        box_height: f32,
        text: String,
        size: u32,
        mode: AutoFit,
        min_size: u16,
        max_size: u16,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Asset {
    Svg(SvgId),
    Bitmap(BitmapId),
    Mesh(MeshId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssetEvent {
    Resolve(Asset),
    Draw(Asset),
}

#[derive(Default)]
struct RecordingRenderer {
    events: Vec<RenderEvent>,
    rects: Vec<(f32, f32, f32, f32, Color)>,
    asset_events: Rc<RefCell<Vec<AssetEvent>>>,
}

impl RenderFrame for RecordingRenderer {
    fn save(&mut self) {
        self.events.push(RenderEvent::Save);
    }

    fn restore(&mut self) {
        self.events.push(RenderEvent::Restore);
    }

    fn translate(&mut self, x: f32, y: f32) {
        self.events.push(RenderEvent::Translate(x, y));
    }

    fn rotate(&mut self, angle_radians: f32) {
        self.events.push(RenderEvent::Rotate(angle_radians));
    }

    fn push_scissor(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {}

    fn pop_scissor(&mut self) {}

    fn brightness(&self) -> f32 {
        1.0
    }

    fn set_brightness(&mut self, _brightness: f32) {}

    fn begin_frame(&mut self, _width: u32, _height: u32, _dpi_scale: f32) {}

    fn flush(&mut self) {}

    fn width(&self) -> f32 {
        0.0
    }

    fn height(&self) -> f32 {
        0.0
    }
}

impl RenderShapes for RecordingRenderer {
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.rects.push((x, y, w, h, color));
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
        self.events.push(RenderEvent::Arc {
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            fill: *fill,
            segments: segments.clone(),
            cap,
        });
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
}

impl RenderText for RecordingRenderer {
    fn draw_text(&mut self, _text: &str, _x: f32, _y: f32, _size: f32, _color: Color) {}

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

    fn draw_canvas_text(&mut self, _text: &str, _x: f32, _y: f32, _style: &TextStyle) {}

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
        self.events.push(RenderEvent::CurvedText {
            cx,
            cy,
            radius,
            angle,
            anchor,
            facing,
            text: text.to_owned(),
            style: *style,
        });
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
        self.events.push(RenderEvent::AutofitText {
            x,
            y,
            box_width,
            box_height,
            text: text.to_string(),
            size: style.size,
            mode,
            min_size,
            max_size,
        });
    }
}

impl RenderAssets for RecordingRenderer {
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
        icon_id: SvgId,
        _anti_alias: bool,
        _fills: &[(String, Color)],
    ) {
        self.asset_events
            .borrow_mut()
            .push(AssetEvent::Draw(Asset::Svg(icon_id)));
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

    fn draw_bitmap(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, bitmap_id: BitmapId) {
        self.asset_events
            .borrow_mut()
            .push(AssetEvent::Draw(Asset::Bitmap(bitmap_id)));
    }

    fn draw_nine_patch(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        bitmap_id: BitmapId,
        _left: u16,
        _top: u16,
        _right: u16,
        _bottom: u16,
    ) {
        self.asset_events
            .borrow_mut()
            .push(AssetEvent::Draw(Asset::Bitmap(bitmap_id)));
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
        mesh_id: MeshId,
        _args: MeshDrawArgs,
    ) {
        self.asset_events
            .borrow_mut()
            .push(AssetEvent::Draw(Asset::Mesh(mesh_id)));
    }

    fn draw_sphere(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        bitmap_id: BitmapId,
        _center_lat: f32,
        center_lon: f32,
        _zoom: f32,
        _light_lat: f32,
        _light_lon: f32,
        _atmosphere: bool,
    ) {
        self.events.push(RenderEvent::Sphere { center_lon });
        self.asset_events
            .borrow_mut()
            .push(AssetEvent::Draw(Asset::Bitmap(bitmap_id)));
    }

    fn evict_prefix(&mut self, _prefix: &str) -> usize {
        0
    }

    fn bitmap_resident_bytes(&self) -> u64 {
        0
    }
}

impl RenderOffscreen for RecordingRenderer {
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
}

fn animation_context<'a>(
    animation_states: &'a mut HashMap<u64, AnimationState>,
    transition_states: &'a mut HashMap<TransitionStateKey, TransitionState>,
) -> AnimationContext<'a> {
    AnimationContext {
        emit: crate::tree::EmitMode::default(),
        animation_states,
        transition_states,
        delta_ms: 0,
        frame_counter: 0,
        draw_counter: 0,
        canvas_index: 0,
        draw_in_canvas: 0,
        mesh_slot_counter: 0,
        dynamic_half: crate::tree::DynamicHalf::default(),
        layer_painted: false,
        has_active: false,
        now_unix_secs: 0,
    }
}

#[expect(clippy::too_many_arguments)]
fn render_draw_inner_for_test(
    renderer: &mut dyn Renderer,
    draw: &DrawCommand,
    cx: f32,
    cy: f32,
    cw: f32,
    ch: f32,
    offset: DrawOffset,
    rotation: f32,
    scale: f32,
    alpha: f32,
    orbit_angle_offset: f32,
    color_override: Option<Color>,
    anim_ctx: &mut AnimationContext<'_>,
) {
    let mut target = RenderTarget::new(renderer, None);
    render_draw_inner(
        &mut target,
        draw,
        cx,
        cy,
        cw,
        ch,
        offset,
        rotation,
        scale,
        alpha,
        orbit_angle_offset,
        color_override,
        anim_ctx,
    );
}
