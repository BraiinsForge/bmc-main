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

//! Fullscreen device-info overlay: the first-boot setup flow, WiFi
//! reconfiguration, and operational-boot connect info.
//!
//! bmc owns the lifecycle and drives this overlay over `deck_device_info_v1`
//! (`device_state`, `setup_progress`, `access_point`); the displayed address
//! comes from the connectivity prober's station IP. Every screen-hold timer
//! lives here; bmc emits transitions the moment they happen.

mod icons;
mod ui;

pub use ui::{DeviceInfoRenderState, DeviceInfoView, render_device_info};

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use bmc_render::renderer::Renderer;
use bmc_system_overlay::{
    AccessPoint, DeviceState, Layer, LayerConfig, SetupStep, SnapshotVersion, SystemOverlay,
    TickOutcome, TouchEvent, UpgradeKind, UpgradeSnapshot, UpgradeState, VersionedSnapshot,
};

/// Generic screen hold (legacy `SCREEN_DURATION`): connected, completed,
/// setup-error, and post-upgrade success screens.
const HOLD: Duration = Duration::from_secs(5);
/// How long the operational connect-info stays up before auto-dismiss.
const SUCCESS_VISIBLE_FOR: Duration = Duration::from_secs(10);
/// How long the operational failure screen stays up before auto-dismiss.
const FAILURE_VISIBLE_FOR: Duration = Duration::from_secs(5);
/// How long an operational boot waits for an IP before showing failure.
const WAIT_FOR_IP: Duration = Duration::from_secs(20);
/// Reconfiguration AP screen auto-hide; the AP stays up (legacy
/// `WIFI_RECONFIG_TIMEOUT`). A later setup event revives the flow.
const RECONFIG_SCREEN_TIMEOUT: Duration = Duration::from_mins(8);
/// Snapshot re-read (wake) cadence while a screen depends on prober state.
const POLL: Duration = Duration::from_secs(1);

/// Injected connectivity source so the state machine is unit-testable.
trait Env {
    /// Latest snapshot and its version when the content changed since `seen`
    /// (`None` = nothing seen yet); `None` otherwise.
    fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot>;
}

struct OsEnv;
impl Env for OsEnv {
    fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
        bmc_system_overlay::snapshot_if_changed(seen)
    }
}

/// Which flow the device lifecycle selects. Mirrors `DeviceState`, plus
/// `Unknown` for before the first `device_state` event (nothing is shown
/// until then).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Unknown,
    FactoryDefault,
    WifiReconfiguration,
    /// Configured but setup unfinished: bmc joins WiFi on its own.
    SetupPending,
    Operational,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    /// Lifecycle unknown yet — stay unmapped rather than guess a flow.
    Hidden,
    SetupStart {
        since: Instant,
    },
    SetupConnecting,
    /// `to_scenes` distinguishes the reconfiguration success (straight back to scenes)
    /// from first-boot success (on to the setup connect-info).
    SetupConnected {
        since: Instant,
        to_scenes: bool,
    },
    /// Setup connect-info. Carries the address rather than reading the prober
    /// live, so losing it cannot drop the screen back to connect progress —
    /// nothing would ever move it off there again.
    SetupConnectInfo {
        ip: Option<Ipv4Addr>,
    },
    SetupCompleted {
        since: Instant,
    },
    SetupError {
        since: Instant,
    },
    /// Sticky setup failure. `restarting` says whether bmc resolves it
    /// by restarting the device, which is all the screen does differently.
    /// Both hold until something outside the overlay moves the device on.
    SetupFatal {
        restarting: bool,
    },
    OpConnecting {
        since: Instant,
    },
    /// Post-firmware-upgrade success, the operational flow's opening screen.
    OpUpgraded {
        since: Instant,
    },
    OpSuccess {
        since: Instant,
        ip: Ipv4Addr,
    },
    OpFailed {
        since: Instant,
    },
    /// Handed off to scenes (unmapped). Setup events revive the flow.
    Done,
}

impl Screen {
    fn in_setup_flow(self) -> bool {
        matches!(
            self,
            Screen::SetupStart { .. }
                | Screen::SetupConnecting
                | Screen::SetupConnected { .. }
                | Screen::SetupConnectInfo { .. }
                | Screen::SetupCompleted { .. }
                | Screen::SetupError { .. }
                | Screen::SetupFatal { .. }
        )
    }

    fn visible(self) -> bool {
        !matches!(self, Screen::Hidden | Screen::Done)
    }
}

/// Advance the screen's own timers for one tick. Pure; returns the next
/// screen and whether it changed.
fn step(screen: Screen, mode: Mode, now: Instant, station_ip: Option<Ipv4Addr>) -> (Screen, bool) {
    let next = match screen {
        Screen::SetupStart { since } => {
            // Only the reconfiguration entry times out;
            // a first boot has no scenes worth returning to.
            if mode == Mode::WifiReconfiguration
                && now.duration_since(since) >= RECONFIG_SCREEN_TIMEOUT
            {
                Screen::Done
            } else {
                screen
            }
        }
        Screen::SetupConnecting => {
            // Only a SetupPending boot self-advances on the address: in AP
            // mode the join outcome arrives as an explicit setup event.
            if mode == Mode::SetupPending && station_ip.is_some() {
                Screen::SetupConnectInfo { ip: station_ip }
            } else {
                screen
            }
        }
        Screen::SetupConnected { since, to_scenes } => {
            if now.duration_since(since) >= HOLD {
                if to_scenes {
                    Screen::Done
                } else {
                    Screen::SetupConnectInfo { ip: station_ip }
                }
            } else {
                screen
            }
        }
        Screen::SetupCompleted { since } => {
            if now.duration_since(since) >= HOLD {
                Screen::Done
            } else {
                screen
            }
        }
        Screen::SetupError { since } => {
            if now.duration_since(since) >= HOLD {
                Screen::SetupStart { since: now }
            } else {
                screen
            }
        }
        Screen::OpConnecting { since } => {
            if let Some(ip) = station_ip {
                Screen::OpSuccess { since: now, ip }
            } else if now.duration_since(since) >= WAIT_FOR_IP {
                Screen::OpFailed { since: now }
            } else {
                screen
            }
        }
        Screen::OpUpgraded { since } => {
            if now.duration_since(since) >= HOLD {
                Screen::OpConnecting { since: now }
            } else {
                screen
            }
        }
        Screen::OpSuccess { since, ip: shown } => {
            if now.duration_since(since) >= SUCCESS_VISIBLE_FOR {
                Screen::Done
            } else {
                // Track an address change, but keep the last-known IP
                // through a transient DHCP loss so the screen does not flicker.
                // A short acquire-then-lose can therefore show a stale IP
                // for up to SUCCESS_VISIBLE_FOR; accepted.
                let ip = station_ip.unwrap_or(shown);
                Screen::OpSuccess { since, ip }
            }
        }
        Screen::OpFailed { since } => {
            if now.duration_since(since) >= FAILURE_VISIBLE_FOR {
                Screen::Done
            } else {
                screen
            }
        }
        Screen::SetupConnectInfo { ip: shown } => Screen::SetupConnectInfo {
            ip: station_ip.or(shown),
        },
        Screen::Hidden | Screen::SetupFatal { .. } | Screen::Done => screen,
    };
    let changed = next != screen;
    (next, changed)
}

/// The operational flow's opening screen for a boot that follows an upgrade.
fn operational_entry(post_upgrade: Option<UpgradeKind>, now: Instant) -> Screen {
    match post_upgrade {
        Some(UpgradeKind::Firmware) => Screen::OpUpgraded { since: now },
        // A package activation only restarted the compositor — the network
        // never dropped, so a connection screen would be stale noise.
        Some(UpgradeKind::Packages) => Screen::Done,
        Some(_) | None => Screen::OpConnecting { since: now },
    }
}

enum NextWake {
    At(Instant),
    Poll,
}

/// The earliest instant `step` could produce a different screen,
/// `None` when only external events can move it.
fn next_deadline(screen: Screen, mode: Mode) -> Option<NextWake> {
    match screen {
        Screen::SetupStart { since } => (mode == Mode::WifiReconfiguration)
            .then_some(NextWake::At(since + RECONFIG_SCREEN_TIMEOUT)),
        Screen::SetupConnecting => (mode == Mode::SetupPending).then_some(NextWake::Poll),
        Screen::SetupConnected { since, .. }
        | Screen::SetupCompleted { since }
        | Screen::SetupError { since }
        | Screen::OpUpgraded { since } => Some(NextWake::At(since + HOLD)),
        // The shown address may still change (late DHCP), so keep polling.
        Screen::SetupConnectInfo { .. }
        | Screen::OpConnecting { .. }
        | Screen::OpSuccess { .. } => Some(NextWake::Poll),
        Screen::OpFailed { since } => Some(NextWake::At(since + FAILURE_VISIBLE_FOR)),
        Screen::Hidden | Screen::SetupFatal { .. } | Screen::Done => None,
    }
}

pub struct DeviceInfoOverlay {
    screen: Screen,
    mode: Mode,
    ap: Option<AccessPoint>,
    /// Target SSID from the `connecting_to_wifi` event;
    /// preferred over the prober's saved-network SSID while set.
    target_ssid: Option<String>,
    station_ip: Option<Ipv4Addr>,
    station_ssid: Option<String>,
    /// Which upgrade this startup follows, from a terminal success snapshot;
    /// `None` for an ordinary boot.
    post_upgrade: Option<UpgradeKind>,
    snapshot_version: Option<SnapshotVersion>,
    /// Latched "content changed" from events between ticks.
    dirty: bool,
    render_state: DeviceInfoRenderState,
    env: Box<dyn Env>,
}

impl std::fmt::Debug for DeviceInfoOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceInfoOverlay")
            .field("screen", &self.screen)
            .field("mode", &self.mode)
            .field("ap", &self.ap)
            .field("station_ip", &self.station_ip)
            .field("post_upgrade", &self.post_upgrade)
            .finish_non_exhaustive()
    }
}

impl Default for DeviceInfoOverlay {
    fn default() -> Self {
        Self {
            screen: Screen::Hidden,
            mode: Mode::Unknown,
            ap: None,
            target_ssid: None,
            station_ip: None,
            station_ssid: None,
            post_upgrade: None,
            snapshot_version: None,
            dirty: false,
            render_state: DeviceInfoRenderState::new(Instant::now()),
            env: Box::new(OsEnv),
        }
    }
}

impl DeviceInfoOverlay {
    /// The network the setup flow is joining: the target named by the event
    /// where there is one, else the saved station network, which is what
    /// a SetupPending boot has, since it never sees a `connecting_to_wifi`.
    ///
    /// Setup screens only. The operational screens describe the network
    /// the device is configured for, so they read the prober directly
    /// rather than inherit a join target that has outlived its flow.
    fn setup_ssid(&self) -> Option<String> {
        self.target_ssid
            .clone()
            .or_else(|| self.station_ssid.clone())
    }

    #[must_use]
    fn view(&self) -> DeviceInfoView {
        match self.screen {
            Screen::Hidden | Screen::Done => DeviceInfoView::Done,
            Screen::SetupStart { .. } => DeviceInfoView::SetupStart {
                ap: self.ap.clone(),
            },
            Screen::SetupConnecting => DeviceInfoView::SetupConnecting {
                ssid: self.setup_ssid(),
            },
            Screen::SetupConnected { .. } => DeviceInfoView::SetupConnected {
                ssid: self.setup_ssid(),
            },
            Screen::SetupConnectInfo { ip } => DeviceInfoView::SetupConnectInfo {
                ip,
                ssid: self.setup_ssid(),
            },
            Screen::SetupCompleted { .. } => DeviceInfoView::SetupCompleted,
            Screen::SetupError { .. } => DeviceInfoView::SetupError,
            Screen::SetupFatal { restarting } => DeviceInfoView::SetupFatal { restarting },
            Screen::OpUpgraded { .. } => DeviceInfoView::UpgradeSuccess,
            Screen::OpConnecting { .. } => DeviceInfoView::Connecting {
                ssid: self.station_ssid.clone(),
            },
            Screen::OpSuccess { ip, .. } => DeviceInfoView::Success { ip },
            Screen::OpFailed { .. } => DeviceInfoView::Failed {
                ssid: self.station_ssid.clone(),
            },
        }
    }

    /// Fold a changed snapshot into the displayed address/SSID; returns
    /// whether either changed.
    fn refresh_from_snapshot(&mut self) -> bool {
        let Some(VersionedSnapshot { version, snapshot }) =
            self.env.snapshot_if_changed(self.snapshot_version)
        else {
            return false;
        };
        self.snapshot_version = Some(version);
        let changed =
            self.station_ip != snapshot.station_ipv4 || self.station_ssid != snapshot.station_ssid;
        self.station_ip = snapshot.station_ipv4;
        self.station_ssid = snapshot.station_ssid;
        changed
    }
}

impl SystemOverlay for DeviceInfoOverlay {
    fn layer_config(&self) -> LayerConfig {
        // Bottom, not the fullscreen default of Top: the device-info screens
        // must sit below a firing alarm (Top), the upgrade splash (Top),
        // and the settings tray (Overlay), while still occluding the scene.
        //
        // Nothing here pauses while covered. An alarm above this layer
        // can consume a whole boot connect-info window, since the holds
        // keep running unseen. Deliberate: there is no occlusion signal
        // to act on, and one brief alarm does not justify a suspend path.
        LayerConfig {
            layer: Layer::Bottom,
            ..LayerConfig::fullscreen("bmc-overlay-device-info")
        }
    }

    fn uses_device_info(&self) -> bool {
        true
    }

    fn prewarm(&mut self, renderer: &mut dyn Renderer) {
        let _ = self.render_state.ensure_icons(renderer);
    }

    fn on_device_state(&mut self, state: DeviceState, boot_flow_delivered: bool) {
        self.mode = match state {
            DeviceState::FactoryDefault => Mode::FactoryDefault,
            DeviceState::WifiReconfiguration => Mode::WifiReconfiguration,
            DeviceState::SetupPending => Mode::SetupPending,
            DeviceState::Operational => Mode::Operational,
        };
        self.dirty = true;
        match self.mode {
            Mode::FactoryDefault | Mode::WifiReconfiguration => {
                if !self.screen.in_setup_flow() {
                    self.screen = Screen::SetupStart {
                        since: Instant::now(),
                    };
                }
            }
            Mode::SetupPending => {
                if !self.screen.in_setup_flow() {
                    self.screen = Screen::SetupConnecting;
                }
            }
            // Reconfiguration exits AP mode first, so mid-setup the lifecycle
            // reaches Operational before the final setup event arrives.
            // Only a cold start is therefore still `Hidden` here.
            // The session flag covers a restarted overlay, `Hidden` again,
            // which must not replay a boot sequence the user already dismissed.
            Mode::Operational => {
                if self.screen == Screen::Hidden && !boot_flow_delivered {
                    self.screen = operational_entry(self.post_upgrade, Instant::now());
                }
            }
            Mode::Unknown => {}
        }
    }

    fn on_setup_progress(&mut self, step: SetupStep, wifi_ssid: &str) {
        let now = Instant::now();
        // Any real step (re)enters the setup flow,
        // including from a dismissed reconfiguration screen (`Done`).
        // Mirrors the legacy listener, which set the screen unconditionally.
        self.screen = match step {
            SetupStep::Idle => return,
            SetupStep::ConnectingToWifi => {
                self.target_ssid = Some(wifi_ssid.to_owned());
                Screen::SetupConnecting
            }
            SetupStep::WifiConnectionSuccess => Screen::SetupConnected {
                since: now,
                to_scenes: false,
            },
            SetupStep::WifiReconfigSuccess => Screen::SetupConnected {
                since: now,
                to_scenes: true,
            },
            SetupStep::WifiConnectionFailed => Screen::SetupError { since: now },
            SetupStep::DeviceSetupSuccess => Screen::SetupCompleted { since: now },
            SetupStep::UnexpectedError { restarting } => Screen::SetupFatal { restarting },
        };
        self.dirty = true;
    }

    fn on_access_point(&mut self, ap: Option<&AccessPoint>) {
        self.ap = ap.cloned();
        self.dirty = true;
    }

    fn uses_upgrade(&self) -> bool {
        true
    }

    /// A terminal *success* snapshot marks this startup as post-upgrade.
    ///
    /// The runner drains device-info events before applying the snapshot,
    /// whatever order the wire replayed them in, so a post-upgrade boot
    /// already sits on `OpConnecting` and the swap below raises the screen.
    /// The latch is for the other order, where no connect screen exists yet;
    /// `operational_entry` consumes it.
    ///
    /// `remaining` is ignored: this overlay times the screen itself.
    fn on_upgrade_state(&mut self, snapshot: UpgradeSnapshot) {
        if !matches!(snapshot.state, UpgradeState::Succeeded { .. }) {
            return;
        }
        self.post_upgrade = Some(snapshot.kind);
        if matches!(self.screen, Screen::OpConnecting { .. }) {
            self.screen = operational_entry(self.post_upgrade, Instant::now());
            self.dirty = true;
        }
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        let probe_changed = self.refresh_from_snapshot();
        let (next, screen_changed) = step(self.screen, self.mode, now, self.station_ip);
        self.screen = next;
        let visible = self.screen.visible();
        let next_wake = match next_deadline(self.screen, self.mode) {
            Some(NextWake::At(deadline)) => Some(deadline),
            Some(NextWake::Poll) => Some(now + POLL),
            None => None,
        };
        let dirty = std::mem::take(&mut self.dirty);
        TickOutcome {
            visible,
            wants_render: visible && (screen_changed || probe_changed || dirty),
            next_wake,
        }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        let view = self.view();
        render_device_info(r, size, &mut self.render_state, &view);
    }

    fn on_touch(&mut self, event: TouchEvent) {
        if !matches!(event, TouchEvent::Down { .. }) {
            return;
        }
        // Touch acts on the operational flow only. The setup screens stay:
        // dismissing SetupStart would leave a blank screen
        // with the AP up and the user mid-wizard.
        if matches!(self.screen, Screen::OpUpgraded { .. }) {
            // An interstitial rather than the end of the flow,
            // so skipping it goes on to connect instead of back to the scenes.
            self.screen = Screen::OpConnecting {
                since: Instant::now(),
            };
            self.dirty = true;
        } else if matches!(
            self.screen,
            Screen::OpConnecting { .. } | Screen::OpSuccess { .. } | Screen::OpFailed { .. }
        ) {
            self.screen = Screen::Done;
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_system_overlay::Snapshot;

    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    fn setup_ap() -> AccessPoint {
        AccessPoint {
            ssid: "Deck setup".to_owned(),
            setup_url: "http://10.0.0.21/".to_owned(),
        }
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

    fn overlay_with_ip(ip: Option<Ipv4Addr>) -> DeviceInfoOverlay {
        DeviceInfoOverlay {
            env: Box::new(StaticEnv {
                snapshot: Some(Snapshot {
                    ipv4: ip,
                    station_ipv4: ip,
                    station_ssid: None,
                    wifi_signal_dbm: None,
                }),
            }),
            ..DeviceInfoOverlay::default()
        }
    }

    fn succeeded(kind: UpgradeKind, remaining: Duration) -> UpgradeSnapshot {
        UpgradeSnapshot {
            kind,
            state: UpgradeState::Succeeded { remaining },
        }
    }

    #[test]
    fn hidden_until_the_first_device_state() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        let tick = overlay.tick(t0());
        assert!(!tick.visible);
        assert_eq!(tick.next_wake, None);
    }

    #[test]
    fn operational_runs_the_connect_flow() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::Operational, false);
        let start = t0();
        let tick = overlay.tick(start);
        assert!(tick.visible);
        assert!(matches!(overlay.screen, Screen::OpSuccess { .. }));

        let tick2 = overlay.tick(start + SUCCESS_VISIBLE_FOR + POLL);
        assert!(!tick2.visible);
        assert_eq!(overlay.screen, Screen::Done);
    }

    #[test]
    fn a_resumed_bind_does_not_replay_the_boot_flow() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::Operational, true);
        assert_eq!(overlay.screen, Screen::Hidden);
        let tick = overlay.tick(t0());
        assert!(!tick.visible);
        assert_eq!(tick.next_wake, None);
    }

    #[test]
    fn a_resumed_bind_does_not_replay_the_upgrade_screen() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_upgrade_state(succeeded(UpgradeKind::Firmware, Duration::from_secs(3)));
        overlay.on_device_state(DeviceState::Operational, true);
        assert_eq!(overlay.screen, Screen::Hidden);
    }

    #[test]
    fn a_resumed_bind_still_enters_the_setup_flow() {
        // Unlike the boot sequence, these reflect a standing condition:
        // the device really is waiting in setup right now.
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::FactoryDefault, true);
        assert!(matches!(overlay.screen, Screen::SetupStart { .. }));

        let mut pending = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        pending.on_device_state(DeviceState::SetupPending, true);
        let _ = pending.tick(t0());
        assert_eq!(
            pending.screen,
            Screen::SetupConnectInfo {
                ip: Some(Ipv4Addr::new(10, 0, 0, 5))
            },
            "the setup connect-info must come back for a restarted overlay"
        );
    }

    #[test]
    fn operational_without_ip_fails_after_deadline() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::Operational, false);
        let start = t0();
        let _ = overlay.tick(start);
        let _ = overlay.tick(start + WAIT_FOR_IP);
        assert!(matches!(overlay.screen, Screen::OpFailed { .. }));
    }

    #[test]
    fn factory_default_shows_setup_start_and_ignores_touch() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_access_point(Some(&setup_ap()));
        let tick = overlay.tick(t0());
        assert!(tick.visible);
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupStart {
                ap: Some(setup_ap())
            }
        );

        overlay.on_touch(TouchEvent::Down {
            id: 0,
            x: 0.0,
            y: 0.0,
        });
        assert!(overlay.tick(t0()).visible, "setup screens ignore touch");
    }

    #[test]
    fn first_boot_success_walks_to_connect_info() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        assert_eq!(overlay.screen, Screen::SetupConnecting);

        overlay.on_setup_progress(SetupStep::WifiConnectionSuccess, "");
        let start = t0();
        let _ = overlay.tick(start + HOLD);
        assert_eq!(
            overlay.screen,
            Screen::SetupConnectInfo {
                ip: Some(Ipv4Addr::new(10, 0, 0, 5))
            }
        );
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupConnectInfo {
                ip: Some(Ipv4Addr::new(10, 0, 0, 5)),
                ssid: Some("HomeNet".to_owned()),
            }
        );

        overlay.on_setup_progress(SetupStep::DeviceSetupSuccess, "");
        let _ = overlay.tick(start + HOLD + HOLD);
        assert_eq!(overlay.screen, Screen::Done);
    }

    #[test]
    fn a_join_target_never_reaches_the_operational_screens() {
        // The target belongs to the setup flow. The operational screens name
        // the network the device is configured for, which is the prober's.
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupConnecting {
                ssid: Some("HomeNet".to_owned())
            }
        );

        overlay.screen = Screen::OpConnecting { since: t0() };
        assert_eq!(
            overlay.view(),
            DeviceInfoView::Connecting { ssid: None },
            "the stale target must not survive into the connect screen"
        );
    }

    #[test]
    fn ap_mode_connecting_does_not_self_advance_on_ip() {
        // In AP mode the join outcome must come from bmc's setup event;
        // a station address appearing early must not skip ahead.
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        let _ = overlay.tick(t0());
        assert_eq!(overlay.screen, Screen::SetupConnecting);
    }

    #[test]
    fn setup_pending_advances_to_connect_info_on_ip() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::SetupPending, false);
        let _ = overlay.tick(t0());
        assert_eq!(
            overlay.screen,
            Screen::SetupConnectInfo {
                ip: Some(Ipv4Addr::new(10, 0, 0, 5))
            }
        );
    }

    #[test]
    fn connection_failure_returns_to_setup_start() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_setup_progress(SetupStep::WifiConnectionFailed, "");
        let start = t0();
        let _ = overlay.tick(start + HOLD);
        assert!(matches!(overlay.screen, Screen::SetupStart { .. }));
    }

    #[test]
    fn reconfig_success_returns_to_scenes_without_connect_info() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        // Reconfiguration exits AP mode before the success event arrives.
        overlay.on_device_state(DeviceState::Operational, false);
        assert_eq!(
            overlay.screen,
            Screen::SetupConnecting,
            "flow survives the lifecycle flip"
        );

        overlay.on_setup_progress(SetupStep::WifiReconfigSuccess, "");
        let tick = overlay.tick(t0() + HOLD);
        assert_eq!(overlay.screen, Screen::Done);
        assert!(!tick.visible);
    }

    #[test]
    fn reconfig_setup_start_times_out_but_events_revive_it() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        let _ = overlay.tick(t0() + RECONFIG_SCREEN_TIMEOUT);
        assert_eq!(overlay.screen, Screen::Done, "AP screen auto-hides");

        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        assert_eq!(overlay.screen, Screen::SetupConnecting, "late join revives");
    }

    #[test]
    fn first_boot_setup_start_never_times_out() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        let _ = overlay.tick(t0() + RECONFIG_SCREEN_TIMEOUT + HOLD);
        assert!(matches!(overlay.screen, Screen::SetupStart { .. }));
    }

    #[test]
    fn unexpected_error_is_sticky() {
        // Both variants wait for something outside the overlay: the device
        // restarting, or the user restarting it.
        for restarting in [true, false] {
            let mut overlay = overlay_with_ip(None);
            overlay.on_device_state(DeviceState::SetupPending, false);
            overlay.on_setup_progress(SetupStep::UnexpectedError { restarting }, "");
            let tick = overlay.tick(t0() + HOLD + HOLD);
            assert_eq!(overlay.screen, Screen::SetupFatal { restarting });
            assert!(tick.visible, "restarting={restarting}");
            assert_eq!(tick.next_wake, None, "restarting={restarting}");

            overlay.on_touch(TouchEvent::Down {
                id: 0,
                x: 0.0,
                y: 0.0,
            });
            assert!(
                overlay.tick(t0()).visible,
                "a touch must not dismiss an unresolved failure (restarting={restarting})"
            );
        }
    }

    #[test]
    fn the_fatal_screen_says_whether_a_restart_is_coming() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: false }, "");
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupFatal { restarting: false }
        );

        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: true }, "");
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupFatal { restarting: true }
        );
    }

    #[test]
    fn package_upgrade_success_skips_the_connect_screen() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::Operational, false);
        overlay.on_upgrade_state(succeeded(UpgradeKind::Packages, Duration::from_secs(3)));
        assert_eq!(overlay.screen, Screen::Done);
    }

    #[test]
    fn firmware_upgrade_success_opens_the_flow_and_hands_over_to_connecting() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::Operational, false);
        overlay.on_upgrade_state(succeeded(UpgradeKind::Firmware, Duration::from_secs(3)));
        assert!(matches!(overlay.screen, Screen::OpUpgraded { .. }));
        let start = t0();
        let tick = overlay.tick(start);
        assert!(tick.visible);
        assert_eq!(overlay.view(), DeviceInfoView::UpgradeSuccess);

        let _ = overlay.tick(start + HOLD);
        assert!(matches!(overlay.screen, Screen::OpConnecting { .. }));
    }

    #[test]
    fn a_success_snapshot_before_the_lifecycle_still_opens_the_flow() {
        // The compositor replays `deck_upgrade_v1` before `deck_device_info_v1`,
        // so this is the ordering a post-upgrade boot actually sees.
        let mut overlay = overlay_with_ip(None);
        overlay.on_upgrade_state(succeeded(UpgradeKind::Firmware, Duration::from_secs(3)));
        assert_eq!(overlay.screen, Screen::Hidden, "no flow to show it in yet");

        overlay.on_device_state(DeviceState::Operational, false);
        assert!(matches!(overlay.screen, Screen::OpUpgraded { .. }));
    }

    #[test]
    fn a_package_success_before_the_lifecycle_still_skips_the_flow() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_upgrade_state(succeeded(UpgradeKind::Packages, Duration::from_secs(3)));
        overlay.on_device_state(DeviceState::Operational, false);
        assert_eq!(overlay.screen, Screen::Done);
    }

    #[test]
    fn touch_skips_the_success_screen_into_the_connect_flow() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::Operational, false);
        overlay.on_upgrade_state(succeeded(UpgradeKind::Firmware, Duration::from_secs(3)));
        let _ = overlay.tick(t0());

        overlay.on_touch(TouchEvent::Down {
            id: 0,
            x: 0.0,
            y: 0.0,
        });
        assert!(matches!(overlay.screen, Screen::OpConnecting { .. }));
        let tick = overlay.tick(t0());
        assert!(tick.visible, "the flow continues rather than handing off");
        assert!(
            tick.wants_render,
            "the connect screen must replace the success screen"
        );
    }

    #[test]
    fn upgrade_snapshots_leave_the_setup_flow_alone() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_upgrade_state(succeeded(UpgradeKind::Packages, Duration::from_secs(3)));
        assert!(
            matches!(overlay.screen, Screen::SetupStart { .. }),
            "a package restart must not skip the setup screens"
        );
    }

    #[test]
    fn a_late_upgrade_cannot_resurrect_a_dismissed_flow() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::Operational, false);
        let start = t0();
        let _ = overlay.tick(start);
        let _ = overlay.tick(start + SUCCESS_VISIBLE_FOR);
        assert_eq!(overlay.screen, Screen::Done);

        overlay.on_upgrade_state(succeeded(UpgradeKind::Firmware, Duration::from_secs(3)));
        assert_eq!(overlay.screen, Screen::Done);
    }

    #[test]
    fn setup_connect_info_keeps_its_address_through_a_probe_loss() {
        let shown = Ipv4Addr::new(10, 0, 0, 5);
        let (next, changed) = step(
            Screen::SetupConnectInfo { ip: Some(shown) },
            Mode::SetupPending,
            t0(),
            None,
        );
        assert_eq!(
            next,
            Screen::SetupConnectInfo { ip: Some(shown) },
            "the QR must stay up: the user needs it to finish the wizard"
        );
        assert!(!changed);
    }

    #[test]
    fn setup_connect_info_picks_up_a_late_address() {
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        let (next, changed) = step(
            Screen::SetupConnectInfo { ip: None },
            Mode::SetupPending,
            t0(),
            Some(ip),
        );
        assert_eq!(next, Screen::SetupConnectInfo { ip: Some(ip) });
        assert!(changed);
    }

    #[test]
    fn op_success_keeps_last_ip_through_transient_probe_loss() {
        let start = t0();
        let shown = Ipv4Addr::new(10, 0, 0, 5);
        let (next, changed) = step(
            Screen::OpSuccess {
                since: start,
                ip: shown,
            },
            Mode::Operational,
            start + POLL,
            None,
        );
        assert_eq!(
            next,
            Screen::OpSuccess {
                since: start,
                ip: shown
            }
        );
        assert!(!changed);
    }

    #[test]
    fn op_touch_dismisses_immediately() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::Operational, false);
        let _ = overlay.tick(t0());
        overlay.on_touch(TouchEvent::Down {
            id: 0,
            x: 0.0,
            y: 0.0,
        });
        let tick = overlay.tick(t0());
        assert!(!tick.visible);
        assert_eq!(tick.next_wake, None);
    }
}
