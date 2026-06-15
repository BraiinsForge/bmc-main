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

/// A screen edge an overlay can arm for swipe-reveal. Opt in via
/// [`SystemOverlay::screen_edge`].
///
/// Only the top edge is offered: a top reveal is a downward gesture, orthogonal
/// to the horizontal scene swipe. Bottom and left/right are unsupported —
/// left/right would be horizontal gestures that conflict with scene navigation
/// (which can begin anywhere, including at a screen edge), and no Stage-3 overlay
/// needs the bottom edge. Kept as an enum so a future edge needs no API change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenEdge {
    Top,
}

/// A control request an overlay wants to send over `deck_settings_v1`. The
/// framework drains these after `tick` and forwards them to the compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRequest {
    SetBrightness(u8),
    ReconfigureWifi,
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
            layer: Layer::Top,
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

    /// A small overlay pinned to the bottom-right corner with no input region,
    /// on the `Bottom` layer so a fullscreen `Top`/`Overlay` surface occludes it.
    #[must_use]
    pub fn bottom_right(namespace: impl Into<String>, size: (u32, u32)) -> Self {
        Self {
            layer: Layer::Bottom,
            anchor: Anchor::Bottom | Anchor::Right,
            size,
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            exclusive_zone: 0,
            namespace: namespace.into(),
            input: InputRegion::None,
        }
    }
}

#[must_use]
pub(crate) fn resolved_configured_size(
    config_size: (u32, u32),
    configured_size: (u32, u32),
) -> (u32, u32) {
    (
        if configured_size.0 == 0 {
            config_size.0.max(1)
        } else {
            configured_size.0
        },
        if configured_size.1 == 0 {
            config_size.1.max(1)
        } else {
            configured_size.1
        },
    )
}

/// Result of an overlay's per-pass background work.
#[derive(Debug, Clone, Copy, Default)]
pub struct TickOutcome {
    /// Whether the overlay wants to be on-screen. When `false` the framework
    /// unmaps the surface (NULL buffer) and frees its export buffers; when it
    /// flips back to `true` the framework reallocates and renders a fresh frame.
    pub visible: bool,
    /// The overlay's content changed and it wants a redraw this pass. Ignored
    /// while `visible` is `false`.
    pub wants_render: bool,
    /// Earliest instant the overlay wants to be ticked again. `None` means
    /// "only on external events".
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

    /// Opt in to screen-edge reveal. `None` (default) means a normal overlay
    /// whose visibility is driven by [`TickOutcome::visible`]. `Some(edge)` arms
    /// that edge at startup: the surface stays hidden (no buffer) until the
    /// compositor reveals it on the edge gesture, and re-arms on hide.
    fn screen_edge(&self) -> Option<ScreenEdge> {
        None
    }

    /// Called once each time the compositor reveals the overlay's armed edge,
    /// before the first frame of that reveal. Use it to reset per-reveal state.
    fn on_reveal(&mut self) {}

    /// Whether this overlay binds the `deck_settings_v1` control channel. `false`
    /// (default) means the framework neither binds it nor delivers settings
    /// events to this overlay.
    fn uses_settings(&self) -> bool {
        false
    }

    /// Effective display brightness (0-100) reported by the compositor. Called
    /// before `tick` when a `brightness` event arrived. Default: no-op.
    fn on_brightness(&mut self, _value: u8) {}

    /// WiFi setup-AP SSID reported by the compositor: `Some(ssid)` while setup
    /// mode is active, `None` when inactive. Called before `tick`. Default:
    /// no-op.
    fn on_wifi_ap(&mut self, _ssid: Option<&str>) {}

    /// Drain control requests the overlay wants to send this pass. Called after
    /// `tick`. Default: none.
    fn drain_settings_requests(&mut self) -> Vec<SettingsRequest> {
        Vec::new()
    }

    /// While a blit-only animation is running with unchanged content, the panel
    /// offset (px) the host should blit the cached panel at this frame; `None`
    /// when the host must full-paint (content changed or no animation).
    ///
    /// Lets a screen-edge overlay slide by re-blitting a once-painted GPU cache
    /// instead of re-laying-out and repainting every frame. Default: `None`
    /// (every frame full-paints), so non-animating overlays are unaffected.
    fn wants_cached_blit(&self, _now: Instant) -> Option<f32> {
        None
    }

    /// Take and clear the overlay's content-changed flag. The host calls this
    /// after a full paint to learn whether the just-painted frame must refresh
    /// the cached panel source. Default: `false` (overlays without a cache never
    /// signal dirty, so the host always treats their paints as authoritative).
    fn take_content_dirty(&mut self) -> bool {
        false
    }
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

    #[test]
    fn resolved_configured_size_falls_back_when_compositor_reports_zero_axis() {
        assert_eq!(resolved_configured_size((420, 180), (0, 180)), (420, 180));
        assert_eq!(resolved_configured_size((420, 180), (420, 0)), (420, 180));
        assert_eq!(resolved_configured_size((0, 0), (0, 0)), (1, 1));
    }
}
