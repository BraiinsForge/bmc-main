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

use super::layout::{ButtonContext, split_min_content_words};
use super::*;
use crate::FrameTimings;
use crate::components::{ButtonSize, ButtonStyle};
use crate::gpu::mesh::MeshDrawArgs;
use crate::interaction::InteractionState;
use crate::interaction::Rect;
use crate::renderer::prelude::*;
use crate::renderer::{FrameBackground, FrameClear};
// Vocabulary modules rather than the root, whose component
// builders clash with taffy's prelude, `span` for one.
use bmc_ui::{animation::*, arc::*, colors::*, fill::*, ids::*, relative_time::*, text::*};
use std::collections::HashMap;
use taffy::prelude::*;

mod banding;
mod capture;
mod damage_span;
mod dim;
mod emit_mode;
mod intrinsic_size;
mod modal;
mod text_overflow_layout;

/// Records which key each static-layer call carried: one renderer serves
/// every slot, so the key is all that keeps two slots' layers apart.
#[derive(Default)]
struct KeyRecordingRenderer {
    draws: Vec<crate::interaction::Rect>,
    calls: Vec<(&'static str, String)>,
    captured: Vec<String>,
    /// Makes `end_static_layer` lose the layer, as an eviction mid-capture does.
    lose_captures: bool,
    /// How many draws preceded each `clear_frame`.
    clears: Vec<usize>,
    paragraphs: Vec<String>,
    /// Each `fill_rect`, with the brightness it painted at.
    fills: Vec<(crate::interaction::Rect, Color, f32)>,
    /// `None` means undimmed: the derived `Default` would start at black.
    brightness: Option<f32>,
    meshes: Vec<(MeshId, u8)>,
}

impl KeyRecordingRenderer {
    fn drawn(&self) -> usize {
        self.draws.len() + self.meshes.len()
    }
}

impl RenderFrame for KeyRecordingRenderer {
    fn save(&mut self) {}

    fn restore(&mut self) {}

    fn translate(&mut self, _x: f32, _y: f32) {}

    fn rotate(&mut self, _angle_radians: f32) {}

    fn push_scissor(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {}

    fn pop_scissor(&mut self) {}

    fn brightness(&self) -> f32 {
        self.brightness.unwrap_or(1.0)
    }

    fn set_brightness(&mut self, brightness: f32) {
        self.brightness = Some(brightness);
    }

    fn begin_frame(&mut self, _width: u32, _height: u32, _dpi_scale: f32) {}

    fn begin_frame_with_clear(
        &mut self,
        _width: u32,
        _height: u32,
        _dpi_scale: f32,
        _background: FrameBackground,
        _clear: FrameClear,
    ) {
    }

    fn clear_frame(&mut self) {
        self.clears.push(self.drawn());
    }

    fn flush(&mut self) {}

    fn width(&self) -> f32 {
        0.0
    }

    fn height(&self) -> f32 {
        0.0
    }
}

impl RenderShapes for KeyRecordingRenderer {
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let rect = crate::interaction::Rect::new(x, y, w, h);
        self.draws.push(rect);
        self.fills.push((rect, color, self.brightness()));
    }

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, _radius: f32, _color: Color) {
        self.draws.push(crate::interaction::Rect::new(x, y, w, h));
    }

    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, _color: Color) {
        self.draws.push(crate::interaction::Rect::new(
            cx - r,
            cy - r,
            2.0 * r,
            2.0 * r,
        ));
    }

    fn fill_rect_paint(&mut self, x: f32, y: f32, w: f32, h: f32, _fill: &Fill) {
        self.draws.push(crate::interaction::Rect::new(x, y, w, h));
    }

    fn fill_circle_paint(&mut self, cx: f32, cy: f32, r: f32, _fill: &Fill) {
        self.draws.push(crate::interaction::Rect::new(
            cx - r,
            cy - r,
            2.0 * r,
            2.0 * r,
        ));
    }

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
        self.calls.push(("stroke", String::new()));
    }

    fn draw_line(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _width: f32, _color: Color) {}

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

impl RenderText for KeyRecordingRenderer {
    fn draw_text(&mut self, _text: &str, _x: f32, _y: f32, _size: f32, _color: Color) {}

    fn measure_text(&mut self, _text: &str, _size: f32) -> f32 {
        0.0
    }

    fn measure_paragraph(
        &mut self,
        _style: &TextStyle,
        _spans: &[Span],
        _max_width: Option<f32>,
    ) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn draw_paragraph(
        &mut self,
        _style: &TextStyle,
        spans: &[Span],
        _x: f32,
        _y: f32,
        _max_width: f32,
    ) {
        self.paragraphs
            .extend(spans.iter().map(|span| span.text.clone()));
    }

    fn draw_paragraph_clipped(
        &mut self,
        _style: &TextStyle,
        _spans: &[Span],
        _x: f32,
        _y: f32,
        _max_width: f32,
        _clip_top: f32,
        _clip_bottom: f32,
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
}

impl RenderAssets for KeyRecordingRenderer {
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

    fn register_bitmap_rgba_nearest(
        &mut self,
        _tag: &str,
        _rgba: &[u8],
        _width: u32,
        _height: u32,
    ) -> Option<BitmapId> {
        None
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
        slot_index: u8,
        mesh_id: MeshId,
        _args: MeshDrawArgs,
    ) {
        self.meshes.push((mesh_id, slot_index));
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

    fn evict_prefix(&mut self, _prefix: &str) -> usize {
        0
    }

    fn bitmap_resident_bytes(&self) -> u64 {
        0
    }
}

impl RenderOffscreen for KeyRecordingRenderer {
    fn begin_static_layer(&mut self, key: &str, _width: u32, _height: u32) -> bool {
        self.calls.push(("begin", key.to_owned()));
        self.captured.retain(|k| k != key);
        true
    }

    fn end_static_layer(&mut self, key: &str) {
        self.calls.push(("end", key.to_owned()));
        if !self.lose_captures {
            self.captured.push(key.to_owned());
        }
    }

    fn blit_static_layer(&mut self, key: &str) -> bool {
        self.calls.push(("blit", key.to_owned()));
        self.captured.iter().any(|k| k == key)
    }

    fn invalidate_static_layer(&mut self, key: &str) {
        self.calls.push(("invalidate", key.to_owned()));
        self.captured.retain(|k| k != key);
    }

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
        _inner: &mut dyn FnMut(&mut dyn Renderer),
    ) {
    }
}

struct SlotState {
    interaction: InteractionState,
    modal_states: HashMap<String, crate::ModalState>,
    scroll_states: HashMap<String, crate::ScrollState>,
    animation_states: HashMap<u64, crate::AnimationState>,
    transition_states: HashMap<crate::TransitionStateKey, crate::TransitionState>,
    taffy: TaffyTree<super::NodeContext>,
}

impl Default for SlotState {
    fn default() -> Self {
        Self {
            interaction: InteractionState::new(),
            modal_states: HashMap::new(),
            scroll_states: HashMap::new(),
            animation_states: HashMap::new(),
            transition_states: HashMap::new(),
            taffy: TaffyTree::new(),
        }
    }
}

fn render_with(
    state: &mut SlotState,
    renderer: &mut KeyRecordingRenderer,
    tree: &Node,
    static_layer: LayerUse,
) -> super::TreeResult {
    let mut ctx = ProcessContext {
        interaction: &mut state.interaction,
        modal_states: &mut state.modal_states,
        scroll_states: &mut state.scroll_states,
        animation_states: &mut state.animation_states,
        transition_states: &mut state.transition_states,
        taffy: &mut state.taffy,
        frame_counter: 1,
        delta_ms: 16,
        now_unix_secs: 0,
        emit: super::EmitMode::All,
        static_layer,
        static_layer_key: "modal-test",
        layout_outlines: false,
        stroke_outlines: false,
        damage_rects: &[],
    };
    let mut timings = FrameTimings::default();
    layout_and_render(tree, 200.0, 200.0, renderer, &mut timings, &mut ctx)
        .expect("BUG: layout_and_render must succeed for the modal tree")
        .0
}

/// The cached-frame pass: dynamic draws only, scissored to `damage`.
fn render_dynamic_pass(
    state: &mut SlotState,
    renderer: &mut KeyRecordingRenderer,
    tree: &Node,
    damage: &[crate::interaction::Rect],
) {
    let mut ctx = ProcessContext {
        interaction: &mut state.interaction,
        modal_states: &mut state.modal_states,
        scroll_states: &mut state.scroll_states,
        animation_states: &mut state.animation_states,
        transition_states: &mut state.transition_states,
        taffy: &mut state.taffy,
        frame_counter: 3,
        delta_ms: 16,
        now_unix_secs: 0,
        emit: super::EmitMode::DynamicOnly,
        static_layer: LayerUse::Reuse,
        static_layer_key: "damage-test",
        layout_outlines: false,
        stroke_outlines: false,
        damage_rects: damage,
    };
    let mut timings = FrameTimings::default();
    layout_and_render(tree, 100.0, 100.0, renderer, &mut timings, &mut ctx)
        .expect("BUG: layout_and_render must succeed for the dynamic pass");
}

/// One walk in `emit` with no layer of its own:
/// a capture frame's two halves, run singly so a test can read them apart.
fn render_pass(
    state: &mut SlotState,
    renderer: &mut KeyRecordingRenderer,
    tree: &Node,
    emit: super::EmitMode,
) {
    let mut ctx = ProcessContext {
        interaction: &mut state.interaction,
        modal_states: &mut state.modal_states,
        scroll_states: &mut state.scroll_states,
        animation_states: &mut state.animation_states,
        transition_states: &mut state.transition_states,
        taffy: &mut state.taffy,
        frame_counter: 1,
        delta_ms: 16,
        now_unix_secs: 0,
        emit,
        static_layer: LayerUse::Ignore,
        static_layer_key: "band-test",
        layout_outlines: false,
        stroke_outlines: false,
        damage_rects: &[],
    };
    let mut timings = FrameTimings::default();
    layout_and_render(tree, 100.0, 100.0, renderer, &mut timings, &mut ctx)
        .expect("BUG: layout_and_render must succeed for a banded tree");
}

fn render_once(
    state: &mut SlotState,
    renderer: &mut KeyRecordingRenderer,
    tree: &Node,
) -> super::TreeResult {
    let mut ctx = ProcessContext {
        interaction: &mut state.interaction,
        modal_states: &mut state.modal_states,
        scroll_states: &mut state.scroll_states,
        animation_states: &mut state.animation_states,
        transition_states: &mut state.transition_states,
        taffy: &mut state.taffy,
        frame_counter: 1,
        delta_ms: 16,
        now_unix_secs: 0,
        emit: super::EmitMode::All,
        static_layer: LayerUse::Ignore,
        static_layer_key: "modal-test",
        layout_outlines: false,
        stroke_outlines: false,
        damage_rects: &[],
    };
    let mut timings = FrameTimings::default();
    let (result, _) = layout_and_render(tree, 100.0, 100.0, renderer, &mut timings, &mut ctx)
        .expect("BUG: layout_and_render must succeed for a modal tree");
    result
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "expected brightness {expected}, got {actual}"
    );
}

/// `render_once` with the static layer live, so the frame splits into
/// a capture pass and a dynamic pass — every scrolling frame's shape.
fn capture_once(
    state: &mut SlotState,
    renderer: &mut KeyRecordingRenderer,
    tree: &Node,
) -> super::TreeResult {
    capture_collecting(state, renderer, tree, false, false)
}

/// [`capture_once`], reporting each laid-out box when `layout_outlines` asks
/// and stroking it into the frame when `stroke_outlines` does.
fn capture_collecting(
    state: &mut SlotState,
    renderer: &mut KeyRecordingRenderer,
    tree: &Node,
    layout_outlines: bool,
    stroke_outlines: bool,
) -> super::TreeResult {
    let mut ctx = ProcessContext {
        interaction: &mut state.interaction,
        modal_states: &mut state.modal_states,
        scroll_states: &mut state.scroll_states,
        animation_states: &mut state.animation_states,
        transition_states: &mut state.transition_states,
        taffy: &mut state.taffy,
        frame_counter: 1,
        delta_ms: 16,
        now_unix_secs: 0,
        emit: super::EmitMode::All,
        static_layer: LayerUse::Capture,
        static_layer_key: "scroll-test",
        layout_outlines,
        stroke_outlines,
        damage_rects: &[],
    };
    let mut timings = FrameTimings::default();
    let (result, _) = layout_and_render(tree, 100.0, 100.0, renderer, &mut timings, &mut ctx)
        .expect("BUG: layout_and_render must succeed for a scroll tree");
    result
}

/// A canvas whose one draw animates, so the walk tags it dynamic and the
/// cached-frame pass repaints it.
fn moving_dot(x: f32) -> Node {
    Node::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws: vec![animating(super::Draw::Rect {
            x,
            y: 10.0,
            w: 12.0,
            h: 12.0,
            fill: Fill::Solid(Color::from_rgb(9, 9, 9)),
        })],
    }
}

/// `inner` under a never-ending alpha animation, so it is always dynamic.
fn animating(inner: super::Draw) -> super::Draw {
    super::Draw::Modified {
        animations: vec![super::AnimationDef {
            property: AnimProperty::Alpha,
            from: 0.0,
            to: 1.0,
            duration_ms: 500,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(inner),
    }
}

fn within(outer: &[crate::interaction::Rect], inner: crate::interaction::Rect) -> bool {
    outer.iter().any(|o| {
        o.x <= inner.x
            && o.y <= inner.y
            && o.x + o.w >= inner.x + inner.w
            && o.y + o.h >= inner.y + inner.h
    })
}

#[test]
fn test_props_size() {
    // In-memory size may differ from wire SIZE due to alignment padding
    assert!(std::mem::size_of::<PropsData>() >= PropsData::SIZE);
}
