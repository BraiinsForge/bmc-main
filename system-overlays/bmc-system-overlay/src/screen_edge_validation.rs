// Copyright (C) 2026  Braiins Systems s.r.o.

//! Throwaway top-edge verification overlay. Arms the top edge, draws a marker on
//! reveal, and dismisses on tap. Removed when the real swipe panel lands.

use std::time::Instant;

use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;

use crate::overlay::{
    InputRegion, LayerConfig, ScreenEdge, SystemOverlay, TickOutcome, TouchEvent,
};

/// Panel height in logical pixels (top strip, full width).
const PANEL_HEIGHT: u32 = 200;
const LABEL: &str = "screen edge OK - tap to dismiss";

#[derive(Debug, Default)]
pub struct ScreenEdgeValidationOverlay {
    /// True from reveal until a tap dismisses it.
    showing: bool,
    /// Whether the current showing has been drawn at least once.
    rendered: bool,
}

impl SystemOverlay for ScreenEdgeValidationOverlay {
    fn layer_config(&self) -> LayerConfig {
        LayerConfig {
            layer: Layer::Overlay,
            anchor: Anchor::Top | Anchor::Left | Anchor::Right,
            size: (0, PANEL_HEIGHT),
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            exclusive_zone: 0,
            namespace: "bmc-screen-edge-validation".to_owned(),
            input: InputRegion::Full,
        }
    }

    fn tick(&mut self, _now: Instant) -> TickOutcome {
        TickOutcome {
            visible: self.showing,
            wants_render: self.showing && !self.rendered,
            next_wake: None,
        }
    }

    fn render(&mut self, renderer: &mut dyn Renderer, size: (u32, u32)) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "panel dimensions fit comfortably in f32 mantissa"
        )]
        let (w, h) = (size.0 as f32, size.1 as f32);
        renderer.fill_rect(0.0, 0.0, w, h, Color::from_rgba(20, 40, 120, 200));

        let font = 30.0;
        let text_width = renderer.measure_text(LABEL, font);
        renderer.draw_text(
            LABEL,
            (w - text_width) / 2.0,
            h / 2.0 + font / 3.0,
            font,
            Color::from_rgba(255, 255, 255, 255),
        );
        self.rendered = true;
    }

    fn on_touch(&mut self, event: TouchEvent) {
        if matches!(event, TouchEvent::Down { .. }) {
            self.showing = false;
        }
    }

    fn screen_edge(&self) -> Option<ScreenEdge> {
        Some(ScreenEdge::Top)
    }

    fn on_reveal(&mut self) {
        self.showing = true;
        self.rendered = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_render::gpu::mesh::MeshDrawArgs;
    use bmc_render::tree::{SpanData, TextStyle};
    use bmc_wasm_protocol::{
        ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, BitmapId, Fill, MeshId, SvgId,
    };

    #[derive(Default)]
    struct TestRenderer {
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

        fn draw_line(
            &mut self,
            _x1: f32,
            _y1: f32,
            _x2: f32,
            _y2: f32,
            _width: f32,
            _color: Color,
        ) {
        }

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

        fn bitmap_sample(
            &self,
            _bitmap_id: BitmapId,
            _x: u32,
            _y: u32,
            _w: u32,
            _h: u32,
        ) -> Option<Color> {
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

    #[test]
    fn hidden_until_revealed_then_dismissed_by_tap() {
        let now = Instant::now();
        let mut overlay = ScreenEdgeValidationOverlay::default();
        let hidden = overlay.tick(now);

        assert!(!hidden.visible);
        assert!(!hidden.wants_render);
        assert_eq!(hidden.next_wake, None);

        overlay.on_reveal();
        let revealed = overlay.tick(now);

        assert!(revealed.visible);
        assert!(revealed.wants_render);
        assert_eq!(revealed.next_wake, None);

        let mut renderer = TestRenderer::default();
        overlay.render(&mut renderer, (480, PANEL_HEIGHT));
        assert!(renderer.filled);
        assert_eq!(renderer.text.as_deref(), Some(LABEL));

        let rendered = overlay.tick(now);

        assert!(rendered.visible);
        assert!(!rendered.wants_render);
        assert_eq!(rendered.next_wake, None);

        overlay.on_touch(TouchEvent::Down {
            id: 0,
            x: 1.0,
            y: 1.0,
        });
        let dismissed = overlay.tick(now);

        assert!(!dismissed.visible);
        assert!(!dismissed.wants_render);
        assert_eq!(dismissed.next_wake, None);
    }

    #[test]
    fn arms_the_top_edge_and_uses_requested_layer_config() {
        let overlay = ScreenEdgeValidationOverlay::default();
        let config = overlay.layer_config();

        assert_eq!(overlay.screen_edge(), Some(ScreenEdge::Top));
        assert_eq!(config.layer, Layer::Overlay);
        assert!(config.anchor.contains(Anchor::Top));
        assert!(config.anchor.contains(Anchor::Left));
        assert!(config.anchor.contains(Anchor::Right));
        assert!(!config.anchor.contains(Anchor::Bottom));
        assert_eq!(config.size, (0, PANEL_HEIGHT));
        assert_eq!(config.margin_top, 0);
        assert_eq!(config.margin_right, 0);
        assert_eq!(config.margin_bottom, 0);
        assert_eq!(config.margin_left, 0);
        assert_eq!(config.exclusive_zone, 0);
        assert_eq!(config.namespace, "bmc-screen-edge-validation");
        assert_eq!(config.input, InputRegion::Full);
    }
}
