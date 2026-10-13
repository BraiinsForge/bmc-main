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

use bmc_platform::DisplayShape;
use bmc_render::animation::DEFAULT_ANIMATION_FRAME_DELAY_MS;
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

/// Decoded `deck_platform_v1` capability set: what the hardware platform
/// supports. Plain bools so overlays never touch the raw protocol bitfield.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent per-board hardware flags, mirroring bmc_platform::HardwareCapabilities"
)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlatformCaps {
    pub wifi: bool,
    pub ethernet: bool,
    pub mining: bool,
    pub boser_managed: bool,
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
    SwitchingUplink,
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

/// The surface an overlay draws into, in logical pixels,
/// and the shape it is seen in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayViewport {
    pub width: u32,
    pub height: u32,
    /// Round only when the surface covers a round display.
    pub shape: DisplayShape,
}

impl OverlayViewport {
    #[must_use]
    pub fn rectangular(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            shape: DisplayShape::Rectangular,
        }
    }

    #[must_use]
    pub fn size(self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// The panel as `deck_platform_v1.display_info` describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlatformDisplay {
    pub width: u32,
    pub height: u32,
    pub shape: DisplayShape,
}

/// A surface covering the whole display is seen in the display's shape;
/// a smaller one sits inside the panel and stays rectangular.
/// Until the compositor has described the display, every surface is rectangular.
#[must_use]
pub(crate) fn overlay_viewport(
    size: (u32, u32),
    display: Option<PlatformDisplay>,
) -> OverlayViewport {
    let (width, height) = size;
    let covers_display = display.filter(|d| (d.width, d.height) == size);
    OverlayViewport {
        width,
        height,
        shape: covers_display.map_or(DisplayShape::Rectangular, |d| d.shape),
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

/// Why a frame renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// The overlay's state changed, or the frame must reflect it anew:
    /// build the tree from the current state.
    Fresh,
    /// Only animations move: replay the last tree.
    AnimationOnly,
}

/// What a rendered frame asks of the next one at the overlay's animation cadence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NextFrame {
    /// Nothing moves: no frame at the cadence.
    #[default]
    Idle,
    /// Animations or transitions still run: replay the tree at the cadence.
    Replay,
    /// The frame's input changed what the overlay draws: build a new tree at the cadence.
    Rebuild,
}

/// What a rendered frame asks of the driver's frame schedule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[must_use]
pub struct RenderOutcome {
    pub next: NextFrame,
    /// Next boundary of a host-rendered time node, whatever `next` asks for.
    pub host_frame_delay_ms: Option<u32>,
}

/// The frames an overlay's last render asked for,
/// on top of the ones its `tick` asks for.
#[derive(Debug, Clone)]
pub(crate) struct OverlayFrameSchedule {
    animation_frame_delay: Duration,
    /// When the frame the schedule was last fed from started.
    started: Option<Instant>,
    last: RenderOutcome,
}

impl OverlayFrameSchedule {
    pub fn new(animation_frame_delay_ms: u32) -> Self {
        Self {
            animation_frame_delay: Duration::from_millis(animation_frame_delay_ms.into()),
            started: None,
            last: RenderOutcome::default(),
        }
    }

    /// The kind of the next frame, as far as the last render asked for it:
    /// a replay or a time-node boundary replays the tree, anything else builds one.
    /// A driver that owes a fresh frame for its own reasons does not ask.
    #[must_use]
    pub fn frame_kind(&self) -> FrameKind {
        let replay = match self.last.next {
            NextFrame::Replay => true,
            NextFrame::Idle => self.last.host_frame_delay_ms.is_some(),
            NextFrame::Rebuild => false,
        };
        if replay {
            FrameKind::AnimationOnly
        } else {
            FrameKind::Fresh
        }
    }

    /// Take in what the frame started at `started` asked for.
    pub fn complete(&mut self, started: Instant, outcome: RenderOutcome) {
        self.last = outcome;
        self.started = Some(started);
    }

    /// When the next frame is due, if the last render asked for one.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        let started = self.started?;
        let cadence = (self.last.next != NextFrame::Idle).then_some(self.animation_frame_delay);
        let boundary = self
            .last
            .host_frame_delay_ms
            .map(|ms| Duration::from_millis(ms.into()));
        let delay = [cadence, boundary].into_iter().flatten().min()?;
        Some(started + delay)
    }

    #[must_use]
    pub fn is_due(&self, now: Instant) -> bool {
        self.deadline().is_some_and(|deadline| now >= deadline)
    }

    /// Fold the schedule into the wake the overlay's tick asked for.
    /// A hidden overlay renders nothing, so its schedule neither wakes the loop nor is due.
    ///
    /// A deadline already passed is `frame_due`, not a wake: an overlay that cannot render
    /// yet, its buffers still with the compositor, would otherwise poll in a tight loop.
    /// The buffer's release arrives on the connection the loop already waits on.
    #[must_use]
    pub fn wake(&self, visible: bool, tick_wake: Option<Instant>, now: Instant) -> FrameWake {
        if !visible {
            return FrameWake {
                next_wake: tick_wake,
                frame_due: false,
            };
        }
        let upcoming = self.deadline().filter(|deadline| *deadline > now);
        FrameWake {
            next_wake: earliest(tick_wake, upcoming),
            frame_due: self.is_due(now),
        }
    }

    /// Forget every wake, for an overlay that hid, failed or is torn down.
    pub fn clear(&mut self) {
        self.started = None;
        self.last = RenderOutcome::default();
    }
}

/// What both drivers decide about an overlay's next frame: the fresh frame they latch for
/// its tick, a reveal or a configure, and the frames its last render asked for.
#[derive(Debug, Clone)]
pub(crate) struct OverlayFrames {
    fresh_requested: bool,
    schedule: OverlayFrameSchedule,
}

impl OverlayFrames {
    pub fn new(animation_frame_delay_ms: u32) -> Self {
        Self {
            fresh_requested: false,
            schedule: OverlayFrameSchedule::new(animation_frame_delay_ms),
        }
    }

    /// The overlay's tick, a reveal or a configure needs a fresh frame.
    /// It stays latched until a frame renders or a slide frame moves the surface,
    /// however long the gates hold it back.
    pub fn request_fresh(&mut self) {
        self.fresh_requested = true;
    }

    /// A hidden overlay renders nothing, so a frame latched for it would only wait.
    pub fn drop_request(&mut self) {
        self.fresh_requested = false;
    }

    /// Whether the overlay wants a frame now: a latched fresh one,
    /// or one its last render asked for that has come due.
    #[must_use]
    pub fn wanted(&self, visible: bool, now: Instant) -> bool {
        self.fresh_requested || self.schedule.wake(visible, None, now).frame_due
    }

    /// Whether a frame the last render asked for has come due, so a slide must paint it.
    #[must_use]
    pub fn frame_due(&self, now: Instant) -> bool {
        self.schedule.is_due(now)
    }

    /// The kind of the next frame: fresh when one is latched or the surface is not mapped yet,
    /// the schedule's otherwise.
    #[must_use]
    pub fn frame_kind(&self, mapped: bool) -> FrameKind {
        if self.fresh_requested || !mapped {
            FrameKind::Fresh
        } else {
            self.schedule.frame_kind()
        }
    }

    #[must_use]
    pub fn wake(&self, visible: bool, tick_wake: Option<Instant>, now: Instant) -> FrameWake {
        self.schedule.wake(visible, tick_wake, now)
    }

    /// A frame started at `started` was painted and submitted, asking for `outcome` next.
    /// The cadence counts from the start, so a frame slower than it leaves the next one due.
    pub fn rendered(&mut self, started: Instant, outcome: RenderOutcome) {
        self.fresh_requested = false;
        self.schedule.complete(started, outcome);
    }

    /// A slide frame moved the surface without painting it.
    pub fn moved(&mut self) {
        self.fresh_requested = false;
    }

    /// The surface was unmapped or is torn down: no frame is owed, and the overlay drops
    /// what only redraws it.
    pub fn hide(&mut self, overlay: &mut dyn SystemOverlay) {
        self.fail();
        overlay.on_hidden();
    }

    /// The overlay failed: nothing of it renders again.
    pub fn fail(&mut self) {
        self.fresh_requested = false;
        self.schedule.clear();
    }
}

/// When an overlay wakes next, and whether its schedule wants a frame now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameWake {
    pub next_wake: Option<Instant>,
    pub frame_due: bool,
}

/// `wait` rounded up to whole milliseconds, the resolution the host polls at.
/// Rounded down, the host wakes just short of a deadline, finds nothing due,
/// and polls in a tight loop through the remaining fraction.
#[must_use]
pub(crate) fn whole_millis_up(wait: Duration) -> Duration {
    Duration::from_millis(u64::try_from(wait.as_micros().div_ceil(1_000)).unwrap_or(u64::MAX))
}

/// The sooner of two optional wakes.
#[must_use]
fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
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
pub use ::deck_upgrade_v1::{DownloadProgress, UpgradeSnapshot, UpgradeState, UpgradeUpdate};

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

    /// Draw the overlay into `viewport`, replaying its last tree on a
    /// [`FrameKind::AnimationOnly`] frame, and report what the frame asks for next.
    /// The clicks and drags this frame hit-tests come back from this same call,
    /// so an overlay whose handling of them changes what it draws asks for
    /// [`NextFrame::Rebuild`] itself; nothing else rebuilds for them.
    /// The `&mut dyn Renderer` is valid only for this call: do not store it.
    fn render(
        &mut self,
        renderer: &mut dyn Renderer,
        viewport: OverlayViewport,
        frame: FrameKind,
    ) -> RenderOutcome;

    /// Least time between two frames while the overlay's tree animates.
    fn animation_frame_delay_ms(&self) -> u32 {
        DEFAULT_ANIMATION_FRAME_DELAY_MS
    }

    /// The surface was unmapped. Drop what only redraws it, such as a cached tree.
    fn on_hidden(&mut self) {}

    /// Pay one-time renderer setup costs at host startup instead of on the
    /// first reveal. The host calls this once, before the event loop, with the
    /// GL context current. Use it to register SVG icons and warm font glyphs so
    /// the first swipe-reveal does not stall.
    /// `viewport` is the one the first render will get.
    /// Default: no-op. The `&mut dyn Renderer` is valid only for this call: do not store it.
    fn prewarm(&mut self, _renderer: &mut dyn Renderer, _viewport: OverlayViewport) {}

    /// Handle a touch event (only delivered when input region is not `None`).
    fn on_touch(&mut self, _event: TouchEvent) {}

    /// Whether this overlay binds the compositor upgrade-state protocol.
    fn uses_upgrade(&self) -> bool {
        false
    }
    /// Receive one coherent compositor upgrade snapshot before `tick`.
    fn on_upgrade_state(&mut self, _snapshot: UpgradeSnapshot) {}
    /// Clear the current upgrade presentation before `tick`.
    fn on_upgrade_cleared(&mut self) {}

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

    /// The user pressed the IP-report button and wants the device address
    /// on screen. Delivered before `tick`, never replayed on bind.
    fn on_report_ip(&mut self) {}

    /// Whether this overlay is told the platform capabilities and the product name.
    /// The framework binds `deck_platform_v1` either way, for the display.
    fn uses_platform(&self) -> bool {
        false
    }

    /// Hardware platform capabilities reported by the compositor. Delivered
    /// once, before the first `tick` that follows the bind; the set never
    /// changes for the compositor's lifetime. Default: no-op.
    fn on_platform_capabilities(&mut self, _caps: PlatformCaps) {}

    /// The product's display name ("Braiins Deck", "Braiins Mini Miner"),
    /// for copy that addresses the user by it.
    /// Delivered once, right after the capabilities and before the first `tick`
    /// that follows the bind; a v1 compositor never sends it.
    /// A label, never a gate: branch on [`Self::on_platform_capabilities`] instead.
    /// Default: no-op.
    fn on_platform_product_name(&mut self, _name: &str) {}

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

    /// Whether a hosted slide frame can reuse its attached content.
    /// Return `false` for content changes or frames that need a fresh paint.
    /// Standalone ignores both hooks and repaints every frame without sliding.
    fn can_reuse_content(&self, _now: Instant) -> bool {
        false
    }

    /// Vertical surface translation in logical pixels; positive offsets move down.
    /// Applies to hosted frames, including fresh paints.
    /// `viewport` is the one this pass renders into.
    /// Return it independently of content changes; [`Self::can_reuse_content`]
    /// determines whether the attached content can be reused. Standalone ignores it.
    fn layer_shell_offset(&self, _now: Instant, _viewport: OverlayViewport) -> Option<f32> {
        None
    }

    /// Take and clear the content-changed flag after a full paint.
    fn take_content_dirty(&mut self) -> bool {
        false
    }

    /// Observe the content-changed flag without consuming it.
    fn content_dirty(&self) -> bool {
        false
    }

    /// Invalidate attached content so the next frame paints again.
    fn mark_content_dirty(&mut self) {}

    /// A frame of this overlay was just submitted to the compositor. Called
    /// with a timestamp taken after the submit, so time-anchored animations
    /// anchor at the hand-off rather than at the trigger that requested the
    /// frame. Default: no-op.
    fn on_frame_submitted(&mut self, _now: Instant) {}
}

/// The `deck_platform_v1` events received since the last dispatch round.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PlatformEvents {
    pub caps: Option<PlatformCaps>,
    pub product_name: Option<String>,
}

/// Hand the received platform events to an overlay that asked for them.
/// Runs before every other protocol's events and before `tick`,
/// so the set is in place before the overlay decides anything on it.
pub(crate) fn deliver_platform_events(overlay: &mut dyn SystemOverlay, events: PlatformEvents) {
    if !overlay.uses_platform() {
        return;
    }
    if let Some(caps) = events.caps {
        overlay.on_platform_capabilities(caps);
    }
    if let Some(name) = events.product_name {
        overlay.on_platform_product_name(&name);
    }
}

pub(crate) fn deliver_upgrade_update_and_tick(
    overlay: &mut dyn SystemOverlay,
    update: Option<UpgradeUpdate>,
    now: Instant,
) -> TickOutcome {
    if overlay.uses_upgrade()
        && let Some(update) = update
    {
        match update {
            UpgradeUpdate::Snapshot(snapshot) => overlay.on_upgrade_state(snapshot),
            UpgradeUpdate::Cleared => overlay.on_upgrade_cleared(),
        }
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
        clears: usize,
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

        fn render(
            &mut self,
            _renderer: &mut dyn Renderer,
            _viewport: OverlayViewport,
            _frame: FrameKind,
        ) -> RenderOutcome {
            RenderOutcome::default()
        }

        fn uses_upgrade(&self) -> bool {
            self.enabled
        }

        fn on_upgrade_state(&mut self, snapshot: UpgradeSnapshot) {
            self.calls.push("upgrade");
            self.snapshot = Some(snapshot);
        }

        fn on_upgrade_cleared(&mut self) {
            self.calls.push("clear");
            self.clears += 1;
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
        let _ = deliver_upgrade_update_and_tick(
            &mut overlay,
            Some(UpgradeUpdate::Snapshot(snapshot)),
            Instant::now(),
        );
        overlay
    }

    #[test]
    fn shared_upgrade_delivery_calls_callback_before_tick() {
        let overlay = run_upgrade_delivery();

        assert!(overlay.snapshot.is_some());
        assert_eq!(overlay.calls, vec!["upgrade", "tick"]);
    }

    #[test]
    fn shared_upgrade_clear_calls_callback_before_tick() {
        let mut overlay = RecordingUpgradeOverlay {
            enabled: true,
            ..RecordingUpgradeOverlay::default()
        };

        let _ = deliver_upgrade_update_and_tick(
            &mut overlay,
            Some(UpgradeUpdate::Cleared),
            Instant::now(),
        );

        assert_eq!(overlay.clears, 1);
        assert_eq!(overlay.calls, vec!["clear", "tick"]);
    }

    #[test]
    fn opted_out_overlay_does_not_receive_upgrade_snapshot() {
        let mut overlay = RecordingUpgradeOverlay::default();
        let _ = deliver_upgrade_update_and_tick(
            &mut overlay,
            Some(UpgradeUpdate::Snapshot(UpgradeSnapshot {
                kind: UpgradeKind::Firmware,
                state: UpgradeState::Succeeded {
                    remaining: Duration::from_secs(1),
                },
            })),
            Instant::now(),
        );
        let _ = deliver_upgrade_update_and_tick(
            &mut overlay,
            Some(UpgradeUpdate::Cleared),
            Instant::now(),
        );

        assert_eq!(overlay.snapshot, None);
        assert_eq!(overlay.clears, 0);
        assert_eq!(overlay.calls, vec!["tick", "tick"]);
    }

    #[derive(Default)]
    struct RecordingPlatformOverlay {
        enabled: bool,
        caps: Option<PlatformCaps>,
        product_name: Option<String>,
    }

    impl SystemOverlay for RecordingPlatformOverlay {
        fn layer_config(&self) -> LayerConfig {
            LayerConfig::fullscreen("recording-platform-overlay")
        }

        fn tick(&mut self, _now: Instant) -> TickOutcome {
            TickOutcome::default()
        }

        fn render(
            &mut self,
            _renderer: &mut dyn Renderer,
            _viewport: OverlayViewport,
            _frame: FrameKind,
        ) -> RenderOutcome {
            RenderOutcome::default()
        }

        fn uses_platform(&self) -> bool {
            self.enabled
        }

        fn on_platform_capabilities(&mut self, caps: PlatformCaps) {
            self.caps = Some(caps);
        }

        fn on_platform_product_name(&mut self, name: &str) {
            self.product_name = Some(name.to_owned());
        }
    }

    const MINER_CAPS: PlatformCaps = PlatformCaps {
        wifi: true,
        ethernet: true,
        mining: true,
        boser_managed: true,
    };

    fn miner_events() -> PlatformEvents {
        PlatformEvents {
            caps: Some(MINER_CAPS),
            product_name: Some("Braiins Mini Miner".to_owned()),
        }
    }

    #[test]
    fn platform_events_reach_an_opted_in_overlay() {
        let mut overlay = RecordingPlatformOverlay {
            enabled: true,
            ..RecordingPlatformOverlay::default()
        };
        deliver_platform_events(&mut overlay, miner_events());
        assert_eq!(overlay.caps, Some(MINER_CAPS));
        assert_eq!(overlay.product_name.as_deref(), Some("Braiins Mini Miner"));
    }

    #[test]
    fn platform_events_skip_an_opted_out_overlay() {
        let mut overlay = RecordingPlatformOverlay::default();
        deliver_platform_events(&mut overlay, miner_events());
        assert_eq!(overlay.caps, None);
        assert_eq!(overlay.product_name, None);
    }

    #[test]
    fn resolved_configured_size_falls_back_when_compositor_reports_zero_axis() {
        assert_eq!(resolved_configured_size((420, 180), (0, 180)), (420, 180));
        assert_eq!(resolved_configured_size((420, 180), (420, 0)), (420, 180));
        assert_eq!(resolved_configured_size((0, 0), (0, 0)), (1, 1));
    }

    const ROUND_PANEL: PlatformDisplay = PlatformDisplay {
        width: 480,
        height: 480,
        shape: DisplayShape::Round,
    };

    #[test]
    fn a_surface_covering_a_round_panel_is_round() {
        assert_eq!(
            overlay_viewport((480, 480), Some(ROUND_PANEL)).shape,
            DisplayShape::Round
        );
    }

    #[test]
    fn a_surface_smaller_than_a_round_panel_stays_rectangular() {
        assert_eq!(
            overlay_viewport((480, 120), Some(ROUND_PANEL)).shape,
            DisplayShape::Rectangular
        );
    }

    #[test]
    fn a_surface_is_rectangular_until_the_display_is_described() {
        assert_eq!(
            overlay_viewport((480, 480), None),
            OverlayViewport::rectangular(480, 480)
        );
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

    const CADENCE_MS: u32 = 33;

    fn animating() -> RenderOutcome {
        RenderOutcome {
            next: NextFrame::Replay,
            ..RenderOutcome::default()
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn an_animating_frame_asks_for_the_next_one_at_the_cadence() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(CADENCE_MS);
        schedule.complete(t0, animating());
        assert_eq!(schedule.deadline(), Some(t0 + ms(33)));
        assert!(!schedule.is_due(t0 + ms(32)));
        assert!(schedule.is_due(t0 + ms(33)));
        assert_eq!(schedule.frame_kind(), FrameKind::AnimationOnly);
    }

    #[test]
    fn the_overlay_sets_its_own_cadence() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(100);
        schedule.complete(t0, animating());
        assert_eq!(schedule.deadline(), Some(t0 + ms(100)));
    }

    #[test]
    fn a_settled_frame_asks_for_nothing() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(CADENCE_MS);
        schedule.complete(t0, animating());
        schedule.complete(t0 + ms(33), RenderOutcome::default());
        assert_eq!(schedule.deadline(), None);
        assert_eq!(schedule.frame_kind(), FrameKind::Fresh);
    }

    #[test]
    fn a_requested_rebuild_comes_fresh_at_the_cadence() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(CADENCE_MS);
        schedule.complete(
            t0,
            RenderOutcome {
                next: NextFrame::Rebuild,
                ..RenderOutcome::default()
            },
        );
        assert_eq!(schedule.deadline(), Some(t0 + ms(33)));
        assert_eq!(schedule.frame_kind(), FrameKind::Fresh);
    }

    #[test]
    fn a_rebuild_wins_over_a_time_node_boundary() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(CADENCE_MS);
        schedule.complete(
            t0,
            RenderOutcome {
                next: NextFrame::Rebuild,
                host_frame_delay_ms: Some(10),
            },
        );
        assert_eq!(
            schedule.deadline(),
            Some(t0 + ms(10)),
            "the earlier of the two wakes"
        );
        assert_eq!(schedule.frame_kind(), FrameKind::Fresh);
    }

    #[test]
    fn the_cadence_caps_a_far_time_node_boundary() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(CADENCE_MS);
        schedule.complete(
            t0,
            RenderOutcome {
                next: NextFrame::Replay,
                host_frame_delay_ms: Some(1_000),
            },
        );
        assert_eq!(schedule.deadline(), Some(t0 + ms(33)));
    }

    #[test]
    fn a_time_node_replays_at_its_boundary() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(CADENCE_MS);
        schedule.complete(
            t0,
            RenderOutcome {
                host_frame_delay_ms: Some(1_000),
                ..RenderOutcome::default()
            },
        );
        assert_eq!(schedule.deadline(), Some(t0 + ms(1_000)));
        assert_eq!(schedule.frame_kind(), FrameKind::AnimationOnly);
    }

    #[test]
    fn a_visible_animating_overlay_wakes_at_the_earlier_of_tick_and_frame() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(CADENCE_MS);
        schedule.complete(t0, animating());
        let wake = schedule.wake(true, Some(t0 + ms(2_000)), t0 + ms(20));
        assert_eq!(
            wake,
            FrameWake {
                next_wake: Some(t0 + ms(33)),
                frame_due: false,
            }
        );
        let wake = schedule.wake(true, Some(t0 + ms(10)), t0);
        assert_eq!(wake.next_wake, Some(t0 + ms(10)));
    }

    #[test]
    fn a_frame_that_cannot_render_yet_does_not_spin_the_loop() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(CADENCE_MS);
        schedule.complete(t0, animating());
        let now = t0 + ms(40);
        let wake = schedule.wake(true, Some(t0 + ms(2_000)), now);
        assert_eq!(
            wake,
            FrameWake {
                next_wake: Some(t0 + ms(2_000)),
                frame_due: true,
            },
            "a passed deadline is due, not a wake"
        );
        let both_buffers_held = PollGate {
            failed: false,
            visible: true,
            wants_render: wake.frame_due,
            client_running: true,
            target_available: false,
        };
        let tick = wake.next_wake.map(|t| t.saturating_duration_since(now));
        assert_eq!(
            overlay_poll_timeout(both_buffers_held, tick, None),
            Some(ms(1_960)),
            "the loop waits for the buffer release or the tick, not in a tight loop"
        );
    }

    #[test]
    fn a_hidden_overlay_is_woken_only_by_its_tick() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(CADENCE_MS);
        schedule.complete(t0, animating());
        let wake = schedule.wake(false, None, t0 + ms(40));
        assert_eq!(
            wake,
            FrameWake {
                next_wake: None,
                frame_due: false,
            }
        );
    }

    #[test]
    fn a_settled_overlay_is_woken_only_by_its_tick() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(CADENCE_MS);
        schedule.complete(t0, RenderOutcome::default());
        let wake = schedule.wake(true, Some(t0 + ms(2_000)), t0 + ms(40));
        assert_eq!(
            wake,
            FrameWake {
                next_wake: Some(t0 + ms(2_000)),
                frame_due: false,
            }
        );
    }

    #[test]
    fn clear_forgets_the_wake_but_keeps_the_cadence() {
        let t0 = Instant::now();
        let mut schedule = OverlayFrameSchedule::new(100);
        schedule.complete(t0, animating());
        schedule.clear();
        assert_eq!(schedule.deadline(), None);
        schedule.complete(t0, animating());
        assert_eq!(schedule.deadline(), Some(t0 + ms(100)));
    }

    /// Counts the `on_hidden` calls a driver makes.
    #[derive(Default)]
    struct HideCounter {
        hidden: usize,
    }

    impl SystemOverlay for HideCounter {
        fn layer_config(&self) -> LayerConfig {
            LayerConfig::fullscreen("hide-counter")
        }

        fn tick(&mut self, _now: Instant) -> TickOutcome {
            TickOutcome::default()
        }

        fn render(
            &mut self,
            _renderer: &mut dyn Renderer,
            _viewport: OverlayViewport,
            _frame: FrameKind,
        ) -> RenderOutcome {
            RenderOutcome::default()
        }

        fn on_hidden(&mut self) {
            self.hidden += 1;
        }
    }

    /// Frames for a visible, mapped overlay whose last render, at `t0`, asked for a replay.
    fn replaying(t0: Instant) -> OverlayFrames {
        let mut frames = OverlayFrames::new(CADENCE_MS);
        frames.rendered(t0, animating());
        frames
    }

    #[test]
    fn a_due_replay_is_wanted_while_the_overlay_is_visible() {
        let t0 = Instant::now();
        let frames = replaying(t0);
        assert!(!frames.wanted(true, t0 + ms(20)), "not due yet");
        assert!(frames.wanted(true, t0 + ms(33)));
        assert!(
            !frames.wanted(false, t0 + ms(33)),
            "a hidden overlay renders nothing"
        );
    }

    #[test]
    fn a_requested_fresh_frame_stays_wanted_until_one_renders() {
        let t0 = Instant::now();
        let mut frames = OverlayFrames::new(CADENCE_MS);
        assert!(!frames.wanted(true, t0));
        frames.request_fresh();
        assert!(frames.wanted(true, t0));
        assert!(
            frames.wanted(true, t0 + ms(500)),
            "held back by the gates, still owed"
        );
        frames.rendered(t0 + ms(500), RenderOutcome::default());
        assert!(!frames.wanted(true, t0 + ms(500)));
    }

    #[test]
    fn a_requested_or_first_frame_is_fresh_whatever_the_last_render_asked() {
        let t0 = Instant::now();
        let mut frames = replaying(t0);
        assert_eq!(frames.frame_kind(true), FrameKind::AnimationOnly);
        assert_eq!(
            frames.frame_kind(false),
            FrameKind::Fresh,
            "a surface not mapped yet has nothing to replay over"
        );
        frames.request_fresh();
        assert_eq!(frames.frame_kind(true), FrameKind::Fresh);
    }

    #[test]
    fn a_moved_slide_frame_keeps_the_replay_owed() {
        let t0 = Instant::now();
        let mut frames = replaying(t0);
        frames.request_fresh();
        frames.moved();
        assert!(
            !frames.wanted(true, t0 + ms(10)),
            "the move answered the request"
        );
        assert!(frames.wanted(true, t0 + ms(33)), "the replay still comes");
        assert_eq!(frames.frame_kind(true), FrameKind::AnimationOnly);
    }

    #[test]
    fn hiding_drops_every_frame_and_tells_the_overlay() {
        let t0 = Instant::now();
        let mut frames = replaying(t0);
        frames.request_fresh();
        let mut overlay = HideCounter::default();
        frames.hide(&mut overlay);
        assert_eq!(overlay.hidden, 1, "the overlay drops its cached tree");
        assert!(!frames.wanted(true, t0 + ms(33)));
        assert_eq!(frames.wake(true, None, t0).next_wake, None);
    }

    #[test]
    fn a_failed_overlay_wants_no_frame() {
        let t0 = Instant::now();
        let mut frames = replaying(t0);
        frames.request_fresh();
        frames.fail();
        assert!(!frames.wanted(true, t0 + ms(33)));
        assert_eq!(frames.wake(true, None, t0).next_wake, None);
    }

    #[test]
    fn a_wait_rounds_up_to_the_millisecond_the_host_polls_at() {
        assert_eq!(
            whole_millis_up(Duration::from_micros(700)),
            ms(1),
            "a deadline 0.7 ms off must not poll at zero"
        );
        assert_eq!(whole_millis_up(Duration::from_micros(32_700)), ms(33));
        assert_eq!(whole_millis_up(ms(33)), ms(33), "whole milliseconds stay");
        assert_eq!(whole_millis_up(Duration::ZERO), Duration::ZERO);
    }
}
