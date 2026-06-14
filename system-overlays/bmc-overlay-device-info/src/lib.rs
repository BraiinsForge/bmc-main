// Copyright (C) 2026  Braiins Systems s.r.o.

//! Fullscreen operational-startup overlay: show WiFi/IP connection progress,
//! then success or failure, then unmap for the rest of the session.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use bmc_system_overlay::{
    LayerConfig, SystemOverlay, TickOutcome, TouchEvent, configured_station_ssid, primary_ipv4,
};

/// How long to wait for an IPv4 before showing the connection-failure state.
const WAIT_FOR_IP: Duration = Duration::from_secs(20);
/// How long the success state (connected + IP) stays up before auto-dismiss.
const SUCCESS_VISIBLE_FOR: Duration = Duration::from_secs(10);
/// How long the failure state stays up before auto-dismiss.
const FAILURE_VISIBLE_FOR: Duration = Duration::from_secs(5);
/// Connectivity re-check cadence while waiting for an address.
const POLL: Duration = Duration::from_secs(1);

/// Injected connectivity source so the state machine is unit-testable.
trait Env {
    fn ipv4(&self) -> Option<Ipv4Addr>;
    fn station_ssid(&self) -> Option<String>;
}

struct OsEnv;
impl Env for OsEnv {
    fn ipv4(&self) -> Option<Ipv4Addr> {
        primary_ipv4()
    }

    fn station_ssid(&self) -> Option<String> {
        configured_station_ssid()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Mapped immediately at operational startup; polling for an IPv4.
    Connecting { since: Instant },
    /// IPv4 appeared before timeout; show the last-known IP for a fixed duration.
    Success { since: Instant, ip: Ipv4Addr },
    /// Timeout expired without IPv4; show failure briefly.
    Failed { since: Instant },
    /// Touch/timeout dismissed; unmapped permanently.
    Done,
}

#[must_use]
fn phase_visible(phase: Phase) -> bool {
    !matches!(phase, Phase::Done)
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
    last_probe: Option<Instant>,
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
            last_probe: None,
            env: Box::new(OsEnv),
        }
    }
}

impl DeviceInfoOverlay {
    fn probe_if_due(&mut self, now: Instant) -> bool {
        if self
            .last_probe
            .is_some_and(|last| now.duration_since(last) < POLL)
        {
            return false;
        }

        self.last_probe = Some(now);
        let next_ip = self.env.ipv4();
        let next_ssid = self.env.station_ssid();
        let changed = self.ip != next_ip || self.ssid != next_ssid;
        self.ip = next_ip;
        self.ssid = next_ssid;
        changed
    }

    #[must_use]
    fn ssid_text(&self, fallback: &str) -> String {
        self.ssid
            .as_deref()
            .map_or_else(|| fallback.to_owned(), |ssid| format!("WiFi SSID: {ssid}"))
    }
}

impl SystemOverlay for DeviceInfoOverlay {
    fn layer_config(&self) -> LayerConfig {
        LayerConfig::fullscreen("bmc-overlay-device-info")
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        if matches!(self.phase, Phase::Done) {
            return TickOutcome {
                visible: false,
                wants_render: false,
                next_wake: None,
            };
        }

        let probe_changed = self.probe_if_due(now);
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
            Phase::Done => None,
        };
        TickOutcome {
            visible,
            wants_render: visible && (phase_changed || probe_changed),
            next_wake,
        }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "display dimensions fit comfortably in f32 mantissa"
        )]
        let (w, h) = (size.0 as f32, size.1 as f32);
        // Opaque: this fullscreen Top surface must fully occlude the offline
        // indicator on the Bottom layer while both are mapped at boot.
        r.fill_rect(0.0, 0.0, w, h, Color::from_rgba(0, 0, 0, 255));
        let (title, detail, footer) = match self.phase {
            Phase::Connecting { .. } => (
                "Connecting...",
                self.ssid_text("Waiting for WiFi connection"),
                Some("Waiting for IP address"),
            ),
            Phase::Success { ip, .. } => ("You're connected", format!("http://{ip}/"), None),
            Phase::Failed { .. } => (
                "Problem with connection.",
                self.ssid_text("No WiFi SSID configured"),
                Some("No IP address assigned"),
            ),
            Phase::Done => return,
        };

        draw_centered(r, title, w, h / 2.0 - 52.0, 44.0);
        draw_centered(r, &detail, w, h / 2.0, 32.0);
        if let Some(footer) = footer {
            draw_centered(r, footer, w, h / 2.0 + 44.0, 26.0);
        }
    }

    fn on_touch(&mut self, event: TouchEvent) {
        if matches!(event, TouchEvent::Down { .. }) {
            self.phase = Phase::Done;
        }
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
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn t0() -> Instant {
        Instant::now()
    }

    struct StaticEnv {
        ip: Option<Ipv4Addr>,
    }

    impl Env for StaticEnv {
        fn ipv4(&self) -> Option<Ipv4Addr> {
            self.ip
        }

        fn station_ssid(&self) -> Option<String> {
            None
        }
    }

    struct CountingEnv {
        calls: Rc<Cell<usize>>,
        ip: Option<Ipv4Addr>,
    }

    impl Env for CountingEnv {
        fn ipv4(&self) -> Option<Ipv4Addr> {
            self.calls.set(self.calls.get() + 1);
            self.ip
        }

        fn station_ssid(&self) -> Option<String> {
            None
        }
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
    fn tick_reuses_cached_probe_between_poll_intervals() {
        let start = t0();
        let calls = Rc::new(Cell::new(0));
        let mut overlay = DeviceInfoOverlay {
            phase: Phase::Connecting { since: start },
            ip: None,
            ssid: None,
            last_probe: None,
            env: Box::new(CountingEnv {
                calls: Rc::clone(&calls),
                ip: None,
            }),
        };

        let _ = overlay.tick(start);
        let _ = overlay.tick(start + Duration::from_millis(500));

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn touch_down_hides_immediately() {
        let start = t0();
        let mut overlay = DeviceInfoOverlay {
            phase: Phase::Connecting { since: start },
            ip: None,
            ssid: None,
            last_probe: None,
            env: Box::new(StaticEnv { ip: None }),
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
