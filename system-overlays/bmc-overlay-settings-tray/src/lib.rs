// Copyright (C) 2026  Braiins Systems s.r.o.

//! Swipe-from-top quick-settings overlay: a brightness slider plus WiFi station
//! info and hold-to-confirm reconfigure/reconnect buttons. Ported from the
//! `settings-stub` widget to a native `bmc-render` `TreeNode` overlay.
//!
//! The surface is fullscreen with a full input region so the tray blocks scene
//! swipes behind it while up. It dismisses on an upward swipe or after an
//! inactivity timeout.

mod dismiss;
mod fsm;
mod icons;
pub mod ui;

use std::net::Ipv4Addr;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use bmc_platform::{BmcInfo, DisplayShape, HardwareProfile, Product};
use bmc_render::renderer::Renderer;
use bmc_system_overlay::{
    Anchor, InputRegion, Layer, LayerConfig, ScreenEdge, SettingsRequest, SystemOverlay,
    TickOutcome, TouchEvent, TreeUi, primary_ipv4,
};

use crate::dismiss::Pt;
use crate::fsm::{ButtonState, FsmAction, ReconnectAction, ReconnectState};
use crate::ui::{Panel, WifiIcons, WifiView};

/// Kernel hostname, exposed by procfs.
const HOSTNAME_PATH: &str = "/proc/sys/kernel/hostname";

/// How often network info (IP / WiFi signal / SSID) is re-read while up.
const NETWORK_REFRESH: Duration = Duration::from_millis(2000);

/// Idle period after which the tray auto-dismisses.
const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(15);

/// Minimum spacing between brightness writes while dragging, so a drag does not
/// drive ~90 per-frame `set_brightness` config writes. A separate
/// release flush (see `flush_brightness_on_release`) guarantees the value the
/// finger lifted at is delivered past the throttle.
const BRIGHTNESS_SEND_INTERVAL: Duration = Duration::from_millis(80);

/// Fast wake cadence while a hold FSM is animating, so the hold/timeout edges
/// fire without a touch/network event to wake the loop.
const FAST_WAKE: Duration = Duration::from_millis(33);

/// Shell sequence the reconnect button runs: pulse the WiFi-reset GPIO, then
/// bounce the WiFi stack. Run via `sh` because the `$(gpiofind …)` substitution
/// and the inter-step `sleep`s need a shell.
const WIFI_RECONNECT_SEQUENCE: &str = "gpioset $(gpiofind WIFI-RESET)=1; sleep 1; \
     gpioset $(gpiofind WIFI-RESET)=0; sleep 1; wifi down; sleep 1; wifi up";

/// Whether WiFi reconfiguration is supported on this platform. It only works
/// where the setup AP runs over the mac80211 radio (BMC100, BFM100). The BMM
/// boards drive their ESP32 AP through a separate firmware path the overlay
/// does not implement, so the reconfigure/reconnect buttons are hidden there.
fn wifi_reconfig_supported(product: Product) -> bool {
    matches!(product, Product::Bmc100 | Product::Bfm100)
}

/// Spawn the detached WiFi reconnect sequence. The sequence sleeps ~3s and is
/// never waited on synchronously; the handle is reaped so it does not zombie.
fn spawn_wifi_reconnect() -> Option<Child> {
    match Command::new("/bin/sh")
        .arg("-c")
        .arg(WIFI_RECONNECT_SEQUENCE)
        .spawn()
    {
        Ok(child) => Some(child),
        Err(err) => {
            tracing::error!("failed to spawn WiFi reconnect sequence: {err}");
            None
        }
    }
}

/// Device hostname from procfs, trimmed of its trailing newline. `None` when
/// the file is unreadable or empty.
fn read_hostname() -> Option<String> {
    let raw = std::fs::read_to_string(HOSTNAME_PATH).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// WiFi signal level (dBm) of the first wireless interface in
/// `/proc/net/wireless`. The "level" column may carry a trailing dot.
fn read_wifi_signal_dbm() -> Option<i32> {
    let content = std::fs::read_to_string("/proc/net/wireless").ok()?;
    for line in content.lines().skip(2) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 4 {
            let level = cols[3].trim_end_matches('.');
            if let Ok(value) = level.parse::<f64>() {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "signal level in dBm is a small integer"
                )]
                return Some(value as i32);
            }
        }
    }
    None
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

/// Product selector for deterministic storybook tray views.
#[doc(hidden)]
pub use bmc_platform::Product as SettingsTrayProduct;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsTrayView {
    pub shape: DisplayShape,
    pub width: u32,
    pub height: u32,
    pub brightness: u8,
    pub hostname: Option<String>,
    pub ip: Option<String>,
    pub wifi_signal: Option<i32>,
    pub ssid: Option<String>,
    pub setup_ssid: Option<String>,
    pub wifi_buttons: bool,
    pub wifi_label: &'static str,
}

impl SettingsTrayView {
    /// Build a deterministic storybook-facing view shell for a hardware product.
    #[doc(hidden)]
    #[must_use]
    pub fn for_product(product: SettingsTrayProduct) -> Self {
        let profile = HardwareProfile::for_product(product);
        Self {
            shape: profile.display.shape,
            width: profile.display.logical_width,
            height: profile.display.logical_height,
            brightness: 50,
            hostname: None,
            ip: None,
            wifi_signal: None,
            ssid: None,
            setup_ssid: None,
            wifi_buttons: wifi_reconfig_supported(product),
            wifi_label: ButtonState::default().label(),
        }
    }
}

#[expect(missing_debug_implementations, reason = "TreeUi is not Debug")]
pub struct SettingsTrayRenderState {
    tree: TreeUi,
    icons: Option<WifiIcons>,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SettingsTrayRenderOutput {
    pub brightness_drag: Option<u8>,
    pub brightness_release: Option<u8>,
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
    hostname: Option<String>,
    ip: Option<String>,
    wifi_signal: Option<i32>,
    ssid: Option<String>,
    /// WiFi setup-AP SSID from `on_wifi_ap` (`None` = not in setup mode).
    setup_ssid: Option<String>,

    button: ButtonState,
    reconnect: ReconnectState,
    reconnect_child: Option<Child>,

    render_state: SettingsTrayRenderState,

    touch_track: Option<TouchTrack>,
    last_interaction: Instant,
    last_network_refresh: Instant,
    last_brightness_sent: Instant,
    /// Set on finger-up; consumed by the release flush, which sends the final
    /// brightness value once past the throttle.
    slider_released: bool,
    /// Set by the adapter render when `brightness_drag` is returned (the slider
    /// moved during this touch sequence); cleared on every finger-down so a
    /// fresh sequence starts unlocked.
    slider_dragged: bool,

    dismissing: bool,
    /// Set on any content change; drives the Task-9 panel cache.
    content_dirty: bool,
    /// Pure reveal/dismiss slide phase; the host reads its offset to blit the
    /// cached panel without re-laying-out the tree.
    slide: Slide,

    pending_requests: Vec<SettingsRequest>,
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
            hostname,
            ip: None,
            wifi_signal: None,
            ssid: None,
            setup_ssid: None,
            button: ButtonState::default(),
            reconnect: ReconnectState::default(),
            reconnect_child: None,
            render_state: SettingsTrayRenderState::new(now),
            touch_track: None,
            last_interaction: now,
            last_network_refresh: now,
            last_brightness_sent: now,
            slider_released: false,
            slider_dragged: false,
            dismissing: false,
            content_dirty: true,
            slide: Slide::default(),
            pending_requests: Vec::new(),
        }
    }

    #[must_use]
    fn view(&self) -> SettingsTrayView {
        let mut view = SettingsTrayView::for_product(self.product);
        view.shape = self.shape;
        view.width = self.width;
        view.height = self.height;
        view.brightness = self.brightness;
        view.hostname.clone_from(&self.hostname);
        view.ip.clone_from(&self.ip);
        view.wifi_signal = self.wifi_signal;
        view.ssid.clone_from(&self.ssid);
        view.setup_ssid.clone_from(&self.setup_ssid);
        view.wifi_buttons = wifi_reconfig_supported(self.product);
        view.wifi_label = self.button.label();
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
    /// Re-read IP / WiFi signal / SSID at most every `NETWORK_REFRESH`. Sets
    /// `content_dirty` when the IP, SSID, or signal icon band changes. dBm
    /// jitter inside one band does not trigger repaints, keeping hidden cache
    /// refreshes rare.
    fn refresh_network_if_due(&mut self, now: Instant) {
        if now.duration_since(self.last_network_refresh) < NETWORK_REFRESH {
            return;
        }
        self.last_network_refresh = now;
        let ip = primary_ipv4().as_ref().map(Ipv4Addr::to_string);
        let signal = read_wifi_signal_dbm();
        let ssid = bmc_system_overlay::configured_station_ssid();
        let band_changed = ui::signal_band(signal) != ui::signal_band(self.wifi_signal);
        if ip != self.ip || band_changed || ssid != self.ssid {
            self.ip = ip;
            self.ssid = ssid;
            self.content_dirty = true;
        }
        self.wifi_signal = signal;
    }

    /// Reap a finished reconnect child so the `sh` process does not zombie.
    fn reap_reconnect_child(&mut self) {
        if let Some(child) = self.reconnect_child.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) | Err(_) => self.reconnect_child = None,
                Ok(None) => {}
            }
        }
    }

    /// Advance both hold FSMs from the tree's press state and apply their side
    /// effects: queue a reconfigure request, spawn the reconnect sequence.
    fn advance_buttons(&mut self, now: Instant) {
        let reconfig_pressed = self.render_state.tree.is_pressed(ui::WIFI_RECONFIG_KEY);
        let prev = self.button;
        if self.button.tick(reconfig_pressed, now) == FsmAction::SendReconfigure {
            self.pending_requests.push(SettingsRequest::ReconfigureWifi);
        }
        if self.button != prev {
            self.content_dirty = true;
        }

        self.reap_reconnect_child();
        let reconnect_pressed = self.render_state.tree.is_pressed(ui::WIFI_RECONNECT_KEY);
        let prev_reconnect = self.reconnect;
        if self.reconnect.tick(reconnect_pressed, now) == ReconnectAction::Spawn {
            if let Some(mut prev) = self.reconnect_child.take() {
                // Drop never waits; reap the previous shell before replacing it.
                let _ = prev.kill();
                let _ = prev.wait();
            }
            self.reconnect_child = spawn_wifi_reconnect();
        }
        if self.reconnect != prev_reconnect {
            self.content_dirty = true;
        }
    }

    /// Flush the final brightness when the finger lifts after a drag that ran at
    /// least one render frame, so `self.brightness` already tracks the drag. The
    /// throttle can otherwise drop the last sub-`BRIGHTNESS_SEND_INTERVAL` of
    /// movement; this guarantees the value the finger lifted at is delivered. A
    /// fully-coalesced down-move-up never sets `slider_dragged`, so its release
    /// is delivered from the click position in `render` instead.
    fn flush_brightness_on_release(&mut self, now: Instant) {
        if !self.slider_released {
            return;
        }
        self.slider_released = false;
        self.pending_requests
            .push(SettingsRequest::SetBrightness(self.brightness));
        self.last_brightness_sent = now;
    }

    /// Whether a hold FSM is mid-animation or a hold button is pressed this
    /// frame, so the loop should fast-poll to keep the hold accruing.
    fn animating(&self) -> bool {
        self.button.is_animating()
            || self.reconnect.is_animating()
            || self.render_state.tree.is_pressed(ui::WIFI_RECONFIG_KEY)
            || self.render_state.tree.is_pressed(ui::WIFI_RECONNECT_KEY)
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
        // reconnect cooldown); when this reset changes content, repaint instead
        // of blitting the stale cache through the reveal ramp.
        if self.button != ButtonState::default() || self.reconnect != ReconnectState::default() {
            self.content_dirty = true;
        }
        self.button = ButtonState::default();
        self.reconnect = ReconnectState::default();
        self.touch_track = None;
        self.dismissing = false;
        self.last_interaction = now;
        self.slide.start_reveal();
    }

    fn on_brightness(&mut self, value: u8) {
        if value != self.brightness {
            self.brightness = value;
            self.content_dirty = true;
        }
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
        self.render_state.tree.push_touch(event);
        self.last_interaction = Instant::now();
        // Force a render so the interaction state processes the queued event and
        // runs its hit-test; without a paint frame the slider/buttons never see
        // the touch (the dismiss path below works off raw deltas, not hit-tests).
        self.content_dirty = true;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "surface-local logical coordinates fit f32 comfortably"
        )]
        match event {
            TouchEvent::Down { x, y, .. } => {
                self.slider_dragged = false;
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
            TouchEvent::Up { .. } => {
                self.slider_released = self.slider_dragged;
                if let Some(track) = self.touch_track.take()
                    && dismiss::classify(track.start, track.latest)
                {
                    self.dismissing = true;
                }
            }
            TouchEvent::Cancel => self.touch_track = None,
        }
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        self.refresh_network_if_due(now);
        self.flush_brightness_on_release(now);
        let was_dirty = self.content_dirty;
        self.advance_buttons(now);

        if now.duration_since(self.last_interaction) >= INACTIVITY_TIMEOUT {
            self.dismissing = true;
        }
        // Begin the slide-out the first tick a dismiss is decided; report
        // not-visible only once it has fully slid off so the framework keeps the
        // surface mapped for the duration of the animation.
        if self.dismissing && !self.slide.is_dismissing() {
            self.slide.start_dismiss();
        }
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
        let view = self.view();
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
        let view = self.view();
        let output = render_settings_tray(renderer, size, &mut self.render_state, &view, now);

        if let Some(b) = output.brightness_drag {
            self.slider_dragged = true;
            if b != self.brightness {
                self.brightness = b;
                self.content_dirty = true;
            }
            if now.duration_since(self.last_brightness_sent) >= BRIGHTNESS_SEND_INTERVAL {
                self.pending_requests
                    .push(SettingsRequest::SetBrightness(b));
                self.last_brightness_sent = now;
            }
        }
        // A fully-coalesced down-move-up (or a tap) sets no `slider_dragged`, so
        // `flush_brightness_on_release` has no drag value to send. The release
        // click still carries the finger-up position — deliver that final
        // brightness here. A multi-pass drag's release is left to the flush.
        if let Some(b) = output.brightness_release
            && !self.slider_dragged
        {
            if b != self.brightness {
                self.brightness = b;
                self.content_dirty = true;
            }
            self.pending_requests
                .push(SettingsRequest::SetBrightness(b));
            self.last_brightness_sent = now;
        }
    }

    fn drain_settings_requests(&mut self) -> Vec<SettingsRequest> {
        std::mem::take(&mut self.pending_requests)
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

pub fn render_settings_tray(
    renderer: &mut dyn Renderer,
    size: (u32, u32),
    state: &mut SettingsTrayRenderState,
    view: &SettingsTrayView,
    now: Instant,
) -> SettingsTrayRenderOutput {
    let icons = *state
        .icons
        .get_or_insert_with(|| icons::register_wifi_icons(renderer));

    let delta_ms =
        u32::try_from(now.duration_since(state.last_render).as_millis()).unwrap_or(u32::MAX);
    state.last_render = now;

    let wifi_view = if let Some(ssid) = view.setup_ssid.as_deref() {
        WifiView::Setup { ap_ssid: ssid }
    } else {
        WifiView::Idle {
            label: view.wifi_label,
        }
    };
    let node = ui::build_tree(
        view.brightness,
        view.hostname.as_deref(),
        view.ip.as_deref(),
        view.wifi_signal,
        view.ssid.as_deref(),
        icons,
        Panel {
            shape: view.shape,
            width: view.width,
            height: view.height,
            wifi_buttons: view.wifi_buttons,
        },
        wifi_view,
    );

    let result = match state.tree.render(&node, size, delta_ms, renderer) {
        Ok(result) => result,
        Err(err) => {
            tracing::error!("settings-tray tree render failed: {err}");
            return SettingsTrayRenderOutput::default();
        }
    };

    let brightness_drag = result.drags.get(ui::BRIGHTNESS_SLIDER_KEY).map(|hit| {
        let frac = (hit.x / hit.width).clamp(0.0, 1.0);
        dismiss::brightness_from_fraction(frac)
    });
    // The release click carries the finger-up position even when the whole
    // down-move-up is coalesced into one frame (no `drags` entry by then).
    let brightness_release = result.clicks.get(ui::BRIGHTNESS_SLIDER_KEY).map(|hit| {
        let frac = (hit.x / hit.width).clamp(0.0, 1.0);
        dismiss::brightness_from_fraction(frac)
    });

    SettingsTrayRenderOutput {
        brightness_drag,
        brightness_release,
    }
}

#[cfg(test)]
mod view_tests {
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

        let view = overlay.view();

        assert_eq!(view.brightness, 70);
        assert_eq!(view.hostname.as_deref(), Some("braiins-deck"));
        assert_eq!(view.ip.as_deref(), Some("192.168.1.42"));
        assert_eq!(view.wifi_signal, Some(-52));
        assert_eq!(view.ssid.as_deref(), Some("Braiins-WiFi"));
        assert_eq!(view.setup_ssid.as_deref(), Some("Deck setup"));
        assert!(view.wifi_buttons);
        assert_eq!(view.wifi_label, ButtonState::Active.label());
    }

    #[test]
    fn empty_setup_ap_ssid_does_not_enter_setup_view() {
        let now = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, now);
        overlay.on_wifi_ap(Some(""));
        assert_eq!(
            overlay.view().setup_ssid,
            None,
            "an empty AP SSID means setup inactive — the setup view must not show"
        );
    }

    #[test]
    fn view_for_product_exposes_bmm101_dimensions_for_stories() {
        let view = SettingsTrayView::for_product(SettingsTrayProduct::Bmm101);

        assert_eq!(view.width, 480);
        assert_eq!(view.height, 320);
        assert!(!view.wifi_buttons);
    }
}

#[cfg(test)]
mod wake_tests {
    use super::*;
    use std::time::{Duration, Instant};

    // A finger-down on a hold-to-confirm button only becomes `is_pressed` during
    // the next `render`, so the hold FSM advances a frame later. The tick that
    // schedules the wake must fast-poll right after a touch-down; otherwise the
    // just-rendered press state isn't re-examined until the 2 s network refresh
    // and the hold timer and its progress stall.
    #[test]
    fn finger_down_schedules_a_fast_wake() {
        let t0 = Instant::now();
        let mut overlay = SettingsTrayOverlay::new_for_product(Product::Bmc100, None, t0);
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
        // dismiss must not force a paint; a dirty one must (touch dismisses).
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
        let h = overlay.view().height as f32;
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
