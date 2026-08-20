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

//! Swipe-from-top quick-settings overlay: round icon controls (±10 step
//! buttons for brightness and volume, night-mode toggle, hold-to-confirm
//! restart and WiFi reconfigure) plus WiFi station info. Ported from
//! the `settings-stub` widget to a native `bmc-render` `TreeNode` overlay.
//!
//! The surface is fullscreen with a full input region so the tray blocks scene
//! swipes behind it while up. It dismisses via its close button, an upward
//! swipe, or an inactivity timeout.

mod dismiss;
mod fsm;
mod icons;
pub mod ui;

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use bmc_platform::{BmcInfo, DisplayShape, HardwareProfile, Product};
use bmc_render::renderer::Renderer;
use bmc_system_overlay::{
    Anchor, InputRegion, Layer, LayerConfig, ScreenEdge, SettingsRequest, SnapshotVersion,
    SystemOverlay, TickOutcome, TouchEvent, TreeUi, VersionedSnapshot,
};

use crate::dismiss::Pt;
use crate::fsm::{ButtonState, FsmAction, RestartAction, RestartState};
use crate::ui::{Panel, WifiView};

/// Kernel hostname, exposed by procfs.
const HOSTNAME_PATH: &str = "/proc/sys/kernel/hostname";

/// Idle wake cadence for re-reading the connectivity snapshot while up.
const NETWORK_REFRESH: Duration = Duration::from_secs(2);

/// Idle period after which the tray auto-dismisses.
const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(15);

/// After a ± step tap, incoming events for that setting stay dropped for this
/// settle window (extended per tap) so a late in-flight echo of our own write
/// cannot snap the value back and steal the base of the next tap. External
/// changes landing inside the window are lost until the next broadcast —
/// accepted.
const STEP_ECHO_SETTLE: Duration = Duration::from_millis(300);

/// Fast wake cadence while a hold FSM is animating, so the hold/timeout edges
/// fire without a touch/network event to wake the loop.
const FAST_WAKE: Duration = Duration::from_millis(33);

/// Whether WiFi reconfiguration is supported on this platform. It only works
/// where the setup AP runs over the mac80211 radio (BMC100, BFM100).
/// The BMM boards drive their ESP32 AP through a separate firmware path
/// the overlay does not implement, so the reconfigure button is hidden there.
fn wifi_reconfig_supported(product: Product) -> bool {
    matches!(product, Product::Bmc100 | Product::Bfm100)
}

/// Device hostname from procfs, trimmed of its trailing newline. `None` when
/// the file is unreadable or empty.
fn read_hostname() -> Option<String> {
    let raw = std::fs::read_to_string(HOSTNAME_PATH).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Injected connectivity source for testing.
trait Env {
    /// Latest snapshot and its version when the content changed since `seen`
    /// (`None` = nothing seen yet); `None` — with no allocation — otherwise.
    fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot>;
}

struct OsEnv;
impl Env for OsEnv {
    fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
        bmc_system_overlay::snapshot_if_changed(seen)
    }
}

/// The active touch's start point and latest position, for the dismiss
/// classifier (`start`→`end` on finger-up).
#[derive(Debug, Clone, Copy)]
struct TouchTrack {
    start: Pt,
    latest: Pt,
}

/// Duration of the reveal/dismiss slide ramp.
const SLIDE_MS: u64 = 180;

/// State of the panel's slide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SlidePhase {
    /// No ramp active; the panel is settled (or was never revealed).
    #[default]
    Idle,
    /// Reveal armed; the panel holds off-screen until the first frame of the
    /// reveal is presented, which anchors the ramp clock. Anchoring at the
    /// trigger instead would let the first frame's paint cost consume the
    /// front of the ease-out ramp.
    RevealPending,
    Revealing {
        since: Instant,
    },
    /// The reveal ramp has elapsed but the settled frame (offset `0`) has not
    /// been presented yet. The ramp only presents frames while a tick observes
    /// it mid-flight, so a loop stall spanning the ramp end would otherwise
    /// freeze the panel at the last presented offset.
    RevealSettling,
    /// Dismiss armed; the panel holds the settled position until the first
    /// frame of the dismiss is presented, which anchors the ramp clock.
    DismissPending,
    Dismissing {
        since: Instant,
    },
}

/// Pure eased vertical slide for the panel band: the panel translates from
/// off-screen (`-height`) to settled (`0`) on reveal and back on dismiss. The
/// offset is computed from `now`; nothing here touches the GPU, so the timing is
/// unit-tested in isolation from the blit.
#[derive(Debug, Clone, Copy, Default)]
struct Slide {
    phase: SlidePhase,
}

impl Slide {
    fn start_reveal(&mut self) {
        self.phase = SlidePhase::RevealPending;
    }

    fn start_dismiss(&mut self) {
        self.phase = SlidePhase::DismissPending;
    }

    /// Whether a dismiss has been started (pending, mid-ramp, or completed).
    fn is_dismissing(&self) -> bool {
        matches!(
            self.phase,
            SlidePhase::DismissPending | SlidePhase::Dismissing { .. }
        )
    }

    /// Input is accepted only while the panel is fully settled.
    fn accepts_input(&self) -> bool {
        matches!(self.phase, SlidePhase::Idle)
    }

    /// Anchor a pending ramp at the moment its first frame was presented.
    /// No-op in every other phase, so late or duplicate present notifications
    /// cannot restart a running ramp.
    fn anchor(&mut self, now: Instant) {
        match self.phase {
            SlidePhase::RevealPending => self.phase = SlidePhase::Revealing { since: now },
            SlidePhase::DismissPending => self.phase = SlidePhase::Dismissing { since: now },
            SlidePhase::Idle
            | SlidePhase::Revealing { .. }
            | SlidePhase::RevealSettling
            | SlidePhase::Dismissing { .. } => {}
        }
    }

    /// Advance the time-driven transition: a reveal ramp that has elapsed moves
    /// to `RevealSettling` until the settled frame is presented.
    fn advance(&mut self, now: Instant) {
        if let SlidePhase::Revealing { since } = self.phase
            && Self::progress(since, now) >= 1.0
        {
            self.phase = SlidePhase::RevealSettling;
        }
    }

    /// Whether the reveal ramp elapsed without the settled frame having been
    /// presented; keeps the tray requesting a render for that one final frame.
    fn needs_settle_frame(&self) -> bool {
        matches!(self.phase, SlidePhase::RevealSettling)
    }

    /// The settled frame was presented; the reveal is done.
    fn mark_settled(&mut self) {
        if matches!(self.phase, SlidePhase::RevealSettling) {
            self.phase = SlidePhase::Idle;
        }
    }

    /// The blit-only decision the host obeys: blit the cached panel at the
    /// current offset only while a slide is running (or for the final settle
    /// frame) *and* the content has not changed this frame; otherwise (`None`)
    /// the host full-paints. Keeping it a method on `Slide` lets the invariant
    /// be unit-tested without a platform-detected `SettingsTrayOverlay`.
    fn cached_blit_offset(&self, now: Instant, content_dirty: bool, height: f32) -> Option<f32> {
        ((self.animating(now) || self.needs_settle_frame()) && !content_dirty)
            .then(|| self.offset(now, height))
    }

    /// Eased progress (0→1) of the active ramp at `now`. `ease-out` cubic so the
    /// panel decelerates as it settles.
    fn progress(start: Instant, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(start).as_millis();
        #[expect(
            clippy::cast_precision_loss,
            reason = "slide elapsed/duration are small millisecond counts"
        )]
        let linear = (elapsed as f32 / SLIDE_MS as f32).clamp(0.0, 1.0);
        let inv = 1.0 - linear;
        1.0 - inv * inv * inv
    }

    /// Vertical offset (px) of the panel band at `now`, given the panel
    /// `height`. `0` once settled (or when no slide is active).
    fn offset(&self, now: Instant, height: f32) -> f32 {
        match self.phase {
            SlidePhase::RevealPending => -height,
            SlidePhase::Revealing { since } => -height * (1.0 - Self::progress(since, now)),
            SlidePhase::Dismissing { since } => -height * Self::progress(since, now),
            SlidePhase::DismissPending | SlidePhase::RevealSettling | SlidePhase::Idle => 0.0,
        }
    }

    /// Whether a ramp is still in progress at `now`.
    fn animating(&self, now: Instant) -> bool {
        match self.phase {
            SlidePhase::Revealing { since } | SlidePhase::Dismissing { since } => {
                Self::progress(since, now) < 1.0
            }
            SlidePhase::RevealPending | SlidePhase::DismissPending => true,
            SlidePhase::RevealSettling | SlidePhase::Idle => false,
        }
    }

    /// Whether a dismiss ramp has fully completed at `now`.
    fn dismiss_done(&self, now: Instant) -> bool {
        match self.phase {
            SlidePhase::Dismissing { since } => Self::progress(since, now) >= 1.0,
            SlidePhase::Revealing { .. }
            | SlidePhase::RevealPending
            | SlidePhase::RevealSettling
            | SlidePhase::DismissPending
            | SlidePhase::Idle => false,
        }
    }
}

/// Product selector for deterministic gallery tray views.
#[doc(hidden)]
pub use bmc_platform::Product as SettingsTrayProduct;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NightModeView {
    pub active: bool,
    /// "HH:MM" boundary of the current state; `None` while night mode is
    /// disabled.
    pub until: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent per-control visibility flags mirroring the capability bits"
)]
pub struct SettingsTrayView {
    pub shape: DisplayShape,
    pub width: u32,
    pub height: u32,
    pub brightness: u8,
    pub show_brightness: bool,
    pub volume: u8,
    pub show_volume: bool,
    pub night_mode: Option<NightModeView>,
    pub show_restart: bool,
    /// Dynamic captions for the shared caption line; `None` when the control
    /// rests. The restart caption carries the decline reason while the FSM
    /// surfaces one.
    pub restart_caption: Option<String>,
    pub reconfig_caption: Option<String>,
    /// 0..=1 hold fractions for the progress rings.
    pub restart_progress: f32,
    pub reconfig_progress: f32,
    pub hostname: Option<String>,
    pub ip: Option<String>,
    pub wifi_signal: Option<i32>,
    pub ssid: Option<String>,
    pub setup_ssid: Option<String>,
    pub wifi_button: bool,
    /// Forced pressed key for gallery injection; the runtime leaves this
    /// `None` and derives the pressed key from the tree's touch state.
    pub pressed_key: Option<String>,
}

impl SettingsTrayView {
    /// Build a deterministic gallery-facing view shell for a hardware product.
    #[doc(hidden)]
    #[must_use]
    pub fn for_product(product: SettingsTrayProduct) -> Self {
        let profile = HardwareProfile::for_product(product);
        Self {
            shape: profile.display.shape,
            width: profile.display.logical_width,
            height: profile.display.logical_height,
            brightness: 50,
            show_brightness: true,
            volume: 50,
            show_volume: matches!(product, SettingsTrayProduct::Bmc100),
            night_mode: Some(NightModeView {
                active: false,
                until: None,
            }),
            show_restart: true,
            restart_caption: None,
            reconfig_caption: None,
            restart_progress: 0.0,
            reconfig_progress: 0.0,
            hostname: None,
            ip: None,
            wifi_signal: None,
            ssid: None,
            setup_ssid: None,
            wifi_button: wifi_reconfig_supported(product),
            pressed_key: None,
        }
    }
}

#[expect(missing_debug_implementations, reason = "TreeUi is not Debug")]
pub struct SettingsTrayRenderState {
    tree: TreeUi,
    icons: Option<icons::TrayIcons>,
    last_render: Instant,
}

impl SettingsTrayRenderState {
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            tree: TreeUi::new(),
            icons: None,
            last_render: now,
        }
    }
}

/// Direction of a ± step tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SettingsTrayRenderOutput {
    pub brightness_step: Option<Step>,
    pub volume_step: Option<Step>,
    pub night_mode_tapped: bool,
    pub close_tapped: bool,
    /// The pressed key sampled after this frame's event processing differs
    /// from the one the tree was built with — the overlay must repaint so
    /// press/release inversion shows (no FSM polls the step buttons).
    pub pressed_changed: bool,
}

/// Step size for the ± volume/brightness buttons, matching the stable tray.
const STEP: u8 = 10;

fn step_value(current: u8, step: Step, min: u8, max: u8) -> u8 {
    match step {
        Step::Up => current.saturating_add(STEP).min(max),
        Step::Down => current.saturating_sub(STEP).max(min),
    }
}

#[expect(missing_debug_implementations, reason = "TreeUi is not Debug")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a distinct single-bit latch with no natural enum pairing"
)]
pub struct SettingsTrayOverlay {
    product: Product,
    shape: DisplayShape,
    width: u32,
    height: u32,
    /// Full panel height (px); sets the slide-animation travel distance.
    panel_height: f32,

    brightness: u8,
    /// End of the post-tap brightness echo settle window.
    brightness_settle_until: Option<Instant>,
    volume: u8,
    /// End of the post-tap volume echo settle window.
    volume_settle_until: Option<Instant>,
    /// Latest night-mode state from the `night_mode` event. No local prediction.
    night_active: bool,
    /// "HH:MM" boundary from the `night_mode` event; `None` while night mode
    /// is disabled or not yet reported.
    night_until: Option<String>,
    hostname: Option<String>,
    ip: Option<String>,
    wifi_signal: Option<i32>,
    ssid: Option<String>,
    /// Version of the last connectivity snapshot folded into the fields
    /// above (`None` = none yet); lets `refresh_network` skip unchanged reads.
    snapshot_version: Option<SnapshotVersion>,
    /// WiFi setup-AP SSID from `on_wifi_ap` (`None` = not in setup mode).
    setup_ssid: Option<String>,

    button: ButtonState,

    restart: RestartState,
    /// Reason from the last `restart_declined` event, shown in place of the
    /// generic message while the restart FSM surfaces one.
    declined_reason: Option<String>,

    render_state: SettingsTrayRenderState,

    touch_track: Option<TouchTrack>,
    last_interaction: Instant,

    dismissing: bool,
    /// Set on any content change; drives the Task-9 panel cache.
    content_dirty: bool,
    /// State changed during a render pass's read-back. Converted into
    /// `content_dirty` at the start of the next tick — setting `content_dirty`
    /// directly would be consumed by the host right after the stale paint the
    /// read-back came from, caching the old frame with no repaint to follow.
    repaint_queued: bool,
    /// Pure reveal/dismiss slide phase; the host reads its offset to blit the
    /// cached panel without re-laying-out the tree.
    slide: Slide,

    /// Capability set from the compositor. `None` until the first (v2-only)
    /// capabilities event — and forever against a v1 compositor, which is the
    /// centralized v1-fallback signal: hide every v2 control and fall back to
    /// the locally derived product gates.
    caps: Option<bmc_system_overlay::SettingsCaps>,

    pending_requests: Vec<SettingsRequest>,

    env: Box<dyn Env>,
}

impl Default for SettingsTrayOverlay {
    fn default() -> Self {
        let product = BmcInfo::load()
            .map(|info| info.bmc_platform.product())
            .expect("BUG: platform detection must succeed for the settings tray");
        Self::new_for_product(product, read_hostname(), Instant::now())
    }
}

impl SettingsTrayOverlay {
    fn new_for_product(product: Product, hostname: Option<String>, now: Instant) -> Self {
        let profile = HardwareProfile::for_product(product);
        let width = profile.display.logical_width;
        let height = profile.display.logical_height;
        let shape = profile.display.shape;
        let panel_height = panel_height_for(height);

        Self {
            product,
            shape,
            width,
            height,
            panel_height,
            brightness: 50,
            brightness_settle_until: None,
            volume: 50,
            volume_settle_until: None,
            night_active: false,
            night_until: None,
            hostname,
            ip: None,
            wifi_signal: None,
            ssid: None,
            snapshot_version: None,
            setup_ssid: None,
            button: ButtonState::default(),
            restart: RestartState::default(),
            declined_reason: None,
            render_state: SettingsTrayRenderState::new(now),
            touch_track: None,
            last_interaction: now,
            dismissing: false,
            content_dirty: true,
            repaint_queued: false,
            slide: Slide::default(),
            caps: None,
            pending_requests: Vec::new(),
            env: Box::new(OsEnv),
        }
    }

    #[must_use]
    fn view(&self, now: Instant) -> SettingsTrayView {
        let mut view = SettingsTrayView::for_product(self.product);
        view.shape = self.shape;
        view.width = self.width;
        view.height = self.height;
        view.brightness = self.brightness;
        view.volume = self.volume;
        view.night_mode = Some(NightModeView {
            active: self.night_active,
            until: self.night_until.clone(),
        });
        view.restart_caption = if self.restart.shows_message() {
            Some(
                self.declined_reason
                    .clone()
                    .unwrap_or_else(|| self.restart.label().to_owned()),
            )
        } else {
            self.restart.caption().map(str::to_owned)
        };
        view.restart_progress = self.restart.progress(now);
        view.reconfig_caption = self.button.caption().map(str::to_owned);
        view.reconfig_progress = self.button.progress(now);
        view.hostname.clone_from(&self.hostname);
        view.ip.clone_from(&self.ip);
        view.wifi_signal = self.wifi_signal;
        view.ssid.clone_from(&self.ssid);
        view.setup_ssid.clone_from(&self.setup_ssid);
        view.wifi_button = wifi_reconfig_supported(self.product);
        if let Some(caps) = self.caps {
            view.show_brightness = caps.brightness;
            view.show_volume = caps.sound;
            view.wifi_button = caps.wifi_setup;
        } else {
            // v1 compositor: exactly today's tray. Brightness + the
            // product-gated WiFi button; every v2 control is hidden (its
            // requests would be protocol violations and its events never
            // arrive).
            view.show_volume = false;
            view.night_mode = None;
            view.show_restart = false;
            view.wifi_button = wifi_reconfig_supported(self.product);
        }
        view
    }
}

/// Panel height in logical pixels. The tray is always display-sized, so this is
/// the full display height; it sets the slide-animation travel distance.
fn panel_height_for(height: u32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "display height is well within f32 mantissa precision"
    )]
    let h = height as f32;
    h
}

impl SettingsTrayOverlay {
    /// Fold a changed connectivity snapshot into the view fields and set
    /// `content_dirty`. The version gate makes the unchanged case free of
    /// allocations, so this is safe to run on every ~30 Hz animation tick; no
    /// change yet (prober has not published) keeps the current placeholders.
    fn refresh_network(&mut self) {
        let Some(VersionedSnapshot { version, snapshot }) =
            self.env.snapshot_if_changed(self.snapshot_version)
        else {
            return;
        };
        self.snapshot_version = Some(version);
        let ip = snapshot.ipv4.as_ref().map(Ipv4Addr::to_string);
        let signal_band_changed =
            ui::signal_band(snapshot.wifi_signal_dbm) != ui::signal_band(self.wifi_signal);
        let content_changed =
            ip != self.ip || snapshot.station_ssid != self.ssid || signal_band_changed;
        self.ip = ip;
        self.wifi_signal = snapshot.wifi_signal_dbm;
        self.ssid = snapshot.station_ssid;
        self.content_dirty |= content_changed;
    }

    /// Advance both hold FSMs from the tree's press state and queue the
    /// request each one confirms.
    fn advance_buttons(&mut self, now: Instant) {
        let reconfig_pressed = self.render_state.tree.is_pressed(ui::WIFI_RECONFIG_KEY);
        let prev = self.button;
        if self.button.tick(reconfig_pressed, now) == FsmAction::SendReconfigure {
            self.pending_requests.push(SettingsRequest::ReconfigureWifi);
        }
        if self.button != prev {
            self.content_dirty = true;
        }

        let restart_pressed = self.render_state.tree.is_pressed(ui::RESTART_KEY);
        let prev_restart = self.restart;
        if self.restart.tick(restart_pressed, now) == RestartAction::SendRestart {
            self.pending_requests.push(SettingsRequest::Restart);
        }
        if self.restart != prev_restart {
            self.content_dirty = true;
        }
        if !self.restart.shows_message() {
            self.declined_reason = None;
        }
    }

    /// Whether an incoming volume event must be dropped: a recent step tap's
    /// settle window is still open.
    fn volume_echo_blocked(&self, now: Instant) -> bool {
        self.volume_settle_until.is_some_and(|t| now < t)
    }

    /// Whether an incoming brightness event must be dropped: a recent step
    /// tap's settle window is still open.
    fn brightness_echo_blocked(&self, now: Instant) -> bool {
        self.brightness_settle_until.is_some_and(|t| now < t)
    }

    /// [`SystemOverlay::on_volume`] with an explicit `now`, so tests drive
    /// the settle window deterministically.
    fn on_volume_at(&mut self, value: u8, now: Instant) {
        if self.volume_echo_blocked(now) {
            return;
        }
        if value != self.volume {
            self.volume = value;
            self.content_dirty = true;
        }
    }

    /// [`SystemOverlay::on_brightness`] with an explicit `now`, so tests
    /// drive the settle window deterministically.
    fn on_brightness_at(&mut self, value: u8, now: Instant) {
        if self.brightness_echo_blocked(now) {
            return;
        }
        if value != self.brightness {
            self.brightness = value;
            self.content_dirty = true;
        }
    }

    /// [`SystemOverlay::on_touch`] with an explicit `now`, so tests drive the
    /// inactivity timer on one injected timeline.
    fn on_touch_at(&mut self, event: TouchEvent, now: Instant) {
        if !self.slide.accepts_input() {
            return;
        }
        if matches!(event, TouchEvent::Up { .. })
            && self
                .touch_track
                .is_some_and(|track| dismiss::classify(track.start, track.latest))
        {
            self.begin_dismiss();
            return;
        }
        self.render_state.tree.push_touch(event);
        self.last_interaction = now;
        // Force a render so the interaction state processes the queued event and
        // runs its hit-test; without a paint frame the controls never see
        // the touch (the dismiss path below works off raw deltas, not hit-tests).
        self.content_dirty = true;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "surface-local logical coordinates fit f32 comfortably"
        )]
        match event {
            TouchEvent::Down { x, y, .. } => {
                let pt = Pt {
                    x: x as f32,
                    y: y as f32,
                };
                self.touch_track = Some(TouchTrack {
                    start: pt,
                    latest: pt,
                });
            }
            TouchEvent::Motion { x, y, .. } => {
                if let Some(track) = self.touch_track.as_mut() {
                    track.latest = Pt {
                        x: x as f32,
                        y: y as f32,
                    };
                }
            }
            TouchEvent::Up { .. } | TouchEvent::Cancel => {
                self.touch_track = None;
            }
        }
    }

    /// Apply a frame's interaction read-back to overlay state and queue the
    /// resulting requests.
    fn apply_render_output(&mut self, output: SettingsTrayRenderOutput, now: Instant) {
        if let Some(step) = output.brightness_step {
            let b = step_value(self.brightness, step, ui::MIN_BRIGHTNESS, 100);
            self.brightness = b;
            self.repaint_queued = true;
            self.pending_requests
                .push(SettingsRequest::SetBrightness(b));
            self.brightness_settle_until = Some(now + STEP_ECHO_SETTLE);
        }
        if let Some(step) = output.volume_step {
            let v = step_value(self.volume, step, 0, 100);
            self.volume = v;
            self.repaint_queued = true;
            self.pending_requests.push(SettingsRequest::SetVolume(v));
            self.volume_settle_until = Some(now + STEP_ECHO_SETTLE);
        }
        if output.night_mode_tapped {
            self.pending_requests.push(SettingsRequest::ToggleNightMode);
        }
        if output.close_tapped {
            self.begin_dismiss();
        }
        if output.pressed_changed && self.slide.accepts_input() {
            self.repaint_queued = true;
        }
    }

    /// Arm the dismiss ramp and freeze interaction state before its first blit.
    fn begin_dismiss(&mut self) {
        if self.slide.is_dismissing() {
            return;
        }
        self.dismissing = true;
        self.slide.start_dismiss();
        self.render_state.tree.cancel_touch();
        self.touch_track = None;
        self.repaint_queued = false;
    }

    /// Whether a hold FSM is mid-animation or a hold button is pressed this
    /// frame, so the loop should fast-poll to keep the hold accruing.
    fn animating(&self) -> bool {
        self.button.is_animating()
            || self.restart.is_animating()
            || self.render_state.tree.is_pressed(ui::WIFI_RECONFIG_KEY)
            || self.render_state.tree.is_pressed(ui::RESTART_KEY)
    }
}

impl SystemOverlay for SettingsTrayOverlay {
    fn layer_config(&self) -> LayerConfig {
        // Built explicitly (not via LayerConfig::fullscreen, which sets
        // Layer::Top): the tray needs Layer::Overlay, the rank reserved for it,
        // and a full input region so a tap below the panel is delivered here.
        LayerConfig {
            layer: Layer::Overlay,
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            size: (0, 0),
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            exclusive_zone: 0,
            namespace: "bmc-settings-tray".to_owned(),
            input: InputRegion::Full,
        }
    }

    fn screen_edge(&self) -> Option<ScreenEdge> {
        Some(ScreenEdge::Top)
    }

    fn uses_settings(&self) -> bool {
        true
    }

    fn on_reveal(&mut self) {
        let now = Instant::now();
        // The panel cache may still show pre-hide transient UI (hold progress,
        // a decline message); when this reset changes content, repaint
        // instead of blitting the stale cache through the reveal ramp.
        if self.button != ButtonState::default()
            || self.restart != RestartState::default()
            || self.declined_reason.is_some()
        {
            self.content_dirty = true;
        }
        self.button = ButtonState::default();
        self.restart = RestartState::default();
        self.declined_reason = None;
        self.touch_track = None;
        self.dismissing = false;
        self.last_interaction = now;
        self.slide.start_reveal();
    }

    fn on_capabilities(&mut self, caps: bmc_system_overlay::SettingsCaps) {
        if self.caps != Some(caps) {
            self.caps = Some(caps);
            self.content_dirty = true;
        }
    }

    fn on_brightness(&mut self, value: u8) {
        self.on_brightness_at(value, Instant::now());
    }

    fn on_volume(&mut self, value: u8) {
        self.on_volume_at(value, Instant::now());
    }

    fn on_night_mode(&mut self, active: bool, until: Option<&str>) {
        if active != self.night_active || until != self.night_until.as_deref() {
            self.night_active = active;
            self.night_until = until.map(str::to_owned);
            self.content_dirty = true;
        }
    }

    fn on_restart_declined(&mut self, reason: &str) {
        self.declined_reason = Some(reason.to_owned());
        self.restart.on_declined(Instant::now());
        self.content_dirty = true;
    }

    fn on_wifi_ap(&mut self, ssid: Option<&str>) {
        // An empty SSID means setup is inactive; treat it as `None` so the setup
        // view shows only for a real AP, independent of upstream filtering.
        let ssid = ssid.filter(|s| !s.is_empty());
        self.setup_ssid = ssid.map(str::to_owned);
        self.button.on_wifi_ap(ssid.is_some());
        self.content_dirty = true;
    }

    fn on_touch(&mut self, event: TouchEvent) {
        self.on_touch_at(event, Instant::now());
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        self.refresh_network();
        // Read-back changes queue here instead of setting content_dirty
        // directly: the host consumes content_dirty right after the paint the
        // read-back came from, which was built before the change.
        if std::mem::take(&mut self.repaint_queued) {
            self.content_dirty = true;
        }
        let was_dirty = self.content_dirty;
        if self.slide.accepts_input() {
            self.advance_buttons(now);
        }

        // A stationary finger emits no wl_touch events, so on_touch cannot
        // refresh the activity timer; treat any in-progress touch as activity
        // so the tray never dismisses out from under a held finger. The
        // timeout counts from finger-up.
        if self.touch_track.is_some() {
            self.last_interaction = now;
        } else if now.duration_since(self.last_interaction) >= INACTIVITY_TIMEOUT {
            self.begin_dismiss();
        }
        // Report not-visible only once the panel has fully slid off, keeping
        // the surface mapped for the duration of the animation.
        self.slide.advance(now);

        let sliding = self.slide.animating(now);
        let visible = !(self.dismissing && self.slide.dismiss_done(now));
        let animating = self.animating() || sliding;
        let wants_render = visible
            && (was_dirty || self.content_dirty || animating || self.slide.needs_settle_frame());
        // Fast-poll whenever we render this pass. `render` flips the tree's
        // `is_pressed` and advances a hold FSM only on the *next* tick, so a
        // finger-down on a hold-to-confirm button — still unpressed in this
        // pass's pre-render state — must be re-examined on the next frame, not
        // deferred to the 2 s network refresh (which would stall the hold).
        let next_wake = if !visible {
            None
        } else if wants_render {
            Some(now + FAST_WAKE)
        } else {
            Some(now + NETWORK_REFRESH)
        };
        TickOutcome {
            visible,
            wants_render,
            next_wake,
        }
    }

    fn prewarm(&mut self, renderer: &mut dyn Renderer) {
        // One full off-screen render at host startup, so the first screen-edge
        // reveal does not stall mid-swipe paying these one-time costs: the
        // Wi-Fi SVG icon compile/upload, rasterizing the panel's text into the
        // shared glyph atlas, and the first tree layout. `render_settings_tray`
        // registers the icons on its first line, so this single pass warms all
        // three; the painted pixels are discarded (the host exports no prewarm
        // buffer).
        let now = Instant::now();
        let mut view = self.view(now);
        view.show_volume = true;
        view.wifi_button = true;
        view.night_mode = Some(NightModeView {
            active: false,
            until: Some("06:30".to_owned()),
        });
        view.show_restart = true;
        let _ = render_settings_tray(
            renderer,
            (self.width, self.height),
            &mut self.render_state,
            &view,
            now,
        );
    }

    fn render(&mut self, renderer: &mut dyn Renderer, size: (u32, u32)) {
        let now = Instant::now();
        let view = self.view(now);
        let output = render_settings_tray(renderer, size, &mut self.render_state, &view, now);
        self.apply_render_output(output, now);
    }

    fn drain_settings_requests(&mut self) -> Vec<SettingsRequest> {
        std::mem::take(&mut self.pending_requests)
    }

    /// Retract when a modal full-screen overlay (a firing alarm, startup) takes
    /// the screen, so the tray never sits on top of it. Generic: the compositor
    /// derives this from any full-screen preempting overlay, so the tray does
    /// not bind — and does not need to know about — each such feature's
    /// protocol. `active == false` (the overlay cleared) needs no action; the
    /// user reopens the tray by pulling it down again.
    fn on_preempted(&mut self, active: bool) {
        if active {
            self.begin_dismiss();
        }
    }

    fn wants_cached_blit(&self, now: Instant) -> Option<f32> {
        self.slide
            .cached_blit_offset(now, self.content_dirty, self.panel_height)
    }

    fn uses_panel_cache(&self) -> bool {
        true
    }

    fn on_frame_submitted(&mut self, now: Instant) {
        self.slide.anchor(now);
        // A frame submitted while settling presented the settled offset (blit
        // or full paint), so the reveal is done. `anchor` and `mark_settled`
        // act on disjoint phases, so their order is irrelevant.
        self.slide.mark_settled();
    }

    fn take_content_dirty(&mut self) -> bool {
        std::mem::take(&mut self.content_dirty)
    }

    fn content_dirty(&self) -> bool {
        self.content_dirty
    }

    fn mark_content_dirty(&mut self) {
        self.content_dirty = true;
    }
}

const PRESSABLE: [&str; 7] = [
    ui::VOLUME_DOWN_KEY,
    ui::VOLUME_UP_KEY,
    ui::BRIGHTNESS_DOWN_KEY,
    ui::BRIGHTNESS_UP_KEY,
    ui::NIGHT_MODE_KEY,
    ui::RESTART_KEY,
    ui::WIFI_RECONFIG_KEY,
];

pub fn render_settings_tray(
    renderer: &mut dyn Renderer,
    size: (u32, u32),
    state: &mut SettingsTrayRenderState,
    view: &SettingsTrayView,
    now: Instant,
) -> SettingsTrayRenderOutput {
    let icons = *state
        .icons
        .get_or_insert_with(|| icons::register_icons(renderer));

    let delta_ms =
        u32::try_from(now.duration_since(state.last_render).as_millis()).unwrap_or(u32::MAX);
    state.last_render = now;

    // Sampled before the render: the tree the render lays out was built from
    // this state, so it is the baseline `pressed_changed` compares against.
    let pressed_derived = PRESSABLE.iter().copied().find(|k| state.tree.is_pressed(k));
    let pressed = view.pressed_key.as_deref().or(pressed_derived);

    let wifi_view = if let Some(ssid) = view.setup_ssid.as_deref() {
        WifiView::Setup { ap_ssid: ssid }
    } else {
        WifiView::Idle
    };
    let controls = ui::Controls {
        brightness: view.show_brightness.then_some(view.brightness),
        volume: view.show_volume.then_some(view.volume),
        night_mode: view.night_mode.as_ref().map(|n| ui::NightMode {
            active: n.active,
            until: n.until.as_deref(),
        }),
        restart: view.show_restart.then_some(ui::HoldControl {
            caption: view.restart_caption.as_deref(),
            progress: view.restart_progress,
        }),
        wifi_reconfig: ui::HoldControl {
            caption: view.reconfig_caption.as_deref(),
            progress: view.reconfig_progress,
        },
        pressed,
    };
    let node = ui::build_tree(
        view.hostname.as_deref(),
        view.ip.as_deref(),
        view.wifi_signal,
        view.ssid.as_deref(),
        icons.wifi,
        Panel {
            shape: view.shape,
            width: view.width,
            height: view.height,
            wifi_button: view.wifi_button,
        },
        wifi_view,
        icons.controls,
        controls,
    );

    let result = match state.tree.render(&node, size, delta_ms, renderer) {
        Ok(result) => result,
        Err(err) => {
            tracing::error!("settings-tray tree render failed: {err}");
            return SettingsTrayRenderOutput::default();
        }
    };

    // The render processed this frame's queued touch events, so the pressed
    // key can differ from the pre-render sample; that difference is what
    // demands the repaint showing the press/release inversion.
    let pressed_after = PRESSABLE.iter().copied().find(|k| state.tree.is_pressed(k));
    let step_of = |down: &str, up: &str| {
        if result.clicks.contains_key(up) {
            Some(Step::Up)
        } else if result.clicks.contains_key(down) {
            Some(Step::Down)
        } else {
            None
        }
    };
    SettingsTrayRenderOutput {
        brightness_step: step_of(ui::BRIGHTNESS_DOWN_KEY, ui::BRIGHTNESS_UP_KEY),
        volume_step: step_of(ui::VOLUME_DOWN_KEY, ui::VOLUME_UP_KEY),
        night_mode_tapped: result.clicks.contains_key(ui::NIGHT_MODE_KEY),
        close_tapped: result.clicks.contains_key(ui::CLOSE_KEY),
        pressed_changed: pressed_after != pressed_derived,
    }
}

#[cfg(test)]
mod view_tests {
    use bmc_system_overlay::Snapshot;

    use super::*;

    #[test]
    fn view_contains_runtime_state_and_setup_ap_status() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(
            Product::Bmc100,
            Some("braiins-deck".to_owned()),
            now,
        );
        overlay.brightness = 70;
        overlay.ip = Some("192.168.1.42".to_owned());
        overlay.wifi_signal = Some(-52);
        overlay.ssid = Some("Braiins-WiFi".to_owned());
        overlay.on_wifi_ap(Some("Deck setup"));

        let view = overlay.view(now);

        assert_eq!(view.brightness, 70);
        assert_eq!(view.hostname.as_deref(), Some("braiins-deck"));
        assert_eq!(view.ip.as_deref(), Some("192.168.1.42"));
        assert_eq!(view.wifi_signal, Some(-52));
        assert_eq!(view.ssid.as_deref(), Some("Braiins-WiFi"));
        assert_eq!(view.setup_ssid.as_deref(), Some("Deck setup"));
        assert!(view.wifi_button);
        assert_eq!(
            view.reconfig_caption, None,
            "an active setup AP silences the reconfigure caption"
        );
    }

    #[test]
    fn night_mode_event_reflects_into_view() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        // Night mode shows only on a v2 compositor; establish one so the event
        // is not gated off by the v1 fallback.
        overlay.on_capabilities(bmc_system_overlay::SettingsCaps {
            brightness: true,
            sound: true,
            wifi_setup: true,
        });
        overlay.on_night_mode(true, Some("06:30"));
        assert_eq!(
            overlay.view(now).night_mode,
            Some(NightModeView {
                active: true,
                until: Some("06:30".to_owned()),
            }),
            "the night_mode event must reflect verbatim into the view"
        );
    }

    #[test]
    fn empty_setup_ap_ssid_does_not_enter_setup_view() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.on_wifi_ap(Some(""));
        assert_eq!(
            overlay.view(now).setup_ssid,
            None,
            "an empty AP SSID means setup inactive — the setup view must not show"
        );
    }

    #[test]
    fn v1_compositor_falls_back_to_product_gates_and_hides_v2_controls() {
        let now = Instant::now();
        let overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        // No capabilities event ever arrived (v1 compositor).
        let view = overlay.view(now);
        assert!(!view.show_volume, "volume is v2-only");
        assert!(view.night_mode.is_none(), "night mode is v2-only");
        assert!(!view.show_restart, "restart is v2-only");
        assert!(
            view.wifi_button,
            "BMC100 keeps its product-gated WiFi button"
        );
    }

    #[test]
    fn capabilities_event_supersedes_the_local_product_gate() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.on_capabilities(bmc_system_overlay::SettingsCaps {
            brightness: true,
            sound: false,
            wifi_setup: false,
        });
        let view = overlay.view(now);
        assert!(!view.show_volume);
        assert!(
            !view.wifi_button,
            "the compositor's word beats wifi_reconfig_supported(product)"
        );
        assert!(
            view.night_mode.is_some(),
            "night mode shows on every v2 compositor"
        );
        assert!(view.show_restart, "restart shows on every v2 compositor");
    }

    #[test]
    fn view_for_product_exposes_bmm101_dimensions_for_stories() {
        let view = SettingsTrayView::for_product(SettingsTrayProduct::Bmm101);

        assert_eq!(view.width, 480);
        assert_eq!(view.height, 320);
        assert!(!view.wifi_button);
    }

    struct StaticEnv {
        snapshot: Option<Snapshot>,
    }

    impl Env for StaticEnv {
        fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
            // Mimic the prober contract: the fixed snapshot is the first
            // version, so a caller that has folded it in gets no re-read.
            if seen.is_some() {
                return None;
            }
            self.snapshot.clone().map(|snapshot| VersionedSnapshot {
                version: SnapshotVersion::FIRST,
                snapshot,
            })
        }
    }

    #[test]
    fn tick_reflects_snapshot_into_view() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.env = Box::new(StaticEnv {
            snapshot: Some(Snapshot {
                ipv4: Some(Ipv4Addr::new(192, 168, 1, 42)),
                station_ssid: Some("Braiins-WiFi".to_owned()),
                wifi_signal_dbm: Some(-52),
            }),
        });

        let _ = overlay.tick(now);
        let view = overlay.view(now);

        assert_eq!(view.ip.as_deref(), Some("192.168.1.42"));
        assert_eq!(view.ssid.as_deref(), Some("Braiins-WiFi"));
        assert_eq!(view.wifi_signal, Some(-52));
    }

    // The versioned read only pays off if the ~30 Hz animation ticks stop
    // re-reading an unchanged snapshot, and that requires the overlay to hand
    // the version it last folded in back to the source.
    #[test]
    fn tick_passes_folded_version_back_to_the_source() {
        struct VersionAssertEnv;
        impl Env for VersionAssertEnv {
            fn snapshot_if_changed(
                &self,
                seen: Option<SnapshotVersion>,
            ) -> Option<VersionedSnapshot> {
                let Some(seen) = seen else {
                    return Some(VersionedSnapshot {
                        version: SnapshotVersion::FIRST,
                        snapshot: Snapshot {
                            ipv4: None,
                            station_ssid: Some("Braiins-WiFi".to_owned()),
                            wifi_signal_dbm: None,
                        },
                    });
                };
                assert_eq!(
                    seen,
                    SnapshotVersion::FIRST,
                    "the overlay must echo the version it folded in"
                );
                None
            }
        }

        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.env = Box::new(VersionAssertEnv);

        let _ = overlay.tick(now);
        let _ = overlay.tick(now + FAST_WAKE);
    }

    #[test]
    fn unknown_snapshot_keeps_placeholders() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.env = Box::new(StaticEnv { snapshot: None });

        let _ = overlay.tick(now);
        let view = overlay.view(now);

        assert_eq!(view.ip, None);
        assert_eq!(view.ssid, None);
        assert_eq!(view.wifi_signal, None);
    }

    #[test]
    fn signal_jitter_within_one_icon_band_does_not_dirty_content() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.ip = Some("192.168.1.42".to_owned());
        overlay.ssid = Some("Braiins-WiFi".to_owned());
        overlay.wifi_signal = Some(-52);
        overlay.env = Box::new(StaticEnv {
            snapshot: Some(Snapshot {
                ipv4: Some(Ipv4Addr::new(192, 168, 1, 42)),
                station_ssid: Some("Braiins-WiFi".to_owned()),
                wifi_signal_dbm: Some(-57),
            }),
        });
        let _ = overlay.take_content_dirty();

        let _ = overlay.tick(now);

        assert_eq!(overlay.wifi_signal, Some(-57), "retain the latest reading");
        assert!(
            !overlay.content_dirty(),
            "unchanged rendered signal band must keep the panel cache clean"
        );
    }

    #[test]
    fn signal_crossing_into_another_icon_band_dirties_content() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.wifi_signal = Some(-52);
        overlay.env = Box::new(StaticEnv {
            snapshot: Some(Snapshot {
                ipv4: None,
                station_ssid: None,
                wifi_signal_dbm: Some(-76),
            }),
        });
        let _ = overlay.take_content_dirty();

        let _ = overlay.tick(now);

        assert!(
            overlay.content_dirty(),
            "a different rendered signal band must refresh the panel cache"
        );
    }
}

#[cfg(test)]
mod wake_tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct StaticEnv;
    impl Env for StaticEnv {
        fn snapshot_if_changed(&self, _seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
            None
        }
    }

    // A finger-down on a hold-to-confirm button only becomes `is_pressed` during
    // the next `render`, so the hold FSM advances a frame later. The tick that
    // schedules the wake must fast-poll right after a touch-down; otherwise the
    // just-rendered press state isn't re-examined until the 2 s network refresh
    // and the hold timer and its progress stall.
    #[test]
    fn finger_down_schedules_a_fast_wake() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);
        overlay.env = Box::new(StaticEnv);
        // Drop the construction-time dirty flag the way a first render would.
        let _ = overlay.take_content_dirty();

        // Idle baseline: nothing pending, so the slow network cadence applies.
        let t1 = t0 + Duration::from_millis(10);
        assert_eq!(overlay.tick(t1).next_wake, Some(t1 + NETWORK_REFRESH));

        // Finger-down on the wifi button row; the press lands on the next render.
        overlay.on_touch(TouchEvent::Down {
            id: 0,
            x: 120.0,
            y: 300.0,
        });
        let t2 = t1 + Duration::from_millis(10);
        assert!(
            overlay
                .tick(t2)
                .next_wake
                .is_some_and(|w| w <= t2 + FAST_WAKE),
            "a touch-down must fast-poll so the hold FSM picks up the press promptly"
        );
    }
}

#[cfg(test)]
mod volume_echo_tests {
    use super::*;

    fn overlay() -> SettingsTrayOverlay {
        SettingsTrayOverlay::new_for_product(Product::Bmc100, None, Instant::now())
    }

    #[test]
    fn echo_is_dropped_within_the_settle_window_and_honored_after() {
        let t0 = Instant::now();
        let mut o = overlay();
        o.volume = 40;
        o.apply_render_output(
            SettingsTrayRenderOutput {
                volume_step: Some(Step::Up),
                ..Default::default()
            },
            t0,
        );
        assert_eq!(o.volume, 50);
        o.on_volume_at(50, t0 + STEP_ECHO_SETTLE / 2);
        assert_eq!(
            o.volume, 50,
            "echo within the settle window must be dropped"
        );

        o.on_volume_at(70, t0 + STEP_ECHO_SETTLE);
        assert_eq!(o.volume, 70, "external changes resume after the window");
    }

    #[test]
    fn idle_event_moves_the_knob() {
        let mut o = overlay();
        o.on_volume(15);
        assert_eq!(o.volume, 15);
    }
}

#[cfg(test)]
mod step_tests {
    use super::*;

    fn volume_up() -> SettingsTrayRenderOutput {
        SettingsTrayRenderOutput {
            volume_step: Some(Step::Up),
            ..Default::default()
        }
    }

    #[test]
    fn steps_clamp_to_their_ranges() {
        assert_eq!(step_value(95, Step::Up, ui::MIN_BRIGHTNESS, 100), 100);
        assert_eq!(step_value(15, Step::Down, ui::MIN_BRIGHTNESS, 100), 10);
        assert_eq!(step_value(5, Step::Down, 0, 100), 0);
        assert_eq!(step_value(40, Step::Up, 0, 100), 50);
    }

    #[test]
    fn volume_taps_compound_locally_and_extend_the_settle_window() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);
        overlay.volume = 40;
        overlay.apply_render_output(volume_up(), t0);
        let t1 = t0 + Duration::from_millis(300);
        overlay.apply_render_output(volume_up(), t1);
        let t2 = t1 + Duration::from_millis(300);
        overlay.apply_render_output(volume_up(), t2);
        assert_eq!(overlay.volume, 70, "three +10 taps compound from 40");
        let sent: Vec<_> = overlay.drain_settings_requests();
        assert_eq!(
            sent,
            vec![
                SettingsRequest::SetVolume(50),
                SettingsRequest::SetVolume(60),
                SettingsRequest::SetVolume(70),
            ]
        );
        overlay.on_volume_at(50, t2 + Duration::from_millis(100));
        assert_eq!(
            overlay.volume, 70,
            "a stale echo inside the settle window is dropped"
        );
        assert!(
            overlay.volume_echo_blocked(t0 + STEP_ECHO_SETTLE + Duration::from_millis(100)),
            "the last tap extended the window past the first tap's deadline"
        );
        assert!(!overlay.volume_echo_blocked(t2 + STEP_ECHO_SETTLE));
    }

    #[test]
    fn brightness_taps_compound_locally_and_extend_the_settle_window() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);
        overlay.brightness = 40;
        let brightness_up = || SettingsTrayRenderOutput {
            brightness_step: Some(Step::Up),
            ..Default::default()
        };
        overlay.apply_render_output(brightness_up(), t0);
        let t1 = t0 + Duration::from_millis(300);
        overlay.apply_render_output(brightness_up(), t1);
        let t2 = t1 + Duration::from_millis(300);
        overlay.apply_render_output(brightness_up(), t2);
        assert_eq!(overlay.brightness, 70, "three +10 taps compound from 40");
        let sent: Vec<_> = overlay.drain_settings_requests();
        assert_eq!(
            sent,
            vec![
                SettingsRequest::SetBrightness(50),
                SettingsRequest::SetBrightness(60),
                SettingsRequest::SetBrightness(70),
            ]
        );
        overlay.on_brightness_at(50, t2 + Duration::from_millis(100));
        assert_eq!(
            overlay.brightness, 70,
            "a stale echo inside the settle window is dropped"
        );
        assert!(
            overlay.brightness_echo_blocked(t0 + STEP_ECHO_SETTLE + Duration::from_millis(100)),
            "the last tap extended the window past the first tap's deadline"
        );
        assert!(!overlay.brightness_echo_blocked(t2 + STEP_ECHO_SETTLE));
    }

    #[test]
    fn boundary_taps_stay_clamped_but_still_send() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.volume = 100;
        overlay.brightness = ui::MIN_BRIGHTNESS;
        overlay.apply_render_output(volume_up(), now);
        overlay.apply_render_output(
            SettingsTrayRenderOutput {
                brightness_step: Some(Step::Down),
                ..Default::default()
            },
            now,
        );
        assert_eq!(overlay.volume, 100);
        assert_eq!(overlay.brightness, ui::MIN_BRIGHTNESS);
        let sent: Vec<_> = overlay.drain_settings_requests();
        assert_eq!(
            sent,
            vec![
                SettingsRequest::SetVolume(100),
                SettingsRequest::SetBrightness(ui::MIN_BRIGHTNESS),
            ],
            "a tap at the clamp boundary re-sends the clamped absolute value"
        );
    }

    #[test]
    fn close_tap_starts_the_dismiss_slide_next_tick() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.apply_render_output(
            SettingsTrayRenderOutput {
                close_tapped: true,
                ..Default::default()
            },
            now,
        );
        let outcome = overlay.tick(now + Duration::from_millis(1));
        assert!(overlay.slide.is_dismissing());
        assert!(
            outcome.visible,
            "the surface stays mapped while sliding out"
        );
    }

    #[test]
    fn held_finger_defers_inactivity_dismiss_until_release() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);

        // Finger down, then held perfectly still: libinput emits no further
        // touch events, so on_touch never runs again to refresh the timer.
        overlay.on_touch_at(
            TouchEvent::Down {
                id: 0,
                x: 120.0,
                y: 300.0,
            },
            t0,
        );

        // A tick well past the inactivity timeout must not dismiss while the
        // finger is still down — the in-progress touch counts as activity.
        let _ = overlay.tick(t0 + INACTIVITY_TIMEOUT + Duration::from_secs(5));
        assert!(
            !overlay.slide.is_dismissing(),
            "the tray must not dismiss out from under a held finger"
        );

        // Release in place (no swipe): the touch ends and the timeout starts
        // counting fresh from finger-up.
        let released = t0 + INACTIVITY_TIMEOUT + Duration::from_secs(5);
        overlay.on_touch_at(TouchEvent::Up { id: 0 }, released);
        let _ = overlay.tick(released + Duration::from_millis(1));
        assert!(
            !overlay.slide.is_dismissing(),
            "the inactivity window restarts from finger-up"
        );

        // Once the window elapses after release, the tray dismisses as before.
        let _ = overlay.tick(released + INACTIVITY_TIMEOUT + Duration::from_millis(1));
        assert!(
            overlay.slide.is_dismissing(),
            "the tray still auto-dismisses once idle after the touch ends"
        );
    }

    #[test]
    fn cancelled_touch_still_auto_dismisses() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);

        // Finger down, then the compositor cancels the sequence (no Up): the
        // Cancel arm must clear touch_track, otherwise the leaked Some would
        // refresh last_interaction every tick and the tray never dismisses.
        overlay.on_touch_at(
            TouchEvent::Down {
                id: 0,
                x: 120.0,
                y: 300.0,
            },
            t0,
        );
        overlay.on_touch_at(TouchEvent::Cancel, t0 + Duration::from_millis(1));

        // With the track cleared, the inactivity window elapses and dismisses.
        let _ = overlay.tick(t0 + INACTIVITY_TIMEOUT + Duration::from_millis(1));
        assert!(
            overlay.slide.is_dismissing(),
            "a cancelled touch must not wedge the inactivity timer open"
        );
    }

    #[test]
    fn read_back_changes_repaint_on_the_next_tick() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.content_dirty = false;
        overlay.apply_render_output(volume_up(), now);
        assert!(
            !overlay.content_dirty,
            "read-back must not set content_dirty in the same pass (the host \
             consumes it right after the stale paint)"
        );
        let outcome = overlay.tick(now + Duration::from_millis(10));
        assert!(
            overlay.content_dirty,
            "the queued repaint becomes a dirty tick"
        );
        assert!(outcome.wants_render);

        overlay.content_dirty = false;
        overlay.apply_render_output(
            SettingsTrayRenderOutput {
                pressed_changed: true,
                ..Default::default()
            },
            now,
        );
        assert!(overlay.tick(now + Duration::from_millis(20)).wants_render);
    }
}

#[cfg(test)]
mod slide_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn reveal_holds_offscreen_until_anchored_then_eases_to_zero() {
        let t0 = Instant::now();
        let mut s = Slide::default();
        s.start_reveal();
        // Pending: the clock has not started, however much time passes.
        assert!(s.offset(t0 + Duration::from_millis(500), 200.0) <= -199.0);
        assert!(s.animating(t0 + Duration::from_millis(500)));
        let t1 = t0 + Duration::from_millis(60);
        s.anchor(t1);
        assert!(s.offset(t1, 200.0) <= -199.0);
        assert!(s.offset(t1 + Duration::from_millis(200), 200.0).abs() < 1e-3);
        assert!(!s.animating(t1 + Duration::from_millis(200)));
    }

    #[test]
    fn dismiss_holds_at_rest_until_anchored_then_eases_offscreen() {
        let t0 = Instant::now();
        let mut s = Slide::default();
        s.start_reveal();
        s.anchor(t0);
        s.start_dismiss();
        assert!(
            s.is_dismissing(),
            "pending dismiss must count as dismissing"
        );
        // Pending: panel rests at the settled position, clock not started.
        assert!(s.offset(t0 + Duration::from_millis(500), 200.0).abs() < 1e-3);
        assert!(!s.dismiss_done(t0 + Duration::from_millis(500)));
        // A clean pending dismiss is blit-eligible at rest — an inactivity
        // dismiss must not force a paint; a real content change still must.
        assert_eq!(
            s.cached_blit_offset(t0 + Duration::from_millis(10), false, 200.0),
            Some(0.0)
        );
        assert_eq!(
            s.cached_blit_offset(t0 + Duration::from_millis(10), true, 200.0),
            None
        );
        let t1 = t0 + Duration::from_millis(60);
        s.anchor(t1);
        assert!(s.offset(t1 + Duration::from_millis(400), 200.0) <= -199.0);
        assert!(s.dismiss_done(t1 + Duration::from_millis(400)));
    }

    #[test]
    fn anchor_is_a_noop_outside_pending_phases() {
        let t0 = Instant::now();
        let mut s = Slide::default();
        s.anchor(t0);
        assert_eq!(s.phase, SlidePhase::Idle);
        s.start_reveal();
        s.anchor(t0);
        let mid = t0 + Duration::from_millis(90);
        s.anchor(mid); // must not restart the running ramp
        assert_eq!(s.phase, SlidePhase::Revealing { since: t0 });
        assert!(
            s.offset(mid, 200.0) > -199.0,
            "ramp advanced from t0, not restarted"
        );
    }

    #[test]
    fn pending_reveal_keeps_requesting_frames_after_dirty_consumed() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);
        overlay.on_reveal();
        let _ = overlay.take_content_dirty();
        assert!(
            overlay.tick(t0 + Duration::from_millis(5)).wants_render,
            "a stall between reveal and first paint must not strand the pending phase"
        );
    }

    #[test]
    fn transition_phases_discard_touch_without_dirtying_content() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);

        overlay.on_reveal();
        let _ = overlay.take_content_dirty();
        overlay.on_touch(TouchEvent::Down {
            id: 0,
            x: 120.0,
            y: 100.0,
        });
        assert!(
            !overlay.content_dirty(),
            "reveal-pending input must not invalidate the cached panel"
        );

        overlay.slide.start_dismiss();
        overlay.on_touch(TouchEvent::Motion {
            id: 0,
            x: 120.0,
            y: 40.0,
        });
        assert!(
            !overlay.content_dirty(),
            "dismiss-pending input must not invalidate the cached panel"
        );
    }

    #[test]
    fn dismiss_swipe_arms_pending_phase_without_dirtying_release() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);
        let _ = overlay.take_content_dirty();

        overlay.on_touch(TouchEvent::Down {
            id: 0,
            x: 120.0,
            y: 150.0,
        });
        let _ = overlay.take_content_dirty();
        overlay.on_touch(TouchEvent::Motion {
            id: 0,
            x: 120.0,
            y: 60.0,
        });
        let _ = overlay.take_content_dirty();

        overlay.on_touch(TouchEvent::Up { id: 0 });

        assert_eq!(overlay.slide.phase, SlidePhase::DismissPending);
        assert!(
            !overlay.content_dirty(),
            "the release that starts dismissal must preserve the cached panel"
        );
    }

    #[test]
    fn first_presented_frame_anchors_the_reveal_ramp() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);
        overlay.on_reveal();
        let _ = overlay.take_content_dirty();
        let t1 = t0 + Duration::from_millis(60);
        overlay.on_frame_submitted(t1);
        // Half-way through the ramp measured from t1, the eased offset is
        // -0.125*height. A trigger-time anchor (t0) would be ~150/180 elapsed
        // and nearly settled (~ -0.004*height), so demand a deep mid-flight
        // offset to distinguish the two.
        #[expect(clippy::cast_precision_loss, reason = "display height fits f32")]
        let h = overlay.view(t1).height as f32;
        let mid = overlay.wants_cached_blit(t1 + Duration::from_millis(90));
        assert!(
            mid.is_some_and(|off| off < -h / 16.0),
            "ramp must be anchored at t1, not the reveal trigger: {mid:?}"
        );
    }

    #[test]
    fn dismiss_eases_back_offscreen_then_reports_done() {
        let t0 = Instant::now();
        let mut s = Slide::default();
        s.start_reveal();
        s.anchor(t0);
        s.start_dismiss();
        s.anchor(t0 + Duration::from_millis(200));
        assert!(s.offset(t0 + Duration::from_millis(200), 200.0).abs() < 1e-3);
        assert!(!s.dismiss_done(t0 + Duration::from_millis(200)));
        assert!(s.offset(t0 + Duration::from_millis(400), 200.0) <= -199.0);
        assert!(s.dismiss_done(t0 + Duration::from_millis(400)));
    }

    // The host loop can stall past the end of the reveal ramp (GPU-lock
    // contention, slow widget passes). The last presented frame is then
    // mid-ramp, and without a final settle frame the panel freezes short of
    // the settled position until an unrelated repaint.
    #[test]
    fn ramp_end_between_ticks_still_renders_the_settle_frame() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);
        overlay.on_reveal();
        // Drop the reveal-time dirty flag the way the first paint would.
        let _ = overlay.take_content_dirty();
        overlay.on_frame_submitted(t0);
        assert!(overlay.tick(t0 + Duration::from_millis(90)).wants_render);

        // No render ran before the next tick, which lands after the ramp end:
        // the settled frame has not been presented, so one more render is due.
        let outcome = overlay.tick(t0 + Duration::from_millis(400));
        assert!(
            outcome.wants_render,
            "the settle frame at offset 0 must render even when the ramp ends between ticks"
        );
    }

    #[test]
    fn settle_frame_blits_the_cache_then_a_submit_finishes_the_reveal() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);
        overlay.on_reveal();
        let _ = overlay.take_content_dirty();
        overlay.on_frame_submitted(t0);
        // A tick after the ramp end moves the slide into the settling phase.
        let after = t0 + Duration::from_millis(400);
        assert!(overlay.tick(after).wants_render);
        // The settle frame is a cached blit at offset 0, not a full paint.
        assert!(
            overlay
                .wants_cached_blit(after)
                .is_some_and(|off| off.abs() < 1e-3),
            "settle frame must blit at offset 0"
        );
        // Submitting that frame completes the reveal: no further render is due.
        overlay.on_frame_submitted(after);
        assert!(!overlay.tick(after + Duration::from_millis(50)).wants_render);
        assert!(
            overlay
                .wants_cached_blit(after + Duration::from_millis(50))
                .is_none()
        );
    }

    #[test]
    fn elapsed_reveal_settles_via_cached_blit() {
        let t0 = Instant::now();
        let mut s = Slide::default();
        s.start_reveal();
        s.anchor(t0);
        s.advance(t0 + Duration::from_millis(90));
        assert!(!s.needs_settle_frame(), "mid-ramp is not settling");

        let after = t0 + Duration::from_millis(300);
        s.advance(after);
        assert!(s.needs_settle_frame());
        // The settle frame blits the warm cache at the settled offset (0)
        // rather than full-painting.
        assert!(
            s.cached_blit_offset(after, false, 200.0)
                .is_some_and(|off| off.abs() < 1e-3),
            "settle frame must blit the cache at offset 0"
        );
        // A content-dirty settle frame still full-paints.
        assert_eq!(s.cached_blit_offset(after, true, 200.0), None);
        assert!(s.offset(after, 200.0).abs() < 1e-3);

        s.mark_settled();
        assert!(!s.needs_settle_frame());
    }

    #[test]
    fn dirty_frame_full_paints_then_clean_frame_blits_cache() {
        let t0 = Instant::now();
        let mut s = Slide::default();
        s.start_reveal();
        s.anchor(t0);
        let mid = t0 + Duration::from_millis(90);
        // First reveal frame is content-dirty: the host must full-paint (None).
        assert_eq!(s.cached_blit_offset(mid, true, 200.0), None);
        // Once a frame clears the dirty flag while still animating, the host
        // blits the cache at the eased offset (no paint).
        let blit = s.cached_blit_offset(mid, false, 200.0);
        assert!(blit.is_some_and(|off| (-200.0..0.0).contains(&off)));
        // After the ramp settles, no cached blit is requested even when clean.
        assert_eq!(
            s.cached_blit_offset(t0 + Duration::from_millis(200), false, 200.0),
            None
        );
    }

    #[test]
    fn content_dirty_accessor_is_non_consuming() {
        let mut overlay =
            SettingsTrayOverlay::new_for_product(Product::Bmc100, None, Instant::now());
        assert!(overlay.content_dirty(), "constructed dirty");
        assert!(overlay.content_dirty(), "observing must not consume");
        let _ = overlay.take_content_dirty();
        assert!(!overlay.content_dirty());
    }

    // The panel cache survives hides, so a reveal blits whatever was captured
    // before the previous unmap. When the reveal's state reset changes what
    // the panel would show (a hold was mid-progress when the tray hid), the
    // cache is stale and must be repainted, not blitted through the ramp.
    #[test]
    fn reveal_repaints_when_reset_discards_transient_ui() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);
        let _ = overlay.take_content_dirty();

        // Clean hide/reveal: cache already matches the reset state, keep the blit.
        overlay.on_reveal();
        assert!(
            !overlay.content_dirty(),
            "a reveal from a clean dismiss must not force a repaint"
        );

        overlay.button = ButtonState::Holding { since: t0 };
        overlay.on_reveal();
        assert!(
            overlay.content_dirty(),
            "discarding mid-hold UI must repaint the stale panel cache"
        );
    }
}
