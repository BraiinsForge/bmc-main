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

//! Resting-layout regressions run through the real flex layout: the Large tier
//! splits into two equal halves with every control circle in the bottom one,
//! and BMM101's address table cuts its values, never its labels.
//!
//! The probe renderer wraps paragraphs like a real text backend: constrained
//! below its natural width, a paragraph breaks into more lines the narrower
//! it gets. Taffy resolves a flex container's auto min-size floor by probing
//! content at min-content width, where a wrapping measure degenerates into a
//! per-glyph tower far taller than the panel — the floor then froze the
//! bottom half at the tower height and collapsed the top one. Mock measures
//! that never wrap cannot catch this.
//!
//! A `Clip` or `Ellipsis` line never wraps, as in the renderer,
//! and probed that narrow an `Ellipsis` one keeps its "…".

#![expect(
    clippy::cast_precision_loss,
    reason = "test geometry is far below f32 mantissa precision"
)]

use std::time::Instant;

use bmc_overlay_settings_tray::ui::{Action, Phase};
use bmc_overlay_settings_tray::{
    SettingsTrayRenderState, SettingsTrayView, StatusView, render_settings_tray,
};
use bmc_render::gpu::mesh::MeshDrawArgs;
use bmc_render::renderer::{FrameClear, Renderer};
use bmc_render::tree::{AutoFit, SpanData, TextOverflow, TextStyle};
use bmc_system_overlay::{OverlayViewport, SettingsCaps, ViewportShape};
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{
    ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, BitmapId, Fill, MeshId, SvgId,
};

const DECK: OverlayViewport = OverlayViewport {
    width: 1_280,
    height: 480,
    shape: ViewportShape::Rectangular,
};
const BMM101: OverlayViewport = OverlayViewport {
    width: 480,
    height: 320,
    shape: ViewportShape::Rectangular,
};
const DECK_CAPS: SettingsCaps = SettingsCaps {
    brightness: true,
    sound: true,
    wifi_setup: true,
};
const BMM101_CAPS: SettingsCaps = SettingsCaps {
    brightness: true,
    sound: false,
    wifi_setup: false,
};

/// Records circle fills at absolute coordinates, the box width every
/// paragraph is drawn into and the absolute bottom edge it reaches;
/// text measurement wraps.
#[derive(Default)]
struct ProbeRenderer {
    offset: (f32, f32),
    saved: Vec<(f32, f32)>,
    circles: Vec<(f32, f32, f32)>,
    paragraphs: Vec<(String, f32)>,
    paragraph_bottoms: Vec<(String, f32)>,
    next_svg: u16,
}

impl ProbeRenderer {
    fn record_circle(&mut self, cx: f32, cy: f32, r: f32) {
        self.circles
            .push((cx + self.offset.0, cy + self.offset.1, r));
    }
}

impl Renderer for ProbeRenderer {
    fn fill_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _color: Color) {}
    fn fill_rounded_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _r: f32, _color: Color) {}
    fn stroke_rounded_rect(
        &mut self,
        _x: f32,
        _y: f32,
        _w: f32,
        _h: f32,
        _r: f32,
        _border_width: f32,
        _color: Color,
    ) {
    }
    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, _color: Color) {
        self.record_circle(cx, cy, r);
    }
    fn fill_rect_paint(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _fill: &Fill) {}
    fn fill_circle_paint(&mut self, cx: f32, cy: f32, r: f32, _fill: &Fill) {
        self.record_circle(cx, cy, r);
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
    fn stroke_rect(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _bw: f32, _color: Color) {}
    fn draw_line(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _w: f32, _color: Color) {}
    fn save(&mut self) {
        self.saved.push(self.offset);
    }
    fn restore(&mut self) {
        if let Some(prev) = self.saved.pop() {
            self.offset = prev;
        }
    }
    fn translate(&mut self, x: f32, y: f32) {
        self.offset.0 += x;
        self.offset.1 += y;
    }
    fn rotate(&mut self, _angle_radians: f32) {}
    fn push_scissor(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) {}
    fn pop_scissor(&mut self) {}
    fn brightness(&self) -> f32 {
        1.0
    }
    fn set_brightness(&mut self, _brightness: f32) {}
    fn draw_text(&mut self, _text: &str, _x: f32, _y: f32, _size: f32, _color: Color) {}
    fn measure_text(&mut self, text: &str, size: f32) -> f32 {
        text.chars().count() as f32 * size * 0.6
    }
    fn measure_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
    ) -> (f32, f32) {
        // Greedy word wrap like a real text backend: lines break at
        // whitespace, and a word wider than the limit degenerates into
        // per-glyph lines. Concatenating the spans first keeps a word
        // split across styled spans as one word.
        let char_w = style.size as f32 * 0.6;
        let line_h = style.size as f32 * style.line_height;
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        if style.text_overflow != TextOverflow::Wrap {
            let lines = text.split('\n');
            let line_count = lines.clone().count() as f32;
            let widest = lines
                .map(|line| line.chars().count() as f32 * char_w)
                .fold(0.0, f32::max);
            let width = match (style.text_overflow, max_width) {
                (TextOverflow::Ellipsis, Some(limit)) if limit < widest => limit.max(char_w),
                _ => widest,
            };
            return (width, line_count * line_h);
        }
        let word_chars: Vec<f32> = text
            .split_whitespace()
            .map(|w| w.chars().count() as f32)
            .collect();
        let Some(natural_chars) = word_chars
            .iter()
            .copied()
            .reduce(|total, w| total + 1.0 + w)
        else {
            return (0.0, line_h);
        };
        let natural_w = natural_chars * char_w;
        let Some(limit) = max_width.filter(|w| *w < natural_w) else {
            return (natural_w, line_h);
        };
        let per_line = (limit / char_w).floor().max(1.0);
        let mut lines = 0.0_f32;
        let mut line = 0.0_f32;
        let mut widest = 0.0_f32;
        let flush = |lines: &mut f32, line: &mut f32, widest: &mut f32| {
            if *line > 0.0 {
                *lines += 1.0;
                *widest = widest.max(*line);
                *line = 0.0;
            }
        };
        for wc in word_chars {
            if wc > per_line {
                flush(&mut lines, &mut line, &mut widest);
                lines += (wc / per_line).ceil();
                widest = widest.max(per_line);
            } else if line == 0.0 {
                line = wc;
            } else if line + 1.0 + wc <= per_line {
                line += 1.0 + wc;
            } else {
                flush(&mut lines, &mut line, &mut widest);
                line = wc;
            }
        }
        flush(&mut lines, &mut line, &mut widest);
        (widest * char_w, lines * line_h)
    }
    fn draw_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        _x: f32,
        y: f32,
        max_width: f32,
    ) {
        let (_, height) = self.measure_paragraph(style, spans, Some(max_width));
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        self.paragraph_bottoms
            .push((text.clone(), self.offset.1 + y + height));
        self.paragraphs.push((text, max_width));
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
        self.next_svg += 1;
        SvgId::from_wire(self.next_svg)
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
        _inner: &mut dyn FnMut(&mut dyn Renderer),
    ) {
    }
    fn begin_frame(&mut self, _width: u32, _height: u32, _dpi_scale: f32) {}
    fn begin_frame_with_clear(
        &mut self,
        _width: u32,
        _height: u32,
        _dpi_scale: f32,
        _clear: FrameClear,
    ) {
    }
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
    fn bitmap_resident_bytes(&self) -> u64 {
        0
    }
}

#[test]
fn large_tier_controls_sit_in_the_bottom_half() {
    assert_controls_in_the_bottom_half("braiins-deck", "Braiins-WiFi");
}

#[test]
fn a_long_hostname_and_ssid_keep_the_controls_in_the_bottom_half() {
    assert_controls_in_the_bottom_half(&"braiins-deck-".repeat(6), &"a-network-name-".repeat(5));
}

fn assert_controls_in_the_bottom_half(hostname: &str, ssid: &str) {
    let mut view = SettingsTrayView::resting().with_caps(Some(DECK_CAPS));
    view.hostname = Some(hostname.to_owned());
    view.ip = Some("192.168.1.42".to_owned());
    view.wifi_signal = Some(-52);
    view.ssid = Some(ssid.to_owned());

    let now = Instant::now();
    let mut state = SettingsTrayRenderState::new(now);
    let mut renderer = ProbeRenderer::default();
    render_settings_tray(&mut renderer, DECK, &mut state, &view, now);

    assert!(
        !renderer.circles.is_empty(),
        "the Large tier must render round control buttons"
    );
    // The probe's coarse text metrics shift the flow by a few pixels, so
    // assert the circle centers (not edges) against the middle: the collapsed
    // layout put them a full hundred pixels above it.
    let middle = DECK.height as f32 / 2.0;
    for (cx, cy, r) in &renderer.circles {
        assert!(
            *cy >= middle,
            "control circle at ({cx}, {cy}) r={r} must center below the \
             panel middle {middle}"
        );
        assert!(
            cy + r <= DECK.height as f32 + 1e-3,
            "control circle at ({cx}, {cy}) r={r} must stay on the panel"
        );
    }
}

/// Bottom edge of BMM101's close target: the compact tier's 16 px corner inset
/// plus the 48 px square.
const BMM101_CLOSE_BOTTOM: f32 = 64.0;

/// Radius of a BMM101 control button; the slider thumb is a 16 px circle.
const BMM101_BUTTON_RADIUS: f32 = 32.0;

/// The compact stack is fixed-height from the address table down,
/// so the buttons have nowhere to give: they either fit under the close target
/// and above the panel's bottom edge, or they run off it.
/// The captions under them are what runs off first, so every paragraph is checked too.
#[test]
fn bmm101_controls_fit_between_the_close_target_and_the_bottom_edge() {
    for wifi_setup in [false, true] {
        let caps = SettingsCaps {
            wifi_setup,
            ..BMM101_CAPS
        };
        let mut view = SettingsTrayView::resting().with_caps(Some(caps));
        view.hostname = Some("braiins-mini".to_owned());
        view.ip = Some("10.0.0.42".to_owned());
        view.wifi_signal = Some(-52);
        view.ssid = Some("Workshop-WiFi".to_owned());

        let now = Instant::now();
        let mut state = SettingsTrayRenderState::new(now);
        let mut renderer = ProbeRenderer::default();
        render_settings_tray(&mut renderer, BMM101, &mut state, &view, now);

        let buttons: Vec<_> = renderer
            .circles
            .iter()
            .filter(|(_, _, r)| (*r - BMM101_BUTTON_RADIUS).abs() < 1e-3)
            .collect();
        assert!(
            !buttons.is_empty(),
            "wifi_setup={wifi_setup}: no control buttons"
        );
        let panel_bottom = BMM101.height as f32;
        for (cx, cy, r) in buttons {
            assert!(
                cy - r >= BMM101_CLOSE_BOTTOM - 1e-3,
                "wifi_setup={wifi_setup}: button at ({cx}, {cy}) starts above the \
                 close target's bottom edge {BMM101_CLOSE_BOTTOM}"
            );
            assert!(
                cy + r <= panel_bottom + 1e-3,
                "wifi_setup={wifi_setup}: button at ({cx}, {cy}) runs past the \
                 panel's {panel_bottom} px"
            );
        }
        for (text, bottom) in &renderer.paragraph_bottoms {
            assert!(
                *bottom <= panel_bottom + 1e-3,
                "wifi_setup={wifi_setup}: {text:?} ends at {bottom}, past the \
                 panel's {panel_bottom} px"
            );
        }
    }
}

#[test]
fn large_tier_hold_circle_is_centered_on_its_button() {
    let mut view = SettingsTrayView::resting().with_caps(Some(DECK_CAPS));
    view.status = Some(StatusView {
        action: Action::Restart,
        phase: Phase::Holding { progress: 0.5 },
        reason: None,
    });

    let now = Instant::now();
    let mut state = SettingsTrayRenderState::new(now);
    let mut renderer = ProbeRenderer::default();
    render_settings_tray(&mut renderer, DECK, &mut state, &view, now);

    let &(hold_x, hold_y, _) = renderer
        .circles
        .iter()
        .find(|(_, _, r)| (*r - 84.0).abs() < 1e-3)
        .expect("BUG: a half-held Large-tier button must draw an 84px circle");
    assert!(
        renderer.circles.iter().any(|(x, y, r)| {
            (*r - 56.0).abs() < 1e-3 && (*x - hold_x).abs() < 1e-3 && (*y - hold_y).abs() < 1e-3
        }),
        "the hold circle must share its center with the held button",
    );
}

#[test]
fn bmm101_cuts_a_long_value_and_keeps_every_label_whole() {
    let mut view = SettingsTrayView::resting().with_caps(Some(BMM101_CAPS));
    view.hostname = Some("braiins-mini-".repeat(4));
    view.ip = Some("10.0.0.42".to_owned());
    view.wifi_signal = Some(-52);
    view.ssid = Some("a-network-name-".repeat(3));

    let now = Instant::now();
    let mut state = SettingsTrayRenderState::new(now);
    let mut renderer = ProbeRenderer::default();
    render_settings_tray(&mut renderer, BMM101, &mut state, &view, now);

    for label in ["Hostname", "IP Address", "Connection"] {
        let (_, width) = renderer
            .paragraphs
            .iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("BUG: the address table labels {label}"));
        let natural = label.chars().count() as f32 * 16.0 * 0.6;
        // Taffy lays out on whole pixels.
        assert!(
            *width >= natural - 1.0,
            "{label} was squeezed to {width}px of its {natural}px, so it wraps"
        );
    }
}

#[test]
fn the_layout_follows_the_size_the_tray_renders_at() {
    let view = SettingsTrayView::resting().with_caps(Some(DECK_CAPS));
    let labels_at = |size| {
        let now = Instant::now();
        let mut state = SettingsTrayRenderState::new(now);
        let mut renderer = ProbeRenderer::default();
        render_settings_tray(&mut renderer, size, &mut state, &view, now);
        renderer
            .paragraphs
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>()
    };

    let wide = labels_at(DECK);
    assert!(
        !wide.iter().any(|t| t == "Connection"),
        "{DECK:?} lays out wide: {wide:?}"
    );
    let compact = labels_at(BMM101);
    assert!(
        compact.iter().any(|t| t == "Connection"),
        "{BMM101:?} lays out compact: {compact:?}"
    );
}
