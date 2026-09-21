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
//! What the board is and what it can connect with come over `deck_platform_v1`,
//! so the overlay never reads the hardware profile itself.

mod icons;
mod ui;

pub use ui::{DeviceInfoRenderState, DeviceInfoView, Link, Uplinks, render_device_info};

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use bmc_render::renderer::Renderer;
use bmc_system_overlay::{
    AccessPoint, DeviceState, Layer, LayerConfig, PlatformCaps, SetupStep, SnapshotVersion,
    SystemOverlay, TickOutcome, TouchEvent, UpgradeKind, UpgradeSnapshot, UpgradeState,
    VersionedSnapshot,
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
/// How long an unresolved setup failure holds a device whose setup is done.
/// Long enough to read and act on, and no longer:
/// the tray also shows a setup AP that is still up,
/// so this screen is not the only record of the failure.
const FATAL_SCREEN_TIMEOUT: Duration = Duration::from_mins(1);
/// Snapshot re-read (wake) cadence while a screen depends on prober state.
const POLL: Duration = Duration::from_secs(1);
/// How long a board on its cable keeps a connect-info address the prober no longer sees:
/// long enough to ride out a lease renew,
/// short enough that a pulled cable does not leave a dead wizard URL on the screen.
/// Longer than bmc's grace on the setup-AP screen (`SETUP_URL_CLEAR_AFTER`
/// refreshes, in `bmc/src/startup.rs`), which covers the same pulled cable one
/// screen earlier: that one has an AP coming up behind it, this one only the
/// next lease.
const ADDRESS_LOSS_GRACE: Duration = Duration::from_secs(10);

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

impl From<DeviceState> for Mode {
    fn from(state: DeviceState) -> Self {
        match state {
            DeviceState::FactoryDefault => Mode::FactoryDefault,
            DeviceState::WifiReconfiguration => Mode::WifiReconfiguration,
            DeviceState::SetupPending => Mode::SetupPending,
            DeviceState::Operational => Mode::Operational,
        }
    }
}

impl Mode {
    /// Whether the wizard has been finished on this device,
    /// so a screen may step aside to the scenes. Mid-setup it holds instead:
    /// the scenes are there, but stepping aside would hide the wizard.
    fn setup_done(self) -> bool {
        match self {
            Mode::WifiReconfiguration | Mode::Operational => true,
            Mode::FactoryDefault | Mode::SetupPending | Mode::Unknown => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    /// Lifecycle unknown yet — stay unmapped rather than guess a flow.
    Hidden,
    SetupStart,
    /// The setup AP is being torn down and the device continues over its wired uplink.
    SetupSwitching,
    SetupConnecting,
    SetupConnected {
        since: Instant,
    },
    /// Setup connect-info. Carries the address rather than reading the prober
    /// live, so a momentary loss cannot drop the screen back to connect progress.
    /// A wired SetupPending boot does drop it after `ADDRESS_LOSS_GRACE`
    /// (see `drop_lost_address`).
    SetupConnectInfo {
        ip: Option<Ipv4Addr>,
    },
    SetupCompleted {
        since: Instant,
    },
    SetupError,
    /// Setup failure the overlay cannot resolve.
    /// `restarting` says whether bmc resolves it by restarting the device.
    /// A restart is worth waiting out, so that variant holds.
    /// The other steps aside once the user has had time to read it,
    /// but only once the setup is done (`Mode::setup_done`).
    SetupFatal {
        since: Instant,
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
    /// Whether a setup flow is live on screen. A restarting fatal counts:
    /// bmc is rebooting the device, and this is the last thing the user
    /// sees before it does. A dismissible fatal does not: that flow died there.
    fn setup_in_progress(self) -> bool {
        matches!(
            self,
            Screen::SetupStart
                | Screen::SetupSwitching
                | Screen::SetupConnecting
                | Screen::SetupConnected { .. }
                | Screen::SetupConnectInfo { .. }
                | Screen::SetupCompleted { .. }
                | Screen::SetupError
                | Screen::SetupFatal {
                    restarting: true,
                    ..
                }
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
        Screen::SetupSwitching => {
            // Left by the lifecycle, not a timer: the switchover is done once
            // the device has advanced past the setup AP.
            if mode == Mode::SetupPending {
                Screen::SetupConnecting
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
        Screen::SetupConnected { since } => {
            if now.duration_since(since) >= HOLD {
                // An operational device goes back to its scenes. A first boot
                // still has the wizard to finish, and so has a reconfiguration
                // begun mid-setup: its join leaves the lifecycle on SetupPending,
                // so both go on to the connect-info.
                if mode.setup_done() {
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
        Screen::SetupFatal {
            since,
            restarting: false,
        } if mode.setup_done() && now.duration_since(since) >= FATAL_SCREEN_TIMEOUT => Screen::Done,
        Screen::Hidden
        | Screen::SetupStart
        | Screen::SetupError
        | Screen::SetupFatal { .. }
        | Screen::Done => screen,
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
        Screen::SetupConnecting | Screen::SetupSwitching => {
            (mode == Mode::SetupPending).then_some(NextWake::Poll)
        }
        Screen::SetupConnected { since }
        | Screen::SetupCompleted { since }
        | Screen::OpUpgraded { since } => Some(NextWake::At(since + HOLD)),
        // The shown address may still change (late DHCP), so keep polling.
        Screen::SetupConnectInfo { .. }
        | Screen::OpConnecting { .. }
        | Screen::OpSuccess { .. } => Some(NextWake::Poll),
        Screen::OpFailed { since } => Some(NextWake::At(since + FAILURE_VISIBLE_FOR)),
        Screen::SetupFatal {
            since,
            restarting: false,
        } => mode
            .setup_done()
            .then_some(NextWake::At(since + FATAL_SCREEN_TIMEOUT)),
        Screen::Hidden
        | Screen::SetupStart
        | Screen::SetupError
        | Screen::SetupFatal { .. }
        | Screen::Done => None,
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
    /// When the prober stopped seeing the address a connect-info screen shows;
    /// see [`Self::drop_lost_address`].
    address_lost_since: Option<Instant>,
    /// Which upgrade this startup follows, from a terminal success snapshot;
    /// `None` for an ordinary boot.
    post_upgrade: Option<UpgradeKind>,
    snapshot_version: Option<SnapshotVersion>,
    /// Latched "content changed" from events between ticks.
    dirty: bool,
    render_state: DeviceInfoRenderState,
    /// Product display name the screens address the user with
    /// ("Braiins Deck", "Braiins Mini Miner", ...).
    device_name: String,
    /// Whether this is a mining product; picks the device artwork.
    miner: bool,
    /// Which uplinks the board has,
    /// so a screen waiting on a connection can tell a Wi-Fi join from a cable,
    /// and a board without Wi-Fi is never asked to join one.
    uplinks: Uplinks,
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
        // Until the compositor says otherwise the screens read as the Deck's,
        // which they were written for. A v1 compositor never sends the name.
        Self {
            screen: Screen::Hidden,
            mode: Mode::Unknown,
            ap: None,
            target_ssid: None,
            station_ip: None,
            station_ssid: None,
            address_lost_since: None,
            post_upgrade: None,
            snapshot_version: None,
            dirty: false,
            render_state: DeviceInfoRenderState::new(Instant::now()),
            device_name: "Braiins Deck".to_owned(),
            miner: false,
            uplinks: Uplinks::WIFI_ONLY,
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

    /// What a screen waiting on a connection is waiting for.
    /// A Wi-Fi join is under way when the board has Wi-Fi and a network to name;
    /// otherwise a board with an ethernet port waits on its cable.
    /// A board without one has nothing else to wait on, so it always reads as Wi-Fi.
    fn link_for(&self, ssid: Option<String>) -> Link {
        if self.uplinks.ethernet && !(self.uplinks.wifi && ssid.is_some()) {
            Link::Cable
        } else {
            Link::Wifi { ssid }
        }
    }

    /// Whether the current fatal screen can be sent away,
    /// by touch or by its own timeout. Both paths ask this one question,
    /// so the close glyph never advertises a dismissal the touch handler refuses.
    fn fatal_dismissible(&self) -> bool {
        matches!(
            self.screen,
            Screen::SetupFatal {
                restarting: false,
                ..
            }
        ) && self.mode.setup_done()
    }

    #[must_use]
    fn view(&self) -> DeviceInfoView {
        match self.screen {
            Screen::Hidden | Screen::Done => DeviceInfoView::Done,
            Screen::SetupStart => DeviceInfoView::SetupStart {
                ap: self.ap.clone(),
                uplinks: self.uplinks,
            },
            Screen::SetupSwitching => DeviceInfoView::TurningApOff,
            Screen::SetupConnecting => DeviceInfoView::SetupConnecting {
                link: self.link_for(self.setup_ssid()),
            },
            Screen::SetupConnected { .. } => DeviceInfoView::SetupConnected {
                ssid: self.setup_ssid(),
            },
            Screen::SetupConnectInfo { ip } => DeviceInfoView::SetupConnectInfo {
                ip,
                link: self.link_for(self.setup_ssid()),
            },
            Screen::SetupCompleted { .. } => DeviceInfoView::SetupCompleted,
            Screen::SetupError => DeviceInfoView::SetupError,
            Screen::SetupFatal { restarting, .. } => DeviceInfoView::SetupFatal {
                restarting,
                dismissible: self.fatal_dismissible(),
            },
            Screen::OpUpgraded { .. } => DeviceInfoView::UpgradeSuccess,
            Screen::OpConnecting { .. } => DeviceInfoView::Connecting {
                link: self.link_for(self.station_ssid.clone()),
            },
            Screen::OpSuccess { ip, .. } => DeviceInfoView::Success { ip },
            Screen::OpFailed { .. } => DeviceInfoView::Failed {
                link: self.link_for(self.station_ssid.clone()),
            },
        }
    }

    /// Take a setup connect-info address off the screen
    /// once a board running on its cable has gone [`ADDRESS_LOSS_GRACE`]
    /// without it: that is a pulled cable, and the URL it advertised is dead.
    /// Only a SetupPending boot drops it: that is the one flow whose connect
    /// progress reacquires the address on its own, so anywhere else the screen
    /// would stay on "Connecting" for good.
    /// An address a Wi-Fi join produced is kept through a loss instead,
    /// since a station that drops out comes back with the same one.
    /// Which of the two it is, the overlay asks the way the screens do:
    /// whatever the connect progress would say it is waiting on
    /// is what the address on display came over.
    /// Returns whether the screen changed.
    fn drop_lost_address(&mut self, now: Instant) -> bool {
        let shown = matches!(self.screen, Screen::SetupConnectInfo { ip: Some(_) });
        let on_a_cable = matches!(self.link_for(self.setup_ssid()), Link::Cable);
        let pending = self.mode == Mode::SetupPending;
        if !(shown && pending && on_a_cable && self.station_ip.is_none()) {
            self.address_lost_since = None;
            return false;
        }
        let since = *self.address_lost_since.get_or_insert(now);
        if now.duration_since(since) < ADDRESS_LOSS_GRACE {
            return false;
        }
        self.address_lost_since = None;
        self.screen = Screen::SetupConnecting;
        true
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

    fn uses_platform(&self) -> bool {
        true
    }

    fn on_platform_capabilities(&mut self, caps: PlatformCaps) {
        self.miner = caps.mining;
        self.uplinks = Uplinks {
            wifi: caps.wifi,
            ethernet: caps.ethernet,
        };
        self.dirty = true;
    }

    fn on_platform_product_name(&mut self, name: &str) {
        name.clone_into(&mut self.device_name);
        self.dirty = true;
    }

    fn prewarm(&mut self, renderer: &mut dyn Renderer) {
        let _ = self.render_state.ensure_icons(renderer);
    }

    fn on_device_state(&mut self, state: DeviceState, boot_flow_delivered: bool) {
        let mode = Mode::from(state);
        let previous = std::mem::replace(&mut self.mode, mode);
        self.dirty = true;
        match mode {
            // A different state is a different flow and replaces the screen,
            // except a restart bmc has already announced.
            Mode::FactoryDefault | Mode::WifiReconfiguration => {
                let same_flow = previous == mode && self.screen.setup_in_progress();
                let awaiting_restart = matches!(
                    self.screen,
                    Screen::SetupFatal {
                        restarting: true,
                        ..
                    }
                );
                if !same_flow && !awaiting_restart {
                    self.screen = Screen::SetupStart;
                }
            }
            // A wizard round lands here after its join, whatever mode it ran in,
            // so a flow past the AP continues. The AP and switchover screens are
            // stale once the lifecycle has advanced, so they move to the connect
            // flow, which fills in the uplink address; so does a cold entry.
            // Over a cable that address is already there, and `step` moves the
            // screen on to it before the first frame, so no connect progress
            // describing a join that never happens is ever drawn.
            Mode::SetupPending => {
                if matches!(self.screen, Screen::SetupStart | Screen::SetupSwitching)
                    || !self.screen.setup_in_progress()
                {
                    self.screen = Screen::SetupConnecting;
                }
            }
            // Reconfiguration exits AP mode first, so mid-flow the lifecycle
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
            // Both leave the join behind: a later screen that names the network
            // would otherwise name one the device gave up on.
            SetupStep::SwitchingUplink => {
                self.target_ssid = None;
                Screen::SetupSwitching
            }
            SetupStep::WifiConnectionSuccess | SetupStep::WifiReconfigSuccess => {
                Screen::SetupConnected { since: now }
            }
            SetupStep::WifiConnectionFailed => {
                self.target_ssid = None;
                Screen::SetupError
            }
            SetupStep::DeviceSetupSuccess => Screen::SetupCompleted { since: now },
            SetupStep::UnexpectedError { restarting } => Screen::SetupFatal {
                since: now,
                restarting,
            },
        };
        self.dirty = true;
    }

    fn on_access_point(&mut self, ap: Option<&AccessPoint>) {
        self.ap = ap.cloned();
        if self.ap.is_some() && self.screen == Screen::SetupError {
            self.screen = Screen::SetupStart;
        }
        self.dirty = true;
    }

    /// Put the operational connect-info screen back up, on its usual timer,
    /// or the failure screen when there is no address. When it declines,
    /// `docs/devel/system-overlays/overlays.md` ("IP-report button") has the reasons.
    fn on_report_ip(&mut self) {
        if self.mode != Mode::Operational
            || self.screen.setup_in_progress()
            || matches!(
                self.screen,
                Screen::OpConnecting { .. } | Screen::OpUpgraded { .. }
            )
        {
            tracing::debug!(mode = ?self.mode, screen = ?self.screen, "ignoring report_ip");
            return;
        }
        self.refresh_from_snapshot();
        let now = Instant::now();
        self.screen = match self.station_ip {
            Some(ip) => Screen::OpSuccess { since: now, ip },
            None => Screen::OpFailed { since: now },
        };
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
        let (next, stepped) = step(self.screen, self.mode, now, self.station_ip);
        self.screen = next;
        let screen_changed = stepped | self.drop_lost_address(now);
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
        render_device_info(
            r,
            size,
            &mut self.render_state,
            &view,
            &self.device_name,
            self.miner,
        );
    }

    fn on_touch(&mut self, event: TouchEvent) {
        if !matches!(event, TouchEvent::Down { .. }) {
            return;
        }
        // Touch acts on the operational flow,
        // and on a fatal screen the user can do nothing about.
        // The rest of the setup screens stay: dismissing SetupStart
        // would hide the wizard with the AP still up.
        if self.fatal_dismissible() {
            self.screen = Screen::Done;
        } else if matches!(self.screen, Screen::OpUpgraded { .. }) {
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
    use std::cell::RefCell;
    use std::rc::Rc;

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

    /// A prober the test publishes to, versioned like the real one:
    /// each publish is a new version, and a reader that has folded
    /// the current one in gets no re-read.
    #[derive(Clone, Default)]
    struct Prober(Rc<RefCell<Option<VersionedSnapshot>>>);

    impl Prober {
        fn publish(&self, ip: Option<Ipv4Addr>) {
            self.publish_snapshot(ip, None);
        }

        /// The same on a board with a station network saved, which the prober
        /// reports whether or not the address on `ip` came over it.
        fn publish_joined(&self, ip: Option<Ipv4Addr>, ssid: &str) {
            self.publish_snapshot(ip, Some(ssid.to_owned()));
        }

        fn publish_snapshot(&self, ip: Option<Ipv4Addr>, station_ssid: Option<String>) {
            let mut slot = self.0.borrow_mut();
            let version = slot
                .as_ref()
                .map_or(SnapshotVersion::FIRST, |latest| latest.version.next());
            *slot = Some(VersionedSnapshot {
                version,
                snapshot: Snapshot {
                    ipv4: ip,
                    station_ipv4: ip,
                    station_ssid,
                    wifi_signal_dbm: None,
                },
            });
        }
    }

    impl Env for Prober {
        fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
            let slot = self.0.borrow();
            let latest = slot.as_ref()?;
            (seen != Some(latest.version)).then(|| latest.clone())
        }
    }

    fn overlay_with_prober(ip: Option<Ipv4Addr>) -> (DeviceInfoOverlay, Prober) {
        let prober = Prober::default();
        prober.publish(ip);
        let overlay = DeviceInfoOverlay {
            env: Box::new(prober.clone()),
            ..DeviceInfoOverlay::default()
        };
        (overlay, prober)
    }

    fn overlay_with_ip(ip: Option<Ipv4Addr>) -> DeviceInfoOverlay {
        overlay_with_prober(ip).0
    }

    fn succeeded(kind: UpgradeKind, remaining: Duration) -> UpgradeSnapshot {
        UpgradeSnapshot {
            kind,
            state: UpgradeState::Succeeded { remaining },
        }
    }

    #[test]
    fn a_failed_join_leaves_no_ssid_behind() {
        let mut overlay = wired_board(BOTH, None);
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        overlay.on_setup_progress(SetupStep::WifiConnectionFailed, "");
        // The cable goes in: the wired setup screen, then the lifecycle advances.
        overlay.on_access_point(Some(&AccessPoint {
            ssid: String::new(),
            setup_url: "http://10.33.50.103/".to_owned(),
        }));
        overlay.on_device_state(DeviceState::SetupPending, false);
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupConnecting { link: Link::Cable },
            "the network the join gave up on must not be named again"
        );
    }

    const BOTH: Uplinks = Uplinks {
        wifi: true,
        ethernet: true,
    };
    const CABLE_ONLY: Uplinks = Uplinks {
        wifi: false,
        ethernet: true,
    };

    /// The compositor's platform event for a board with these uplinks.
    fn platform_with(uplinks: Uplinks) -> PlatformCaps {
        PlatformCaps {
            wifi: uplinks.wifi,
            ethernet: uplinks.ethernet,
            ..PlatformCaps::default()
        }
    }

    fn wired_board(uplinks: Uplinks, ip: Option<Ipv4Addr>) -> DeviceInfoOverlay {
        let mut overlay = overlay_with_ip(ip);
        overlay.on_platform_capabilities(platform_with(uplinks));
        overlay
    }

    #[test]
    fn a_board_reads_as_the_deck_until_the_compositor_says_otherwise() {
        let overlay = DeviceInfoOverlay::default();
        assert_eq!(overlay.device_name, "Braiins Deck");
        assert!(!overlay.miner);
        assert_eq!(overlay.uplinks, Uplinks::WIFI_ONLY);
    }

    #[test]
    fn the_platform_events_name_the_board_and_its_uplinks() {
        let mut overlay = DeviceInfoOverlay::default();
        overlay.on_platform_capabilities(PlatformCaps {
            wifi: false,
            ethernet: true,
            mining: true,
            boser_managed: true,
        });
        overlay.on_platform_product_name("Braiins Mini Miner");
        assert_eq!(overlay.device_name, "Braiins Mini Miner");
        assert!(overlay.miner);
        assert_eq!(overlay.uplinks, CABLE_ONLY);
        assert!(overlay.dirty, "a change of wording is a change of content");
    }

    #[test]
    fn an_ethernet_only_setup_pending_boot_waits_on_its_cable() {
        let mut overlay = wired_board(CABLE_ONLY, None);
        overlay.on_device_state(DeviceState::SetupPending, false);
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupConnecting { link: Link::Cable }
        );
    }

    #[test]
    fn a_wifi_less_board_asks_for_a_cable_while_the_ap_is_pending() {
        let mut overlay = wired_board(CABLE_ONLY, None);
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupStart {
                ap: None,
                uplinks: CABLE_ONLY,
            }
        );
    }

    #[test]
    fn a_join_in_flight_keeps_the_wifi_wording_on_a_wired_board() {
        let mut overlay = wired_board(BOTH, None);
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupConnecting {
                link: Link::Wifi {
                    ssid: Some("HomeNet".to_owned())
                }
            }
        );
    }

    #[test]
    fn a_wired_board_with_no_station_configured_waits_on_its_cable() {
        let mut overlay = wired_board(BOTH, None);
        overlay.on_device_state(DeviceState::SetupPending, false);
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupConnecting { link: Link::Cable }
        );
    }

    /// A wired board mid-setup, its connect-info address on screen.
    fn wired_connect_info(uplinks: Uplinks, ip: Ipv4Addr) -> (DeviceInfoOverlay, Prober) {
        let (mut overlay, prober) = overlay_with_prober(Some(ip));
        overlay.on_platform_capabilities(platform_with(uplinks));
        overlay.on_device_state(DeviceState::SetupPending, false);
        // The lifecycle only opens the connect progress; the tick that reads
        // the address is what moves the screen on to the connect info.
        let _ = overlay.tick(t0());
        assert_eq!(overlay.screen, Screen::SetupConnectInfo { ip: Some(ip) });
        (overlay, prober)
    }

    #[test]
    fn a_wired_board_drops_a_connect_info_address_the_cable_lost() {
        let ip = Ipv4Addr::new(10, 33, 50, 103);
        let (mut overlay, prober) = wired_connect_info(BOTH, ip);
        let start = t0();

        prober.publish(None);
        let _ = overlay.tick(start + POLL);
        assert_eq!(
            overlay.screen,
            Screen::SetupConnectInfo { ip: Some(ip) },
            "a blip keeps the address"
        );

        let _ = overlay.tick(start + POLL + ADDRESS_LOSS_GRACE);
        assert_eq!(overlay.screen, Screen::SetupConnecting);
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupConnecting { link: Link::Cable }
        );

        prober.publish(Some(ip));
        let _ = overlay.tick(start + POLL + ADDRESS_LOSS_GRACE + POLL);
        assert_eq!(
            overlay.screen,
            Screen::SetupConnectInfo { ip: Some(ip) },
            "the cable going back in brings the address back"
        );
    }

    #[test]
    fn a_loss_before_the_lifecycle_advances_waits_for_it() {
        let ip = Ipv4Addr::new(10, 33, 50, 103);
        let (mut overlay, prober) = overlay_with_prober(Some(ip));
        overlay.on_platform_capabilities(platform_with(BOTH));
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.screen = Screen::SetupConnectInfo { ip: Some(ip) };
        let start = t0();

        prober.publish(None);
        let _ = overlay.tick(start);
        let _ = overlay.tick(start + ADDRESS_LOSS_GRACE + POLL);
        assert_eq!(
            overlay.screen,
            Screen::SetupConnectInfo { ip: Some(ip) },
            "in AP mode nothing would move the screen off the connect progress"
        );

        overlay.on_device_state(DeviceState::SetupPending, false);
        let advanced = start + ADDRESS_LOSS_GRACE + 2 * POLL;
        let _ = overlay.tick(advanced);
        let _ = overlay.tick(advanced + ADDRESS_LOSS_GRACE);
        assert_eq!(
            overlay.screen,
            Screen::SetupConnecting,
            "the grace counts from the lifecycle advancing"
        );
    }

    #[test]
    fn a_loss_shorter_than_the_grace_starts_the_count_over() {
        let ip = Ipv4Addr::new(10, 33, 50, 103);
        let (mut overlay, prober) = wired_connect_info(BOTH, ip);
        let start = t0();
        let half = ADDRESS_LOSS_GRACE / 2;

        prober.publish(None);
        let _ = overlay.tick(start);
        prober.publish(Some(ip));
        let _ = overlay.tick(start + half);
        prober.publish(None);
        let _ = overlay.tick(start + half + POLL);
        let _ = overlay.tick(start + half + POLL + half);
        assert_eq!(
            overlay.screen,
            Screen::SetupConnectInfo { ip: Some(ip) },
            "two short losses do not add up to one long one"
        );
    }

    #[test]
    fn the_deck_keeps_its_connect_info_address_through_a_loss() {
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        let (mut overlay, prober) = wired_connect_info(Uplinks::WIFI_ONLY, ip);
        prober.publish(None);
        // The first tick past the loss only starts the grace; the second is
        // what would drop the address, so both are needed to prove it holds.
        let start = t0() + POLL;
        let _ = overlay.tick(start);
        let _ = overlay.tick(start + ADDRESS_LOSS_GRACE);
        assert_eq!(overlay.screen, Screen::SetupConnectInfo { ip: Some(ip) });
    }

    #[test]
    fn a_board_with_a_port_keeps_an_address_its_wifi_join_produced() {
        let ip = Ipv4Addr::new(10, 33, 50, 103);
        let (mut overlay, prober) = overlay_with_prober(None);
        overlay.on_platform_capabilities(platform_with(BOTH));
        prober.publish_joined(Some(ip), "HomeNet");
        overlay.on_device_state(DeviceState::SetupPending, false);
        let _ = overlay.tick(t0());
        assert_eq!(overlay.screen, Screen::SetupConnectInfo { ip: Some(ip) });

        prober.publish_joined(None, "HomeNet");
        let start = t0() + POLL;
        let _ = overlay.tick(start);
        let _ = overlay.tick(start + ADDRESS_LOSS_GRACE);
        assert_eq!(
            overlay.screen,
            Screen::SetupConnectInfo { ip: Some(ip) },
            "the port is not the cable: a station that drops out comes back with the same address"
        );
    }

    #[test]
    fn the_button_reports_no_network_on_an_unplugged_wired_board() {
        let mut overlay = dismissed_operational(None);
        overlay.on_platform_capabilities(platform_with(CABLE_ONLY));
        overlay.on_report_ip();
        let _ = overlay.tick(t0());
        assert_eq!(overlay.view(), DeviceInfoView::Failed { link: Link::Cable });
    }

    #[test]
    fn skipping_wifi_forgets_a_join_that_was_tried_first() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        overlay.on_setup_progress(SetupStep::SwitchingUplink, "");
        assert_eq!(overlay.target_ssid, None);
    }

    #[test]
    fn skipping_wifi_over_a_cable_lands_on_the_connect_info() {
        let ip = Ipv4Addr::new(10, 33, 50, 103);
        let mut overlay = overlay_with_ip(Some(ip));
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_setup_progress(SetupStep::SwitchingUplink, "");
        overlay.on_device_state(DeviceState::SetupPending, false);
        let _ = overlay.tick(t0());
        assert_eq!(
            overlay.screen,
            Screen::SetupConnectInfo { ip: Some(ip) },
            "the cable's address is already there, so the connect progress does not hold"
        );
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
        assert!(matches!(overlay.screen, Screen::SetupStart));

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
                ap: Some(setup_ap()),
                uplinks: Uplinks::WIFI_ONLY,
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
                link: Link::Wifi {
                    ssid: Some("HomeNet".to_owned())
                },
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
                link: Link::Wifi {
                    ssid: Some("HomeNet".to_owned())
                }
            }
        );

        overlay.screen = Screen::OpConnecting { since: t0() };
        assert_eq!(
            overlay.view(),
            DeviceInfoView::Connecting {
                link: Link::Wifi { ssid: None }
            },
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
    fn skipping_wifi_shows_the_switchover_until_the_lifecycle_advances() {
        // Ethernet skip: the switching event raises the switchover screen the
        // moment the user skips, and only the lifecycle advance moves it on.
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        assert_eq!(overlay.screen, Screen::SetupStart);

        overlay.on_setup_progress(SetupStep::SwitchingUplink, "");
        assert_eq!(overlay.screen, Screen::SetupSwitching);
        assert_eq!(overlay.view(), DeviceInfoView::TurningApOff);

        // No timer moves it: the teardown can take as long as it takes.
        let _ = overlay.tick(t0() + HOLD + HOLD);
        assert_eq!(overlay.screen, Screen::SetupSwitching);

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
    fn skipping_wifi_without_the_event_still_leaves_the_ap_screen() {
        // A lifecycle advance with no setup event (e.g. an overlay restarted
        // mid-teardown that missed the replay) must still leave the stale AP
        // screen for the connect flow.
        let mut overlay = DeviceInfoOverlay::default();
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        assert_eq!(overlay.screen, Screen::SetupStart);
        overlay.on_device_state(DeviceState::SetupPending, false);
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
    fn connection_failure_returns_to_setup_start_once_the_ap_is_back() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_setup_progress(SetupStep::WifiConnectionFailed, "");

        let _ = overlay.tick(t0() + HOLD + HOLD);
        assert_eq!(
            overlay.screen,
            Screen::SetupError,
            "no timer moves the failure screen; the AP may still be down"
        );

        overlay.on_access_point(Some(&AccessPoint {
            ssid: "Deck setup".to_owned(),
            setup_url: "http://10.0.0.21/".to_owned(),
        }));
        assert_eq!(overlay.screen, Screen::SetupStart);
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
    fn reconfig_success_mid_setup_returns_to_the_connect_info_in_either_order() {
        // bmc reads the lifecycle through a shell script after the join,
        // so the success event may land on either side of it.
        for success_first in [false, true] {
            let old_ip = Ipv4Addr::new(10, 0, 0, 5);
            let new_ip = Ipv4Addr::new(192, 168, 1, 20);
            let (mut overlay, prober) = overlay_with_prober(Some(old_ip));
            overlay.on_device_state(DeviceState::SetupPending, false);
            let _ = overlay.tick(t0());
            assert_eq!(
                overlay.screen,
                Screen::SetupConnectInfo { ip: Some(old_ip) }
            );

            overlay.on_device_state(DeviceState::WifiReconfiguration, false);
            overlay.on_access_point(Some(&setup_ap()));
            assert_eq!(
                overlay.view(),
                DeviceInfoView::SetupStart {
                    ap: Some(setup_ap()),
                    uplinks: Uplinks::WIFI_ONLY,
                },
                "success first: {success_first}"
            );
            overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
            // The join re-homes the station before either event arrives.
            prober.publish(Some(new_ip));
            if success_first {
                overlay.on_setup_progress(SetupStep::WifiReconfigSuccess, "");
                overlay.on_device_state(DeviceState::SetupPending, false);
            } else {
                overlay.on_device_state(DeviceState::SetupPending, false);
                // A poll in between self-advances the connecting screen
                // on the station address; the success still lands over it.
                let _ = overlay.tick(t0());
                overlay.on_setup_progress(SetupStep::WifiReconfigSuccess, "");
            }
            let start = t0();
            let tick = overlay.tick(start);
            assert!(tick.visible, "success first: {success_first}");
            assert!(
                matches!(overlay.screen, Screen::SetupConnected { .. }),
                "success first: {success_first}"
            );

            let tick = overlay.tick(start + HOLD);
            assert_eq!(
                overlay.screen,
                Screen::SetupConnectInfo { ip: Some(new_ip) },
                "success first: {success_first}"
            );
            assert!(tick.visible, "the wizard is not finished");
        }
    }

    #[test]
    fn reconfig_resumed_from_a_reboot_mid_setup_ends_on_the_connect_info() {
        // A reboot mid-reconfiguration keeps both flags, so the device boots
        // straight into WifiReconfiguration with the overlay cold.
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        let mut overlay = overlay_with_ip(Some(ip));
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        assert_eq!(overlay.screen, Screen::SetupStart);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        overlay.on_setup_progress(SetupStep::WifiReconfigSuccess, "");
        overlay.on_device_state(DeviceState::SetupPending, false);

        let start = t0();
        let _ = overlay.tick(start);
        let tick = overlay.tick(start + HOLD);
        assert_eq!(overlay.screen, Screen::SetupConnectInfo { ip: Some(ip) });
        assert!(tick.visible);
    }

    #[test]
    fn a_lifecycle_broadcast_after_the_hold_brings_the_wizard_back() {
        // The hold decides on the mode it has, so a broadcast slower than
        // the hold unmaps first; the scenes show until it lands.
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        let mut overlay = overlay_with_ip(Some(ip));
        overlay.on_device_state(DeviceState::SetupPending, false);
        let _ = overlay.tick(t0());
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        overlay.on_setup_progress(SetupStep::WifiReconfigSuccess, "");

        let start = t0();
        let tick = overlay.tick(start + HOLD);
        assert_eq!(
            overlay.screen,
            Screen::Done,
            "the hold ran out on the stale mode"
        );
        assert!(!tick.visible);

        overlay.on_device_state(DeviceState::SetupPending, false);
        let tick = overlay.tick(start + HOLD);
        assert_eq!(overlay.screen, Screen::SetupConnectInfo { ip: Some(ip) });
        assert!(tick.visible);
    }

    #[test]
    fn setup_start_never_times_out() {
        for state in [
            DeviceState::FactoryDefault,
            DeviceState::WifiReconfiguration,
        ] {
            let mut overlay = overlay_with_ip(None);
            overlay.on_device_state(state, false);
            let tick = overlay.tick(t0() + Duration::from_hours(1));
            assert_eq!(overlay.screen, Screen::SetupStart, "{state:?}");
            assert!(tick.visible, "{state:?}");
            assert_eq!(
                tick.next_wake, None,
                "{state:?}: only a setup event moves it"
            );
        }
    }

    #[test]
    fn a_fatal_screen_mid_setup_is_sticky() {
        // Neither variant steps aside from an unfinished wizard; both wait
        // for something outside the overlay: the device restarting,
        // or the user restarting it.
        for restarting in [true, false] {
            let mut overlay = overlay_with_ip(None);
            overlay.on_device_state(DeviceState::SetupPending, false);
            overlay.on_setup_progress(SetupStep::UnexpectedError { restarting }, "");
            let tick = overlay.tick(t0() + FATAL_SCREEN_TIMEOUT + HOLD);
            assert!(matches!(overlay.screen, Screen::SetupFatal { .. }));
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
    fn a_pending_restart_is_waited_out_even_with_scenes_behind_it() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: true }, "");
        let tick = overlay.tick(t0() + FATAL_SCREEN_TIMEOUT + HOLD);
        assert!(matches!(overlay.screen, Screen::SetupFatal { .. }));
        assert!(tick.visible, "the restart is worth waiting for");
        assert_eq!(tick.next_wake, None);
    }

    #[test]
    fn re_entering_setup_over_a_dismissible_fatal_brings_the_screens_back() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: false }, "");

        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        assert!(matches!(overlay.screen, Screen::SetupStart));
    }

    #[test]
    fn reconfiguring_from_setup_pending_starts_the_setup_flow() {
        // The tray button flips a SetupPending device into WifiReconfiguration.
        // Its AP is now up, so the screen telling the user about it
        // has to replace whatever the SetupPending flow had on show.
        for ip in [Some(Ipv4Addr::new(10, 0, 0, 5)), None] {
            let mut overlay = overlay_with_ip(ip);
            overlay.on_device_state(DeviceState::SetupPending, false);
            let _ = overlay.tick(t0());
            let before = overlay.screen;
            assert!(before.setup_in_progress(), "{ip:?}: {before:?}");

            overlay.on_device_state(DeviceState::WifiReconfiguration, false);
            assert_eq!(overlay.screen, Screen::SetupStart, "from {before:?}");
        }
    }

    #[test]
    fn the_join_moving_the_lifecycle_to_setup_pending_leaves_the_flow_alone() {
        // A first boot's join clears the factory flag, so SetupPending arrives
        // around the success event; whichever comes first, the flow stays.
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");
        overlay.on_setup_progress(SetupStep::WifiConnectionSuccess, "");

        overlay.on_device_state(DeviceState::SetupPending, false);
        assert!(matches!(overlay.screen, Screen::SetupConnected { .. }));
    }

    #[test]
    fn a_repeated_setup_state_leaves_a_live_flow_alone() {
        // The AP watch re-broadcasts the same state once the AP is verified up.
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::ConnectingToWifi, "HomeNet");

        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        assert_eq!(overlay.screen, Screen::SetupConnecting);
    }

    #[test]
    fn a_pending_restart_survives_a_lifecycle_change() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::SetupPending, false);
        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: true }, "");

        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        assert!(matches!(
            overlay.screen,
            Screen::SetupFatal {
                restarting: true,
                ..
            }
        ));
    }

    #[test]
    fn re_entering_setup_leaves_a_pending_restart_on_screen() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: true }, "");

        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        assert!(matches!(overlay.screen, Screen::SetupFatal { .. }));
    }

    #[test]
    fn a_fatal_screen_over_scenes_times_out() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: false }, "");
        // After the event, so the screen's own `since` cannot be later.
        let start = t0();

        let tick = overlay.tick(start);
        assert!(tick.visible, "the user still has to read it");
        assert!(tick.next_wake.is_some(), "a timeout is armed");

        let tick = overlay.tick(start + FATAL_SCREEN_TIMEOUT);
        assert_eq!(overlay.screen, Screen::Done);
        assert!(!tick.visible, "the device goes back to its scenes");
    }

    #[test]
    fn a_touch_dismisses_a_fatal_screen_that_has_scenes_behind_it() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: false }, "");
        overlay.on_touch(TouchEvent::Down {
            id: 0,
            x: 0.0,
            y: 0.0,
        });
        assert_eq!(overlay.screen, Screen::Done);
    }

    #[test]
    fn the_fatal_screen_says_whether_a_restart_is_coming() {
        let mut overlay = overlay_with_ip(None);
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: false }, "");
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupFatal {
                restarting: false,
                dismissible: true,
            }
        );

        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: true }, "");
        assert_eq!(
            overlay.view(),
            DeviceInfoView::SetupFatal {
                restarting: true,
                dismissible: false,
            }
        );
    }

    #[test]
    fn the_close_glyph_never_outlives_the_touch_that_backs_it() {
        // The view's flag is what draws the X, and `on_touch` is what honours
        // it; a screen offering one that does nothing is the failure here.
        for (state, dismissible) in [
            (DeviceState::WifiReconfiguration, true),
            (DeviceState::Operational, true),
            (DeviceState::FactoryDefault, false),
            (DeviceState::SetupPending, false),
        ] {
            let mut overlay = overlay_with_ip(None);
            overlay.on_device_state(state, false);
            overlay.on_setup_progress(SetupStep::UnexpectedError { restarting: false }, "");
            assert_eq!(
                overlay.view(),
                DeviceInfoView::SetupFatal {
                    restarting: false,
                    dismissible,
                },
                "{state:?}"
            );

            overlay.on_touch(TouchEvent::Down {
                id: 0,
                x: 0.0,
                y: 0.0,
            });
            assert_eq!(
                overlay.screen == Screen::Done,
                dismissible,
                "the touch must agree with the glyph: {state:?}"
            );
        }
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
            matches!(overlay.screen, Screen::SetupStart),
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

    /// A device sitting on its scenes, its boot sequence long spent.
    fn dismissed_operational(ip: Option<Ipv4Addr>) -> DeviceInfoOverlay {
        let mut overlay = overlay_with_ip(ip);
        overlay.on_device_state(DeviceState::Operational, true);
        let _ = overlay.tick(t0());
        assert_eq!(overlay.screen, Screen::Hidden);
        overlay
    }

    #[test]
    fn the_button_brings_the_address_back_after_the_boot_flow_is_spent() {
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        let mut overlay = dismissed_operational(Some(ip));

        overlay.on_report_ip();
        let start = t0();
        let tick = overlay.tick(start);

        assert!(tick.visible);
        assert_eq!(overlay.view(), DeviceInfoView::Success { ip });
        assert!(
            !overlay.tick(start + SUCCESS_VISIBLE_FOR + POLL).visible,
            "the screen must hand back to the scenes on its own timer"
        );
    }

    #[test]
    fn the_button_answers_with_the_failure_screen_when_there_is_no_address() {
        let mut overlay = dismissed_operational(None);

        overlay.on_report_ip();
        let _ = overlay.tick(t0());

        assert_eq!(
            overlay.view(),
            DeviceInfoView::Failed {
                link: Link::Wifi { ssid: None }
            }
        );
    }

    #[test]
    fn the_button_shows_the_address_that_changed_while_the_screen_was_away() {
        let (mut overlay, prober) = overlay_with_prober(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::Operational, true);
        let _ = overlay.tick(t0());
        assert_eq!(overlay.screen, Screen::Hidden);
        let renewed = Ipv4Addr::new(10, 0, 0, 7);
        prober.publish(Some(renewed));

        overlay.on_report_ip();

        assert!(matches!(overlay.screen, Screen::OpSuccess { ip, .. } if ip == renewed));
    }

    #[test]
    fn the_button_leaves_a_boot_still_waiting_for_its_address_alone() {
        let (mut overlay, prober) = overlay_with_prober(None);
        overlay.on_device_state(DeviceState::Operational, false);
        let start = t0();
        let _ = overlay.tick(start);
        assert!(matches!(overlay.screen, Screen::OpConnecting { .. }));

        overlay.on_report_ip();

        assert!(
            matches!(overlay.screen, Screen::OpConnecting { .. }),
            "a press before the lease must not end the wait with a failure screen"
        );
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        prober.publish(Some(ip));
        let _ = overlay.tick(start + POLL);
        assert_eq!(overlay.view(), DeviceInfoView::Success { ip });
    }

    #[test]
    fn the_button_leaves_the_post_upgrade_screen_alone() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_upgrade_state(succeeded(UpgradeKind::Firmware, Duration::from_secs(3)));
        overlay.on_device_state(DeviceState::Operational, false);
        assert!(matches!(overlay.screen, Screen::OpUpgraded { .. }));

        overlay.on_report_ip();

        assert!(matches!(overlay.screen, Screen::OpUpgraded { .. }));
    }

    #[test]
    fn the_button_leaves_a_setup_flow_alone() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::FactoryDefault, false);
        overlay.on_access_point(Some(&setup_ap()));

        overlay.on_report_ip();

        assert_eq!(
            overlay.screen,
            Screen::SetupStart,
            "a device mid-setup stays in the wizard"
        );
    }

    /// A reconfiguration that died after the lifecycle already went operational.
    fn fatal_over_scenes(restarting: bool) -> DeviceInfoOverlay {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));
        overlay.on_device_state(DeviceState::WifiReconfiguration, false);
        overlay.on_device_state(DeviceState::Operational, false);
        overlay.on_setup_progress(SetupStep::UnexpectedError { restarting }, "");
        assert!(matches!(overlay.screen, Screen::SetupFatal { .. }));
        overlay
    }

    #[test]
    fn the_button_sends_a_dismissible_fatal_away_like_a_touch_would() {
        let mut overlay = fatal_over_scenes(false);
        assert!(
            overlay.fatal_dismissible(),
            "the screen draws the close glyph"
        );

        overlay.on_report_ip();

        assert_eq!(
            overlay.view(),
            DeviceInfoView::Success {
                ip: Ipv4Addr::new(10, 0, 0, 5)
            }
        );
    }

    #[test]
    fn the_button_leaves_a_pending_restart_on_screen() {
        let mut overlay = fatal_over_scenes(true);
        assert!(
            !overlay.fatal_dismissible(),
            "the screen draws no close glyph"
        );

        overlay.on_report_ip();

        assert!(matches!(
            overlay.screen,
            Screen::SetupFatal {
                restarting: true,
                ..
            }
        ));
    }

    #[test]
    fn the_button_does_nothing_before_the_lifecycle_is_known() {
        let mut overlay = overlay_with_ip(Some(Ipv4Addr::new(10, 0, 0, 5)));

        overlay.on_report_ip();

        assert_eq!(overlay.screen, Screen::Hidden);
    }
}
