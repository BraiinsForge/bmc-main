// Copyright (C) 2026  Braiins Systems s.r.o.

use std::time::{Duration, Instant};

use bmc_render::renderer::Renderer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;

/// Minimum wall-clock gap between two submitted frames. Both the standalone
/// loop and the hosted driver enforce this floor identically.
pub(crate) const MIN_INTER_FRAME: Duration = Duration::from_millis(8);

#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a distinct render-gate predicate; a flags enum would be less readable at the single call site"
)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct RenderGate {
    pub failed: bool,
    pub visible: bool,
    pub mapped: bool,
    pub wants_render: bool,
    pub inter_frame_ok: bool,
    pub client_running: bool,
    pub target_available: bool,
}

#[must_use]
pub(crate) fn overlay_needs_render(gate: RenderGate) -> bool {
    let wants = gate.wants_render || (gate.visible && !gate.mapped);
    !gate.failed
        && gate.visible
        && wants
        && gate.inter_frame_ok
        && gate.client_running
        && gate.target_available
}

#[must_use]
pub(crate) fn overlay_needs_hide(mapped: bool, visible: bool) -> bool {
    mapped && !visible
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResizeTransition {
    pub unmap_before_resize: bool,
    pub mapped_after_resize: bool,
}

#[must_use]
pub(crate) fn resize_transition(mapped: bool) -> ResizeTransition {
    ResizeTransition {
        unmap_before_resize: mapped,
        mapped_after_resize: false,
    }
}

/// Whether a screen-edge overlay is currently visible. An overlay armed to a
/// screen edge is only shown when both the compositor has revealed it (`revealed`)
/// and the overlay's own tick says it wants to be on screen (`overlay_visible`).
#[must_use]
pub(crate) fn screen_edge_visible(revealed: bool, overlay_visible: bool) -> bool {
    revealed && overlay_visible
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a distinct wake-gate predicate; a flags enum would be less readable at the single call site"
)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct PollGate {
    pub failed: bool,
    pub visible: bool,
    pub wants_render: bool,
    pub client_running: bool,
    pub target_available: bool,
}

/// Max time the host may sleep for this overlay. `next_wake` is the tick-based
/// wake (already converted to a remaining `Duration`); `inter_frame_remaining`
/// is `Some` while the 8 ms frame floor has time left.
///
/// The wake decision must agree with `overlay_needs_render`: while invisible
/// (or otherwise non-rendering) a latched `wants_render` must not request an
/// immediate wake, or the host busy-spins on a frame that never renders.
#[must_use]
pub(crate) fn overlay_poll_timeout(
    gate: PollGate,
    next_wake: Option<Duration>,
    inter_frame_remaining: Option<Duration>,
) -> Option<Duration> {
    if gate.failed || !gate.wants_render || !gate.visible || !gate.client_running {
        return next_wake;
    }
    match inter_frame_remaining {
        Some(d) => Some(next_wake.map_or(d, |t| d.min(t))),
        None if gate.target_available => Some(Duration::ZERO),
        None => next_wake,
    }
}

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
/// Top and bottom edges are supported. Left/right edges are omitted because
/// horizontal gestures conflict with scene navigation, which can begin
/// anywhere, including at a screen edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenEdge {
    Top,
    Bottom,
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

    /// Pay one-time renderer setup costs at host startup instead of on the
    /// first reveal. The host calls this once, before the event loop, with the
    /// GL context current. Use it to register SVG icons and warm font glyphs so
    /// the first swipe-reveal does not stall. Default: no-op. The `&mut dyn
    /// Renderer` is valid only for this call: do not store it.
    fn prewarm(&mut self, _renderer: &mut dyn Renderer) {}

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

    /// Whether this overlay presents slide frames through the panel cache.
    /// Gates cache captures so overlays that never blit don't allocate one.
    /// Default: `false`.
    fn uses_panel_cache(&self) -> bool {
        false
    }

    /// Take and clear the overlay's content-changed flag. The host calls this
    /// after a full paint to learn whether the just-painted frame must refresh
    /// the cached panel source. Default: `false` (overlays without a cache never
    /// signal dirty, so the host always treats their paints as authoritative).
    fn take_content_dirty(&mut self) -> bool {
        false
    }

    /// Whether the content-changed flag is currently set, without consuming
    /// it. The host polls this to decide background cache refreshes while
    /// hidden; polling never consumes the flag — only `take_content_dirty`
    /// does. Default: `false` (overlays without a cache never report dirty).
    fn content_dirty(&self) -> bool {
        false
    }

    /// Force the overlay's content-changed flag so the next full paint
    /// refreshes the cached panel. No-op default: overlays that never cache
    /// (offline, device-info) need not implement this.
    fn mark_content_dirty(&mut self) {}

    /// A frame of this overlay was just submitted to the compositor. Called
    /// with a timestamp taken after the submit, so time-anchored animations
    /// anchor at the hand-off rather than at the trigger that requested the
    /// frame. Default: no-op.
    fn on_frame_submitted(&mut self, _now: Instant) {}
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

    fn runnable_gate(visible: bool, mapped: bool, wants_render: bool) -> RenderGate {
        RenderGate {
            failed: false,
            visible,
            mapped,
            wants_render,
            inter_frame_ok: true,
            client_running: true,
            target_available: true,
        }
    }

    #[test]
    fn first_show_renders_without_dirty_flag() {
        assert!(overlay_needs_render(runnable_gate(true, false, false)));
    }

    #[test]
    fn hidden_ignores_latched_render_request() {
        assert!(!overlay_needs_render(runnable_gate(false, false, true)));
    }

    #[test]
    fn mapped_but_invisible_needs_hide() {
        assert!(overlay_needs_hide(true, false));
    }

    #[test]
    fn mapped_resize_unmaps_before_destroying_buffers() {
        assert_eq!(
            resize_transition(true),
            ResizeTransition {
                unmap_before_resize: true,
                mapped_after_resize: false,
            }
        );
        assert_eq!(
            resize_transition(false),
            ResizeTransition {
                unmap_before_resize: false,
                mapped_after_resize: false,
            }
        );
    }

    #[test]
    fn screen_edge_overlay_visible_only_while_revealed() {
        assert!(
            !screen_edge_visible(false, true),
            "armed-but-hidden stays unmapped"
        );
        assert!(screen_edge_visible(true, true), "revealed and wanted maps");
        assert!(
            !screen_edge_visible(true, false),
            "dismissed while revealed unmaps"
        );
    }

    #[test]
    fn throttled_first_show_waits_for_frame_floor() {
        let mut gate = runnable_gate(true, false, false);
        gate.inter_frame_ok = false;
        assert!(!overlay_needs_render(gate));
    }

    fn runnable_poll_gate(visible: bool, wants_render: bool) -> PollGate {
        PollGate {
            failed: false,
            visible,
            wants_render,
            client_running: true,
            target_available: true,
        }
    }

    #[test]
    fn invisible_overlay_with_latched_render_does_not_busy_spin() {
        let gate = runnable_poll_gate(false, true);
        assert_eq!(
            overlay_poll_timeout(gate, Some(Duration::from_secs(2)), None),
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn renderable_overlay_polls_immediately() {
        let gate = runnable_poll_gate(true, true);
        assert_eq!(overlay_poll_timeout(gate, None, None), Some(Duration::ZERO));
    }

    #[test]
    fn renderable_overlay_waits_for_inter_frame_floor() {
        let gate = runnable_poll_gate(true, true);
        assert_eq!(
            overlay_poll_timeout(gate, None, Some(Duration::from_millis(5))),
            Some(Duration::from_millis(5))
        );
    }
}
