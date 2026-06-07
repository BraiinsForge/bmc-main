// Copyright (C) 2026  Braiins Systems s.r.o.

use std::time::Instant;

use bmc_render::renderer::Renderer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;

/// A touch event delivered to an overlay (logical coordinates within the surface).
#[derive(Debug, Clone, Copy)]
pub enum TouchEvent {
    Down { id: i32, x: f64, y: f64 },
    Motion { id: i32, x: f64, y: f64 },
    Up { id: i32 },
    Cancel,
}

/// What region of the surface accepts touch input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRegion {
    /// Whole surface accepts input (the layer-shell default).
    Full,
    /// Surface accepts no input; touches fall through to what is behind it.
    None,
}

/// Static layer-surface configuration, applied once at map time.
#[derive(Debug, Clone)]
pub struct LayerConfig {
    pub layer: Layer,
    pub anchor: Anchor,
    /// Requested size in logical pixels. A zero axis with both opposite anchors
    /// set asks the compositor to stretch that axis.
    pub size: (u32, u32),
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    /// Pixels of output edge the surface reserves (layer-shell exclusive zone).
    /// `0` reserves nothing — correct for every overlay here (fullscreen
    /// blocker, passive corner indicator, top panel). Kept as a knob because
    /// the spec names it as framework plumbing.
    pub exclusive_zone: i32,
    pub namespace: String,
    pub input: InputRegion,
}

impl LayerConfig {
    /// A fullscreen overlay anchored to all four edges.
    #[must_use]
    pub fn fullscreen(namespace: impl Into<String>) -> Self {
        Self {
            layer: Layer::Overlay,
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            size: (0, 0),
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            exclusive_zone: 0,
            namespace: namespace.into(),
            input: InputRegion::Full,
        }
    }
}

/// Result of an overlay's per-pass background work.
#[derive(Debug, Clone, Copy, Default)]
pub struct TickOutcome {
    /// The overlay's content changed and it wants to be rendered this pass.
    pub wants_render: bool,
    /// Earliest instant the overlay wants to be ticked again (for non-event
    /// driven work such as a clock). `None` means "only on external events".
    pub next_wake: Option<Instant>,
}

/// A privileged system overlay. Implementors do background work in `tick`,
/// draw in `render`, and declare placement via `layer_config`.
pub trait SystemOverlay {
    /// Called once before the first render.
    fn init(&mut self) {}

    /// Static placement and input policy.
    fn layer_config(&self) -> LayerConfig;

    /// Per-pass background work. Return whether a render is wanted and when to
    /// wake next. Must not block.
    fn tick(&mut self, now: Instant) -> TickOutcome;

    /// Draw the overlay. `size` is the surface size in logical pixels. The
    /// `&mut dyn Renderer` is valid only for this call: do not store it.
    fn render(&mut self, renderer: &mut dyn Renderer, size: (u32, u32));

    /// Handle a touch event (only delivered when input region is not `None`).
    fn on_touch(&mut self, _event: TouchEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_config_anchors_all_edges() {
        let c = LayerConfig::fullscreen("test");
        assert!(c.anchor.contains(Anchor::Top));
        assert!(c.anchor.contains(Anchor::Bottom));
        assert!(c.anchor.contains(Anchor::Left));
        assert!(c.anchor.contains(Anchor::Right));
        assert_eq!(c.size, (0, 0));
        assert_eq!(c.input, InputRegion::Full);
    }
}
