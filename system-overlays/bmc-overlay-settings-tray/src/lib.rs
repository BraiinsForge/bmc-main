// Copyright (C) 2026  Braiins Systems s.r.o.

//! Swipe-from-top quick-settings overlay: a brightness slider plus WiFi station
//! info and hold-to-confirm reconfigure/reconnect buttons. Ported from the
//! BDK-343 `settings-stub` widget to a native `bmc-render` `TreeNode` overlay.
//!
//! The surface is fullscreen with a full input region so a tap below the panel
//! is delivered here (tap-outside dismiss) rather than falling through, and so
//! the tray blocks scene swipes behind it while up. The panel itself is drawn
//! only in the top `panel_height` band; the rest is transparent.

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

/// Which way the panel is sliding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlidePhase {
    Revealing,
    Dismissing,
}

/// Pure eased vertical slide for the panel band: the panel translates from
/// off-screen (`-height`) to settled (`0`) on reveal and back on dismiss. The
/// offset is computed from `now`; nothing here touches the GPU, so the timing is
/// unit-tested in isolation from the blit.
#[derive(Debug, Clone, Copy, Default)]
struct Slide {
    phase: Option<(SlidePhase, Instant)>,
}

impl Slide {
    fn start_reveal(&mut self, now: Instant) {
        self.phase = Some((SlidePhase::Revealing, now));
    }

    fn start_dismiss(&mut self, now: Instant) {
        self.phase = Some((SlidePhase::Dismissing, now));
    }

    /// Whether a dismiss ramp has been started (regardless of completion).
    fn is_dismissing(&self) -> bool {
        matches!(self.phase, Some((SlidePhase::Dismissing, _)))
    }

    /// The blit-only decision the host obeys: blit the cached panel at the
    /// current offset only while a slide is running *and* the content has not
    /// changed this frame; otherwise (`None`) the host full-paints. Keeping it a
    /// method on `Slide` lets the invariant be unit-tested without a
    /// platform-detected `SettingsTrayOverlay`.
    fn cached_blit_offset(&self, now: Instant, content_dirty: bool, height: f32) -> Option<f32> {
        (self.animating(now) && !content_dirty).then(|| self.offset(now, height))
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
            Some((SlidePhase::Revealing, start)) => -height * (1.0 - Self::progress(start, now)),
            Some((SlidePhase::Dismissing, start)) => -height * Self::progress(start, now),
            None => 0.0,
        }
    }

    /// Whether a ramp is still in progress at `now`.
    fn animating(&self, now: Instant) -> bool {
        match self.phase {
            Some((_, start)) => Self::progress(start, now) < 1.0,
            None => false,
        }
    }

    /// Whether a dismiss ramp has fully completed at `now`.
    fn dismiss_done(&self, now: Instant) -> bool {
        match self.phase {
            Some((SlidePhase::Dismissing, start)) => Self::progress(start, now) >= 1.0,
            Some((SlidePhase::Revealing, _)) | None => false,
        }
    }
}

#[expect(missing_debug_implementations, reason = "TreeUi is not Debug")]
pub struct SettingsTrayOverlay {
    product: Product,
    shape: DisplayShape,
    width: u32,
    height: u32,
    /// Height (px) of the drawn panel band; the rest of the surface is
    /// transparent.
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

    tree: TreeUi,
    icons: Option<WifiIcons>,

    touch_track: Option<TouchTrack>,
    last_interaction: Instant,
    last_network_refresh: Instant,
    last_render: Instant,
    last_brightness_sent: Instant,
    /// Set on finger-up; consumed by the release flush, which sends the final
    /// brightness value once past the throttle.
    slider_released: bool,

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
        // The tray covers the full display, so its size is the panel
        // resolution. Resolve it from the platform profile; detection failure
        // is fatal because rendering at the wrong size is worse than not at all.
        let product = BmcInfo::load()
            .map(|info| info.bmc_platform.product())
            .expect("BUG: platform detection must succeed for the settings tray");
        let profile = HardwareProfile::for_product(product);
        let width = profile.display.logical_width;
        let height = profile.display.logical_height;
        let shape = profile.display.shape;
        let panel_height = panel_height_for(shape, height);

        let now = Instant::now();
        Self {
            product,
            shape,
            width,
            height,
            panel_height,
            brightness: 50,
            hostname: read_hostname(),
            ip: None,
            wifi_signal: None,
            ssid: None,
            setup_ssid: None,
            button: ButtonState::default(),
            reconnect: ReconnectState::default(),
            reconnect_child: None,
            tree: TreeUi::new(),
            icons: None,
            touch_track: None,
            last_interaction: now,
            last_network_refresh: now,
            last_render: now,
            last_brightness_sent: now,
            slider_released: false,
            dismissing: false,
            content_dirty: true,
            slide: Slide::default(),
            pending_requests: Vec::new(),
        }
    }
}

/// Panel band height: a round display draws the whole face; a rectangular one
/// draws the full height too (the original `Panel` is always display-sized).
fn panel_height_for(_shape: DisplayShape, height: u32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "display height is well within f32 mantissa precision"
    )]
    let h = height as f32;
    h
}

impl SettingsTrayOverlay {
    /// Re-read IP / WiFi signal / SSID at most every `NETWORK_REFRESH`. Sets
    /// `content_dirty` when anything changed.
    fn refresh_network_if_due(&mut self, now: Instant) {
        if now.duration_since(self.last_network_refresh) < NETWORK_REFRESH {
            return;
        }
        self.last_network_refresh = now;
        let ip = primary_ipv4().as_ref().map(Ipv4Addr::to_string);
        let signal = read_wifi_signal_dbm();
        let ssid = bmc_system_overlay::configured_station_ssid();
        if ip != self.ip || signal != self.wifi_signal || ssid != self.ssid {
            self.ip = ip;
            self.wifi_signal = signal;
            self.ssid = ssid;
            self.content_dirty = true;
        }
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
        let reconfig_pressed = self.tree.is_pressed(ui::WIFI_RECONFIG_KEY);
        let prev = self.button;
        if self.button.tick(reconfig_pressed, now) == FsmAction::SendReconfigure {
            self.pending_requests.push(SettingsRequest::ReconfigureWifi);
        }
        if self.button != prev {
            self.content_dirty = true;
        }

        self.reap_reconnect_child();
        let reconnect_pressed = self.tree.is_pressed(ui::WIFI_RECONNECT_KEY);
        let prev_reconnect = self.reconnect;
        if self.reconnect.tick(reconnect_pressed, now) == ReconnectAction::Spawn {
            self.reconnect_child = spawn_wifi_reconnect();
        }
        if self.reconnect != prev_reconnect {
            self.content_dirty = true;
        }
    }

    /// Send the final brightness once when the finger lifts. This lives outside
    /// `render`'s `drags` block because `TreeUi::render` clears the touched key
    /// in `begin_frame()`, so a coalesced down-move-up has no `drags` entry on
    /// the release frame — the in-`drags` throttled send would otherwise drop
    /// the value the finger lifted at. Always sends once (clearing the latch),
    /// so the optimistic `self.brightness` is guaranteed delivered.
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
            || self.tree.is_pressed(ui::WIFI_RECONFIG_KEY)
            || self.tree.is_pressed(ui::WIFI_RECONNECT_KEY)
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
        self.button = ButtonState::default();
        self.reconnect = ReconnectState::default();
        self.touch_track = None;
        self.dismissing = false;
        self.last_interaction = now;
        self.content_dirty = true;
        self.slide.start_reveal(now);
    }

    fn on_brightness(&mut self, value: u8) {
        if value != self.brightness {
            self.brightness = value;
            self.content_dirty = true;
        }
    }

    fn on_wifi_ap(&mut self, ssid: Option<&str>) {
        self.setup_ssid = ssid.map(str::to_owned);
        self.button.on_wifi_ap(ssid.is_some());
        self.content_dirty = true;
    }

    fn on_touch(&mut self, event: TouchEvent) {
        self.tree.push_touch(event);
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
                self.slider_released = true;
                if let Some(track) = self.touch_track.take()
                    && dismiss::classify(track.start, track.latest, self.panel_height)
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
            self.slide.start_dismiss(now);
        }

        let sliding = self.slide.animating(now);
        let visible = !(self.dismissing && self.slide.dismiss_done(now));
        let animating = self.animating() || sliding;
        let wants_render = visible && (was_dirty || self.content_dirty || animating);
        let next_wake = if !visible {
            None
        } else if animating {
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
        // Compile + upload the Wi-Fi SVG icons now, at host startup, so the
        // first reveal does not pay the SVG decode/upload cost mid-swipe.
        self.icons
            .get_or_insert_with(|| icons::register_wifi_icons(renderer));
    }

    fn render(&mut self, renderer: &mut dyn Renderer, size: (u32, u32)) {
        let icons = *self
            .icons
            .get_or_insert_with(|| icons::register_wifi_icons(renderer));

        let now = Instant::now();
        let delta_ms =
            u32::try_from(now.duration_since(self.last_render).as_millis()).unwrap_or(u32::MAX);
        self.last_render = now;

        let wifi_view = if let Some(ssid) = self.setup_ssid.as_deref() {
            WifiView::Setup { ap_ssid: ssid }
        } else {
            WifiView::Idle {
                label: self.button.label(),
            }
        };
        // Drive the displayed slider from the (possibly sub-floor) brightness so
        // a night value clamps to 0 rather than underflowing.
        let display_brightness = dismiss::brightness_from_fraction(
            dismiss::brightness_display_fraction(self.brightness),
        );
        let node = ui::build_tree(
            display_brightness,
            self.hostname.as_deref(),
            self.ip.as_deref(),
            self.wifi_signal,
            self.ssid.as_deref(),
            icons,
            Panel {
                shape: self.shape,
                width: self.width,
                height: self.height,
                wifi_buttons: wifi_reconfig_supported(self.product),
            },
            wifi_view,
        );

        let result = match self.tree.render(&node, size, delta_ms, renderer) {
            Ok(result) => result,
            Err(err) => {
                tracing::error!("settings-tray tree render failed: {err}");
                return;
            }
        };

        if let Some(hit) = result.drags.get(ui::BRIGHTNESS_SLIDER_KEY) {
            let frac = (hit.x / hit.width).clamp(0.0, 1.0);
            let b = dismiss::brightness_from_fraction(frac);
            if b != self.brightness {
                // Optimistic local update: the slider follows the finger without
                // waiting for the compositor round-trip; the later `brightness`
                // event reconciles.
                self.brightness = b;
                self.content_dirty = true;
            }
            if now.duration_since(self.last_brightness_sent) >= BRIGHTNESS_SEND_INTERVAL {
                self.pending_requests
                    .push(SettingsRequest::SetBrightness(b));
                self.last_brightness_sent = now;
            }
        }
    }

    fn drain_settings_requests(&mut self) -> Vec<SettingsRequest> {
        std::mem::take(&mut self.pending_requests)
    }

    fn wants_cached_blit(&self, now: Instant) -> Option<f32> {
        self.slide
            .cached_blit_offset(now, self.content_dirty, self.panel_height)
    }

    fn take_content_dirty(&mut self) -> bool {
        std::mem::take(&mut self.content_dirty)
    }
}

#[cfg(test)]
mod slide_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn reveal_eases_from_offscreen_to_zero() {
        let t0 = Instant::now();
        let mut s = Slide::default();
        s.start_reveal(t0);
        assert!(s.offset(t0, 200.0) <= -199.0); // starts offscreen (height 200)
        assert!(s.offset(t0 + Duration::from_millis(200), 200.0).abs() < 1e-3);
        assert!(!s.animating(t0 + Duration::from_millis(200)));
    }

    #[test]
    fn dismiss_eases_back_offscreen_then_reports_done() {
        let t0 = Instant::now();
        let mut s = Slide::default();
        s.start_reveal(t0);
        s.start_dismiss(t0 + Duration::from_millis(200));
        assert!(s.offset(t0 + Duration::from_millis(200), 200.0).abs() < 1e-3);
        assert!(!s.dismiss_done(t0 + Duration::from_millis(200)));
        assert!(s.offset(t0 + Duration::from_millis(400), 200.0) <= -199.0);
        assert!(s.dismiss_done(t0 + Duration::from_millis(400)));
    }

    #[test]
    fn dirty_frame_full_paints_then_clean_frame_blits_cache() {
        let t0 = Instant::now();
        let mut s = Slide::default();
        s.start_reveal(t0);
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
}
