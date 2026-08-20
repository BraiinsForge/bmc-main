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

/// The presentation fence as `hide` sees it this pass: not yet requested,
/// or armed with its deadline either still ahead or already behind now.
#[derive(Debug, Clone, Copy)]
pub(crate) enum FenceState {
    Unarmed,
    Armed { deadline_passed: bool },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HideFenceGate {
    pub fence: FenceState,
    pub frame_presented: bool,
    pub client_running: bool,
}

/// What `HostedOverlay::hide` should do this pass for its non-blocking
/// presentation fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HideFenceAction {
    /// No fence pending yet: request one and wait for a later pass.
    Arm,
    /// Fence pending, resolved by neither the callback nor the deadline:
    /// keep waiting.
    Wait,
    /// Run the unmap sequence now. `timed_out` is true only when the
    /// deadline forced it — worth a warning — as opposed to a presented
    /// frame or a dead client resolving it normally.
    Unmap { timed_out: bool },
}

#[must_use]
pub(crate) fn hide_fence_action(gate: HideFenceGate) -> HideFenceAction {
    let FenceState::Armed { deadline_passed } = gate.fence else {
        return HideFenceAction::Arm;
    };
    if gate.frame_presented || !gate.client_running {
        return HideFenceAction::Unmap { timed_out: false };
    }
    if deadline_passed {
        return HideFenceAction::Unmap { timed_out: true };
    }
    HideFenceAction::Wait
}

/// Stale-fence guard for `tick`: a fence only makes sense while the overlay
/// is on its way out, so a re-reveal drops it rather than letting a later
/// callback or deadline resolve a hide that never happened.
#[must_use]
pub(crate) fn hide_fence_after_tick(visible: bool, hide_fence: Option<Instant>) -> Option<Instant> {
    if visible { None } else { hide_fence }
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
    SetVolume(u8),
    ToggleNightMode,
    Restart,
    ReconfigureWifi,
}

/// Decoded `deck_settings_v1` capability set: which optional controls this
/// compositor supports. Plain bools so overlays never touch the raw protocol
/// bitfield.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsCaps {
    pub brightness: bool,
    pub sound: bool,
    pub wifi_setup: bool,
}

/// A control request an overlay wants to send over `deck_alarm_v1`. The
/// framework drains these after `tick` and forwards them to the compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]

pub enum AlarmRequest {
    Dismiss,
    Snooze,
}

/// An incoming `deck_alarm_v1` event. The client collapses ring/stop into a
/// single latest-wins slot so a `stop` then `ring` arriving in one dispatch
/// round keeps the ring rather than losing it to the trailing stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlarmEvent {
    Ring {
        time: String,
        period: String,
        label: String,
        snooze_allowed: bool,
    },
    Stop,
}

/// Device lifecycle state reported over `deck_device_info_v1`; mirrors bmc's
/// `BmcState`. Plain enum so overlays never touch the wire type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    FactoryDefault,
    SetupPending,
    Operational,
    WifiReconfiguration,
}

/// Setup-flow step reported over `deck_device_info_v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupStep {
    Idle,
    ConnectingToWifi,
    WifiConnectionSuccess,
    WifiConnectionFailed,
    WifiReconfigSuccess,
    DeviceSetupSuccess,
    /// Setup cannot continue. `restarting` says whether bmc resolves it
    /// by restarting or resetting the device, which decides
    /// whether the screen waits it out or asks the user to act.
    UnexpectedError {
        restarting: bool,
    },
}

/// Setup access point reported over `deck_device_info_v1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPoint {
    pub ssid: String,
    /// Address the setup wizard is reached at, e.g. `http://10.0.0.21/`.
    pub setup_url: String,
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
    /// on the `Background` layer — the lowest rank, so every other overlay draws
    /// over it. Background surfaces still paint above the scene, so the indicator
    /// stays visible over the clock; being passive, it never blocks scene input.
    #[must_use]
    pub fn bottom_right(namespace: impl Into<String>, size: (u32, u32)) -> Self {
        Self {
            layer: Layer::Background,
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

/// Presentation type of an upgrade overlay (the `deck_upgrade_v1` wire enum).
pub use ::deck_upgrade_v1::client::deck_upgrade_v1::Kind as UpgradeKind;
/// Upgrade stage supplied by the compositor (the `deck_upgrade_v1` wire enum).
pub use ::deck_upgrade_v1::client::deck_upgrade_v1::Phase as UpgradePhase;
pub use ::deck_upgrade_v1::{DownloadProgress, UpgradeSnapshot, UpgradeState};

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

    /// Whether this overlay binds the compositor upgrade-state protocol.
    fn uses_upgrade(&self) -> bool {
        false
    }
    /// Receive one coherent compositor upgrade snapshot before `tick`.
    fn on_upgrade_state(&mut self, _snapshot: UpgradeSnapshot) {}

    /// Whether this overlay binds the `deck_device_info_v1` state feed.
    /// `false` (default) means the framework neither binds it nor delivers
    /// device-info events to this overlay.
    fn uses_device_info(&self) -> bool {
        false
    }

    /// Device lifecycle state reported by the compositor, delivered before
    /// `tick`. Replayed on bind, so the current state arrives even when bmc
    /// broadcast it before this overlay connected.
    ///
    /// `boot_flow_delivered` means this session's operational boot sequence has
    /// already been handed out: screens that run once per boot must not start,
    /// while those reflecting a standing condition ignore it.
    fn on_device_state(&mut self, _state: DeviceState, _boot_flow_delivered: bool) {}

    /// Setup-flow transition reported by the compositor, delivered before
    /// `tick`. `wifi_ssid` is empty unless the step is `ConnectingToWifi`.
    fn on_setup_progress(&mut self, _step: SetupStep, _wifi_ssid: &str) {}

    /// Setup access point reported by the compositor, delivered before `tick`.
    /// `None` means the AP is down.
    fn on_access_point(&mut self, _ap: Option<&AccessPoint>) {}

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

    /// Effective sound volume (0-100) reported by the compositor. Called
    /// before `tick` when a `volume` event arrived. Default: no-op.
    fn on_volume(&mut self, _value: u8) {}

    /// Capability set reported by a v2 compositor, first event after bind.
    /// Never called against a v1 compositor. Called before `tick`. Default:
    /// no-op.
    fn on_capabilities(&mut self, _caps: SettingsCaps) {}

    /// Night-mode state reported by the compositor. `until` is the "HH:MM"
    /// boundary of the current state, `None` while night mode is disabled.
    /// Called before `tick`. Default: no-op.
    fn on_night_mode(&mut self, _active: bool, _until: Option<&str>) {}

    /// One-shot notification that a restart request was declined. Called
    /// before `tick`. Default: no-op.
    fn on_restart_declined(&mut self, _reason: &str) {}

    /// WiFi setup-AP SSID reported by the compositor: `Some(ssid)` while setup
    /// mode is active, `None` when inactive. Called before `tick`. Default:
    /// no-op.
    fn on_wifi_ap(&mut self, _ssid: Option<&str>) {}

    /// Preemption state reported by the compositor: `true` while a modal
    /// full-screen overlay (alarm, startup) is covering the scene, `false` once
    /// it clears. A transient overlay such as the settings-tray retracts on
    /// `true`. Generic — driven by any full-screen preempting overlay, not a
    /// specific feature. Delivered over `deck_settings_v1`, so gated by
    /// `uses_settings`. Called before `tick`. Default: no-op.
    fn on_preempted(&mut self, _active: bool) {}

    /// Drain control requests the overlay wants to send this pass. Called after
    /// `tick`. Default: none.
    fn drain_settings_requests(&mut self) -> Vec<SettingsRequest> {
        Vec::new()
    }

    /// Whether this overlay binds the `deck_alarm_v1` control channel.  `false`
    /// (default) means the framework neither binds it nor delivers alarm
    /// events to this overlay.
    fn uses_alarm(&self) -> bool {
        false
    }

    /// Active alarm reported by the compositor. `period` is the AM/PM marker
    /// for 12-hour time, empty in 24-hour mode. `snooze_allowed` is `false`
    /// when the alarm has no snooze options configured.
    fn on_alarm_ring(&mut self, _time: &str, _period: &str, _label: &str, _snooze_allowed: bool) {}

    /// Compositor requested to stop the alarm.
    fn on_alarm_stop(&mut self) {}

    /// Drain alarm control requests the overlay wants to send this pass. Called
    /// after `tick`. Default: none.
    fn drain_alarm_requests(&mut self) -> Vec<AlarmRequest> {
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

pub(crate) fn deliver_upgrade_snapshot_and_tick(
    overlay: &mut dyn SystemOverlay,
    snapshot: Option<UpgradeSnapshot>,
    now: Instant,
) -> TickOutcome {
    if overlay.uses_upgrade()
        && let Some(snapshot) = snapshot
    {
        overlay.on_upgrade_state(snapshot);
    }
    overlay.tick(now)
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

    #[derive(Default)]
    struct RecordingUpgradeOverlay {
        enabled: bool,
        snapshot: Option<UpgradeSnapshot>,
        calls: Vec<&'static str>,
    }

    impl SystemOverlay for RecordingUpgradeOverlay {
        fn layer_config(&self) -> LayerConfig {
            LayerConfig::fullscreen("recording-upgrade-overlay")
        }

        fn tick(&mut self, _now: Instant) -> TickOutcome {
            self.calls.push("tick");
            TickOutcome::default()
        }

        fn render(&mut self, _renderer: &mut dyn Renderer, _size: (u32, u32)) {}

        fn uses_upgrade(&self) -> bool {
            self.enabled
        }

        fn on_upgrade_state(&mut self, snapshot: UpgradeSnapshot) {
            self.calls.push("upgrade");
            self.snapshot = Some(snapshot);
        }
    }

    fn run_upgrade_delivery() -> RecordingUpgradeOverlay {
        let snapshot = UpgradeSnapshot {
            kind: UpgradeKind::Packages,
            state: UpgradeState::Running {
                phase: Some(UpgradePhase::PackageRealizing),
                progress: Some(DownloadProgress {
                    downloaded_bytes: 3,
                    total_bytes: Some(5),
                }),
            },
        };
        let mut overlay = RecordingUpgradeOverlay {
            enabled: true,
            ..RecordingUpgradeOverlay::default()
        };
        let _ = deliver_upgrade_snapshot_and_tick(&mut overlay, Some(snapshot), Instant::now());
        overlay
    }

    #[test]
    fn shared_upgrade_delivery_calls_callback_before_tick() {
        let overlay = run_upgrade_delivery();

        assert!(overlay.snapshot.is_some());
        assert_eq!(overlay.calls, vec!["upgrade", "tick"]);
    }

    #[test]
    fn opted_out_overlay_does_not_receive_upgrade_snapshot() {
        let mut overlay = RecordingUpgradeOverlay::default();
        let _ = deliver_upgrade_snapshot_and_tick(
            &mut overlay,
            Some(UpgradeSnapshot {
                kind: UpgradeKind::Firmware,
                state: UpgradeState::Succeeded {
                    remaining: Duration::from_secs(1),
                },
            }),
            Instant::now(),
        );

        assert_eq!(overlay.snapshot, None);
        assert_eq!(overlay.calls, vec!["tick"]);
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

    fn armed_hide_fence_gate(
        frame_presented: bool,
        deadline_passed: bool,
        client_running: bool,
    ) -> HideFenceGate {
        HideFenceGate {
            fence: FenceState::Armed { deadline_passed },
            frame_presented,
            client_running,
        }
    }

    #[test]
    fn hide_fence_arms_when_unset() {
        let gate = HideFenceGate {
            fence: FenceState::Unarmed,
            frame_presented: false,
            client_running: true,
        };
        assert_eq!(hide_fence_action(gate), HideFenceAction::Arm);
    }

    #[test]
    fn hide_fence_unmaps_on_presented_callback() {
        assert_eq!(
            hide_fence_action(armed_hide_fence_gate(true, false, true)),
            HideFenceAction::Unmap { timed_out: false }
        );
    }

    #[test]
    fn hide_fence_unmaps_on_deadline() {
        assert_eq!(
            hide_fence_action(armed_hide_fence_gate(false, true, true)),
            HideFenceAction::Unmap { timed_out: true }
        );
    }

    #[test]
    fn hide_fence_waits_while_unresolved() {
        assert_eq!(
            hide_fence_action(armed_hide_fence_gate(false, false, true)),
            HideFenceAction::Wait
        );
    }

    #[test]
    fn hide_fence_unmaps_when_client_gone() {
        assert_eq!(
            hide_fence_action(armed_hide_fence_gate(false, false, false)),
            HideFenceAction::Unmap { timed_out: false }
        );
    }

    #[test]
    fn visible_overlay_clears_stale_hide_fence() {
        let deadline = Instant::now();
        assert_eq!(hide_fence_after_tick(true, Some(deadline)), None);
        assert_eq!(hide_fence_after_tick(false, Some(deadline)), Some(deadline));
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
