// Copyright (C) 2026  Braiins Systems s.r.o.

use std::time::Instant;

use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;

use crate::overlay::{InputRegion, LayerConfig, SystemOverlay, TickOutcome, TouchEvent};

#[derive(Debug, Default)]
pub struct ValidationOverlay {
    // Content is static, so request exactly one render and then stay idle.
    rendered: bool,
}

impl SystemOverlay for ValidationOverlay {
    fn layer_config(&self) -> LayerConfig {
        LayerConfig {
            layer: Layer::Overlay,
            anchor: Anchor::Bottom | Anchor::Right,
            size: (420, 180),
            margin_top: 0,
            margin_right: 24,
            margin_bottom: 24,
            margin_left: 0,
            exclusive_zone: 0,
            namespace: "bmc-validation-partial".to_owned(),
            input: InputRegion::Full,
        }
    }

    fn tick(&mut self, _now: Instant) -> TickOutcome {
        // Render once; a later resize sets surface-dirty separately, so this
        // does not need to keep asking. No periodic wake.
        TickOutcome {
            visible: true,
            wants_render: !self.rendered,
            next_wake: None,
        }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        // Half-transparent green wash + an opaque marker box, to prove alpha
        // compositing over the live scene.
        #[expect(
            clippy::cast_precision_loss,
            reason = "display dimensions fit comfortably in f32 mantissa"
        )]
        let (w, h) = (size.0 as f32, size.1 as f32);
        r.fill_rect(0.0, 0.0, w, h, Color::from_rgba(0, 200, 0, 128));
        r.fill_rect(
            40.0,
            40.0,
            200.0,
            120.0,
            Color::from_rgba(255, 255, 255, 255),
        );
        r.draw_text(
            "partial overlay OK",
            56.0,
            96.0,
            28.0,
            Color::from_rgba(0, 0, 0, 255),
        );
        self.rendered = true;
    }

    fn on_touch(&mut self, event: TouchEvent) {
        eprintln!("validation-overlay touch: {event:?}");
    }
}
