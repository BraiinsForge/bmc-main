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

//! Fullscreen operational-startup overlay: show WiFi/IP connection progress,
//! then success or failure, then unmap for the rest of the session.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use bmc_system_overlay::{
    Layer, LayerConfig, SnapshotVersion, SystemOverlay, TickOutcome, TouchEvent, UpgradeKind,
    UpgradeSnapshot, UpgradeState, VersionedSnapshot,
};

/// How long to wait for an IPv4 before showing the connection-failure state.
const WAIT_FOR_IP: Duration = Duration::from_secs(20);
/// How long the success state (connected + IP) stays up before auto-dismiss.
const SUCCESS_VISIBLE_FOR: Duration = Duration::from_secs(10);
/// How long the failure state stays up before auto-dismiss.
const FAILURE_VISIBLE_FOR: Duration = Duration::from_secs(5);
/// Snapshot re-read (wake) cadence while waiting for an address.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Mapped immediately at operational startup; polling for an IPv4.
    Connecting { since: Instant },
    /// A firmware-upgrade success screen owns the display; stay unmapped
    /// and start connecting once it is due to hide.
    Postponed { until: Instant },
    /// IPv4 appeared before timeout; show the last-known IP for a fixed duration.
    Success { since: Instant, ip: Ipv4Addr },
    /// Timeout expired without IPv4; show failure briefly.
    Failed { since: Instant },
    /// Touch/timeout dismissed; unmapped permanently.
    Done,
}

#[must_use]
fn phase_visible(phase: Phase) -> bool {
    !matches!(phase, Phase::Done | Phase::Postponed { .. })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceInfoView {
    Connecting { ssid: Option<String> },
    Success { ip: Ipv4Addr },
    Failed { ssid: Option<String> },
    Done,
}

/// Pure transition for one tick. Returns the next phase and whether the status
/// text changed so a redraw is warranted.
fn step(phase: Phase, now: Instant, ip: Option<Ipv4Addr>) -> (Phase, bool) {
    match phase {
        Phase::Connecting { since } => {
            if let Some(ip) = ip {
                (Phase::Success { since: now, ip }, true)
            } else if now.duration_since(since) >= WAIT_FOR_IP {
                (Phase::Failed { since: now }, true)
            } else {
                (Phase::Connecting { since }, false)
            }
        }
        Phase::Postponed { until } => {
            if now >= until {
                (Phase::Connecting { since: now }, true)
            } else {
                (Phase::Postponed { until }, false)
            }
        }
        Phase::Success {
            since,
            ip: shown_ip,
        } => {
            if now.duration_since(since) >= SUCCESS_VISIBLE_FOR {
                (Phase::Done, false)
            } else if let Some(ip) = ip.filter(|ip| *ip != shown_ip) {
                (Phase::Success { since, ip }, true)
            } else {
                // Deliberate: keep the last-known IP through transient
                // DHCP/interface loss. A short acquire-then-lose can therefore
                // show a stale IP for up to SUCCESS_VISIBLE_FOR; that is
                // accepted to avoid flicker. Dismissal is only touch-down or
                // the success/failure timeout.
                (
                    Phase::Success {
                        since,
                        ip: shown_ip,
                    },
                    false,
                )
            }
        }
        Phase::Failed { since } => {
            if now.duration_since(since) >= FAILURE_VISIBLE_FOR {
                (Phase::Done, false)
            } else {
                (Phase::Failed { since }, false)
            }
        }
        Phase::Done => (Phase::Done, false),
    }
}

pub struct DeviceInfoOverlay {
    phase: Phase,
    ip: Option<Ipv4Addr>,
    ssid: Option<String>,
    /// Version of the snapshot `ip`/`ssid` were read from (`None` = none
    /// yet); lets `refresh_from_snapshot` skip unchanged reads.
    snapshot_version: Option<SnapshotVersion>,
    env: Box<dyn Env>,
}

impl std::fmt::Debug for DeviceInfoOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceInfoOverlay")
            .field("phase", &self.phase)
            .field("ip", &self.ip)
            .field("ssid", &self.ssid)
            .finish_non_exhaustive()
    }
}

impl Default for DeviceInfoOverlay {
    fn default() -> Self {
        Self {
            phase: Phase::Connecting {
                since: Instant::now(),
            },
            ip: None,
            ssid: None,
            snapshot_version: None,
            env: Box::new(OsEnv),
        }
    }
}

impl DeviceInfoOverlay {
    #[must_use]
    fn view(&self) -> DeviceInfoView {
        match self.phase {
            Phase::Connecting { .. } => DeviceInfoView::Connecting {
                ssid: self.ssid.clone(),
            },
            Phase::Success { ip, .. } => DeviceInfoView::Success { ip },
            Phase::Failed { .. } => DeviceInfoView::Failed {
                ssid: self.ssid.clone(),
            },
            Phase::Postponed { .. } | Phase::Done => DeviceInfoView::Done,
        }
    }

    /// Fold a changed snapshot into the displayed IP/SSID; returns whether
    /// either changed. No change yet (prober has not published) keeps the
    /// current values — unknown renders the same as "no IP yet". The inner
    /// comparison stays because a signal-strength-only change bumps the
    /// version without touching the fields shown here.
    fn refresh_from_snapshot(&mut self) -> bool {
        let Some(VersionedSnapshot { version, snapshot }) =
            self.env.snapshot_if_changed(self.snapshot_version)
        else {
            return false;
        };
        self.snapshot_version = Some(version);
        let changed = self.ip != snapshot.ipv4 || self.ssid != snapshot.station_ssid;
        self.ip = snapshot.ipv4;
        self.ssid = snapshot.station_ssid;
        changed
    }
}

#[must_use]
fn ssid_text(ssid: Option<&str>, fallback: &str) -> String {
    ssid.map_or_else(|| fallback.to_owned(), |ssid| format!("WiFi SSID: {ssid}"))
}

impl SystemOverlay for DeviceInfoOverlay {
    fn layer_config(&self) -> LayerConfig {
        // Bottom, not the fullscreen default of Top: the startup screen must
        // sit below a firing alarm (Top) and the settings tray (Overlay) if
        // either maps during boot, while still occluding the scene.
        LayerConfig {
            layer: Layer::Bottom,
            ..LayerConfig::fullscreen("bmc-overlay-device-info")
        }
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        if matches!(self.phase, Phase::Done) {
            return TickOutcome {
                visible: false,
                wants_render: false,
                next_wake: None,
            };
        }

        let probe_changed = self.refresh_from_snapshot();
        let (next, phase_changed) = step(self.phase, now, self.ip);
        self.phase = next;
        let visible = phase_visible(self.phase);
        let next_wake = match self.phase {
            Phase::Connecting { since } => {
                let poll = now + POLL;
                let deadline = since + WAIT_FOR_IP;
                Some(if poll < deadline { poll } else { deadline })
            }
            Phase::Success { since, .. } => Some(since + SUCCESS_VISIBLE_FOR),
            Phase::Failed { since } => Some(since + FAILURE_VISIBLE_FOR),
            Phase::Postponed { until } => Some(until),
            Phase::Done => None,
        };
        TickOutcome {
            visible,
            wants_render: visible && (phase_changed || probe_changed),
            next_wake,
        }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        render_device_info(r, size, &self.view());
    }

    fn on_touch(&mut self, event: TouchEvent) {
        if matches!(event, TouchEvent::Down { .. }) {
            self.phase = Phase::Done;
        }
    }

    fn uses_upgrade(&self) -> bool {
        true
    }

    /// A success snapshot while still `Connecting` marks this startup
    /// as post-upgrade. A package restart never dropped the network:
    /// skip the connection screen. A firmware success keeps its overlay
    /// in front: wait out that dwell before connecting.
    fn on_upgrade_state(&mut self, snapshot: UpgradeSnapshot) {
        if !matches!(self.phase, Phase::Connecting { .. }) {
            return;
        }
        let UpgradeState::Succeeded { remaining } = snapshot.state else {
            return;
        };
        if snapshot.kind == UpgradeKind::Packages {
            // TODO(BDK-450): only the regular show-the-IP screen is safe to skip here.
            // Factory-default or config-init screens, once added,
            // must still run after a package restart.
            self.phase = Phase::Done;
        } else if snapshot.kind == UpgradeKind::Firmware {
            self.phase = Phase::Postponed {
                until: Instant::now() + remaining,
            };
        }
    }
}

pub fn render_device_info(r: &mut dyn Renderer, size: (u32, u32), view: &DeviceInfoView) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "display dimensions fit comfortably in f32 mantissa"
    )]
    let (w, h) = (size.0 as f32, size.1 as f32);

    r.fill_rect(0.0, 0.0, w, h, Color::from_rgba(0, 0, 0, 255));

    let (title, detail, footer) = match view {
        DeviceInfoView::Connecting { ssid } => (
            "Connecting...",
            ssid_text(ssid.as_deref(), "Waiting for WiFi connection"),
            Some("Waiting for IP address"),
        ),
        DeviceInfoView::Success { ip } => ("You're connected", format!("http://{ip}/"), None),
        DeviceInfoView::Failed { ssid } => (
            "Problem with connection.",
            ssid_text(ssid.as_deref(), "No WiFi SSID configured"),
            Some("No IP address assigned"),
        ),
        DeviceInfoView::Done => return,
    };

    draw_centered(r, title, w, h / 2.0 - 52.0, 44.0);
    draw_centered(r, &detail, w, h / 2.0, 32.0);
    if let Some(footer) = footer {
        draw_centered(r, footer, w, h / 2.0 + 44.0, 26.0);
    }
}

fn draw_centered(r: &mut dyn Renderer, text: &str, width: f32, y: f32, font: f32) {
    let text_width = r.measure_text(text, font);
    r.draw_text(
        text,
        (width - text_width) / 2.0,
        y,
        font,
        Color::from_rgba(255, 255, 255, 255),
    );
}

#[cfg(test)]
mod tests {
    use bmc_system_overlay::Snapshot;

    use super::*;

    fn t0() -> Instant {
        Instant::now()
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

    fn env_with_ip(ip: Option<Ipv4Addr>) -> Box<dyn Env> {
        Box::new(StaticEnv {
            snapshot: Some(Snapshot {
                ipv4: ip,
                station_ssid: None,
                wifi_signal_dbm: None,
            }),
        })
    }

    #[test]
    fn connecting_is_visible_without_ip() {
        let start = t0();
        let (next, changed) = step(Phase::Connecting { since: start }, start + POLL, None);

        assert_eq!(next, Phase::Connecting { since: start });
        assert!(phase_visible(next));
        assert!(!changed);
    }

    #[test]
    fn connecting_succeeds_when_ip_appears() {
        let now = t0();
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        let (next, changed) = step(Phase::Connecting { since: now }, now + POLL, Some(ip));

        assert_eq!(
            next,
            Phase::Success {
                since: now + POLL,
                ip
            }
        );
        assert!(changed);
    }

    #[test]
    fn connecting_fails_after_ip_timeout() {
        let start = t0();
        let later = start + WAIT_FOR_IP;
        let (next, changed) = step(Phase::Connecting { since: start }, later, None);

        assert_eq!(next, Phase::Failed { since: later });
        assert!(changed);
    }

    #[test]
    fn success_auto_dismisses_after_display_duration() {
        let start = t0();
        let (next, _) = step(
            Phase::Success {
                since: start,
                ip: Ipv4Addr::new(10, 0, 0, 5),
            },
            start + SUCCESS_VISIBLE_FOR,
            Some(Ipv4Addr::new(10, 0, 0, 5)),
        );

        assert_eq!(next, Phase::Done);
    }

    #[test]
    fn success_keeps_last_ip_through_transient_probe_loss() {
        let start = t0();
        let shown_ip = Ipv4Addr::new(10, 0, 0, 5);
        let (next, changed) = step(
            Phase::Success {
                since: start,
                ip: shown_ip,
            },
            start + POLL,
            None,
        );

        assert_eq!(
            next,
            Phase::Success {
                since: start,
                ip: shown_ip,
            }
        );
        assert!(!changed);
    }

    #[test]
    fn unknown_snapshot_stays_connecting_before_deadline() {
        let start = t0();
        let mut overlay = DeviceInfoOverlay {
            phase: Phase::Connecting { since: start },
            ip: None,
            ssid: None,
            snapshot_version: None,
            env: Box::new(StaticEnv { snapshot: None }),
        };

        let tick = overlay.tick(start + POLL);

        assert_eq!(overlay.phase, Phase::Connecting { since: start });
        assert!(tick.visible);
    }

    #[test]
    fn offline_snapshot_fails_after_deadline() {
        let start = t0();
        let mut overlay = DeviceInfoOverlay {
            phase: Phase::Connecting { since: start },
            ip: None,
            ssid: None,
            snapshot_version: None,
            env: env_with_ip(None),
        };

        let _ = overlay.tick(start + WAIT_FOR_IP);

        assert!(matches!(overlay.phase, Phase::Failed { .. }));
    }

    #[test]
    fn view_for_connecting_includes_configured_ssid() {
        let start = t0();
        let overlay = DeviceInfoOverlay {
            phase: Phase::Connecting { since: start },
            ip: None,
            ssid: Some("Braiins-WiFi".to_owned()),
            snapshot_version: None,
            env: Box::new(StaticEnv { snapshot: None }),
        };

        assert_eq!(
            overlay.view(),
            DeviceInfoView::Connecting {
                ssid: Some("Braiins-WiFi".to_owned())
            }
        );
    }

    #[test]
    fn view_for_success_includes_displayed_ip() {
        let start = t0();
        let ip = Ipv4Addr::new(192, 168, 1, 42);
        let overlay = DeviceInfoOverlay {
            phase: Phase::Success { since: start, ip },
            ip: Some(ip),
            ssid: None,
            snapshot_version: None,
            env: env_with_ip(Some(ip)),
        };

        assert_eq!(overlay.view(), DeviceInfoView::Success { ip });
    }

    fn connecting_overlay(start: Instant) -> DeviceInfoOverlay {
        DeviceInfoOverlay {
            phase: Phase::Connecting { since: start },
            ip: None,
            ssid: None,
            snapshot_version: None,
            env: Box::new(StaticEnv { snapshot: None }),
        }
    }

    fn succeeded(kind: UpgradeKind) -> UpgradeSnapshot {
        UpgradeSnapshot {
            kind,
            state: UpgradeState::Succeeded {
                remaining: Duration::from_secs(10),
            },
        }
    }

    #[test]
    fn a_package_activation_restart_skips_the_startup_screen() {
        let start = t0();
        let mut overlay = connecting_overlay(start);

        overlay.on_upgrade_state(succeeded(UpgradeKind::Packages));

        assert_eq!(overlay.phase, Phase::Done);
        assert!(!overlay.tick(start).visible);
    }

    #[test]
    fn a_firmware_success_postpones_connecting_past_its_dwell() {
        let start = t0();
        let mut overlay = connecting_overlay(start);

        overlay.on_upgrade_state(succeeded(UpgradeKind::Firmware));

        let Phase::Postponed { until } = overlay.phase else {
            panic!("firmware success must postpone, got {:?}", overlay.phase);
        };
        let tick = overlay.tick(start);
        assert!(!tick.visible, "stay unmapped under the success screen");
        assert_eq!(tick.next_wake, Some(until));

        let (next, changed) = step(Phase::Postponed { until }, until, None);
        assert_eq!(
            next,
            Phase::Connecting { since: until },
            "the full connect window must start after the dwell"
        );
        assert!(changed);
    }

    #[test]
    fn a_running_package_upgrade_keeps_the_startup_screen() {
        let start = t0();
        let mut overlay = connecting_overlay(start);

        overlay.on_upgrade_state(UpgradeSnapshot {
            kind: UpgradeKind::Packages,
            state: UpgradeState::Running {
                phase: None,
                progress: None,
            },
        });

        assert_eq!(overlay.phase, Phase::Connecting { since: start });
    }

    #[test]
    fn a_live_upgrade_success_does_not_restart_a_shown_screen() {
        let start = t0();
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        let mut overlay = connecting_overlay(start);
        overlay.phase = Phase::Success { since: start, ip };

        overlay.on_upgrade_state(succeeded(UpgradeKind::Firmware));

        assert_eq!(overlay.phase, Phase::Success { since: start, ip });
    }

    #[test]
    fn touch_down_hides_immediately() {
        let start = t0();
        let mut overlay = DeviceInfoOverlay {
            phase: Phase::Connecting { since: start },
            ip: None,
            ssid: None,
            snapshot_version: None,
            env: Box::new(StaticEnv { snapshot: None }),
        };

        overlay.on_touch(TouchEvent::Down {
            id: 0,
            x: 0.0,
            y: 0.0,
        });
        let tick = overlay.tick(start);

        assert_eq!(overlay.phase, Phase::Done);
        assert!(!tick.visible);
        assert_eq!(tick.next_wake, None);
    }
}
