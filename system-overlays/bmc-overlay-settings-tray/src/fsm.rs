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

//! Pure hold-to-confirm state machines ported from the `settings-stub`:
//! the WiFi-reconfigure button (with a completion event) and the restart button.
//! Kept GPU-free so the hold/timeout edges are unit-testable.

use std::time::{Duration, Instant};

use crate::ui::Phase;

/// Hold duration to confirm a WiFi action.
const HOLD: Duration = Duration::from_secs(5);
/// Max wait after firing reconfigure before giving up and showing the error.
/// bmc confirms only once the setup AP is up, and it keeps trying for about 24 s:
/// the AP activation wait polls 20 × 1 s (`ATTEMPTS_TO_ACTIVATE_AP` in bmc-net-drv),
/// then the SSID lookup retries 4 × 1 s (`SETUP_AP_SSID_ATTEMPTS` in bmc).
/// 30 s is bmc's own setup-AP window (`SETUP_AP_WINDOW`),
/// so the tray gives up only after bmc has.
/// That is the mac80211 path. The ESP32 path reflashes the module first,
/// about 20 s more, and its button stays off until the wait covers that.
pub(crate) const RECONFIGURE_TIMEOUT: Duration = Duration::from_secs(30);
/// Max wait after firing restart before giving up and showing the error.
const RESTART_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the transient failure message stays up.
pub(crate) const ERROR_DISPLAY: Duration = Duration::from_secs(3);

/// Fraction of `hold` elapsed since `since`, clamped to 0..=1.
fn hold_fraction(since: Instant, now: Instant, hold: Duration) -> f32 {
    (now.duration_since(since).as_secs_f32() / hold.as_secs_f32()).clamp(0.0, 1.0)
}

/// Hold-to-confirm button state machine for WiFi reconfiguration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Idle { error_since: Option<Instant> },
    Holding { since: Instant },
    Pending { since: Instant },
    Active,
}

impl Default for ButtonState {
    fn default() -> Self {
        ButtonState::Idle { error_since: None }
    }
}

/// Side effect the reconfigure FSM asks the caller to perform on a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsmAction {
    None,
    SendReconfigure,
}

impl ButtonState {
    /// Advance on a touch/timer tick. `pressed` = finger down on the button
    /// this frame. Mutates in place and returns any side effect.
    pub fn tick(&mut self, pressed: bool, now: Instant) -> FsmAction {
        let (next, action) = match *self {
            ButtonState::Idle { error_since } => {
                // Clear the transient error once it has been shown long enough.
                let error_since = error_since.filter(|t| now.duration_since(*t) < ERROR_DISPLAY);
                if pressed {
                    (ButtonState::Holding { since: now }, FsmAction::None)
                } else {
                    (ButtonState::Idle { error_since }, FsmAction::None)
                }
            }
            ButtonState::Holding { since } => {
                if !pressed {
                    (ButtonState::default(), FsmAction::None)
                } else if now.duration_since(since) >= HOLD {
                    (
                        ButtonState::Pending { since: now },
                        FsmAction::SendReconfigure,
                    )
                } else {
                    (ButtonState::Holding { since }, FsmAction::None)
                }
            }
            ButtonState::Pending { since } => {
                if now.duration_since(since) >= RECONFIGURE_TIMEOUT {
                    (
                        ButtonState::Idle {
                            error_since: Some(now),
                        },
                        FsmAction::None,
                    )
                } else {
                    (ButtonState::Pending { since }, FsmAction::None)
                }
            }
            ButtonState::Active => (ButtonState::Active, FsmAction::None),
        };
        *self = next;
        action
    }

    /// Advance on a wifi_ap event. `active` is whether setup mode is on.
    pub fn on_wifi_ap(&mut self, active: bool) {
        *self = if active {
            ButtonState::Active
        } else {
            match *self {
                ButtonState::Active | ButtonState::Pending { .. } => ButtonState::default(),
                ButtonState::Idle { .. } | ButtonState::Holding { .. } => *self,
            }
        };
    }

    /// Keep ticking/repainting (short poll timeout) while time-based.
    #[must_use]
    pub fn is_animating(self) -> bool {
        matches!(
            self,
            ButtonState::Holding { .. }
                | ButtonState::Pending { .. }
                | ButtonState::Idle {
                    error_since: Some(_)
                }
        )
    }

    /// What the notice reports: `None` at rest,
    /// and while setup mode hides the button.
    #[must_use]
    pub fn phase(self, now: Instant) -> Option<Phase> {
        match self {
            ButtonState::Idle { error_since: None } | ButtonState::Active => None,
            ButtonState::Holding { since } => Some(Phase::Holding {
                progress: hold_fraction(since, now, HOLD),
            }),
            ButtonState::Pending { .. } => Some(Phase::Pending),
            ButtonState::Idle {
                error_since: Some(_),
            } => Some(Phase::Failed),
        }
    }
}

const RESTART_HOLD: Duration = Duration::from_secs(5);

/// Hold-to-confirm state machine for the restart button. Restart is
/// relay-routed so bmc can decline it (e.g. during an upgrade); a decline or a
/// pending timeout surfaces as a transient message before snapping back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartState {
    Idle {
        message_since: Option<Instant>,
    },
    Holding {
        since: Instant,
    },
    Pending {
        since: Instant,
    },
    /// A decline or a pending timeout is being surfaced, and re-arming is
    /// locked out until the finger lifts. Without this gate a finger still held
    /// through a decline would immediately re-enter `Holding` — wiping the
    /// reason before it renders and re-firing the restart every hold period.
    Cooldown {
        message_since: Instant,
    },
}

impl Default for RestartState {
    fn default() -> Self {
        RestartState::Idle {
            message_since: None,
        }
    }
}

/// Side effect the restart FSM asks the caller to perform on a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartAction {
    None,
    SendRestart,
}

impl RestartState {
    /// Advance on a touch/timer tick. On success no event ever arrives — the
    /// device reboots and the overlay dies in Pending.
    pub fn tick(&mut self, pressed: bool, now: Instant) -> RestartAction {
        let (next, action) = match *self {
            RestartState::Idle { message_since } => {
                let message_since =
                    message_since.filter(|t| now.duration_since(*t) < ERROR_DISPLAY);
                if pressed {
                    (RestartState::Holding { since: now }, RestartAction::None)
                } else {
                    (RestartState::Idle { message_since }, RestartAction::None)
                }
            }
            RestartState::Holding { since } => {
                if !pressed {
                    (RestartState::default(), RestartAction::None)
                } else if now.duration_since(since) >= RESTART_HOLD {
                    (
                        RestartState::Pending { since: now },
                        RestartAction::SendRestart,
                    )
                } else {
                    (RestartState::Holding { since }, RestartAction::None)
                }
            }
            RestartState::Pending { since } => {
                if now.duration_since(since) >= RESTART_TIMEOUT {
                    (
                        RestartState::Cooldown { message_since: now },
                        RestartAction::None,
                    )
                } else {
                    (RestartState::Pending { since }, RestartAction::None)
                }
            }
            RestartState::Cooldown { message_since } => {
                if pressed {
                    (
                        RestartState::Cooldown { message_since },
                        RestartAction::None,
                    )
                } else {
                    (
                        RestartState::Idle {
                            message_since: Some(message_since)
                                .filter(|t| now.duration_since(*t) < ERROR_DISPLAY),
                        },
                        RestartAction::None,
                    )
                }
            }
        };
        *self = next;
        action
    }

    /// A restart_declined event arrived: surface the reason and lock out
    /// re-arming until the finger lifts (via `Cooldown`), so a sustained hold
    /// neither hides the reason nor re-fires the restart.
    pub fn on_declined(&mut self, now: Instant) {
        *self = RestartState::Cooldown { message_since: now };
    }

    /// Whether the transient decline/timeout message is currently shown.
    #[must_use]
    pub fn shows_message(self) -> bool {
        matches!(
            self,
            RestartState::Idle {
                message_since: Some(_)
            } | RestartState::Cooldown { .. }
        )
    }

    /// Keep ticking/repainting (short poll timeout) while time-based.
    #[must_use]
    pub fn is_animating(self) -> bool {
        matches!(
            self,
            RestartState::Holding { .. }
                | RestartState::Pending { .. }
                | RestartState::Cooldown { .. }
                | RestartState::Idle {
                    message_since: Some(_)
                }
        )
    }

    /// What the notice reports: `None` at rest.
    #[must_use]
    pub fn phase(self, now: Instant) -> Option<Phase> {
        match self {
            RestartState::Idle {
                message_since: None,
            } => None,
            RestartState::Holding { since } => Some(Phase::Holding {
                progress: hold_fraction(since, now, RESTART_HOLD),
            }),
            RestartState::Pending { .. } => Some(Phase::Pending),
            RestartState::Idle {
                message_since: Some(_),
            }
            | RestartState::Cooldown { .. } => Some(Phase::Failed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn hold_five_seconds_sends_reconfigure() {
        let t0 = Instant::now();
        let mut b = ButtonState::default();
        assert_eq!(b.tick(true, t0), FsmAction::None);
        assert_eq!(b.tick(true, t0 + Duration::from_secs(4)), FsmAction::None);
        assert_eq!(
            b.tick(true, t0 + Duration::from_secs(5)),
            FsmAction::SendReconfigure
        );
    }

    #[test]
    fn release_before_threshold_resets() {
        let t0 = Instant::now();
        let mut b = ButtonState::default();
        b.tick(true, t0);
        assert_eq!(b.tick(false, t0 + Duration::from_secs(1)), FsmAction::None);
        assert!(matches!(b, ButtonState::Idle { .. }));
    }

    #[test]
    fn pending_times_out_to_error() {
        let t0 = Instant::now();
        let mut b = ButtonState::default();
        b.tick(true, t0);
        let fired = t0 + Duration::from_secs(5);
        b.tick(true, fired);
        b.tick(
            true,
            fired + RECONFIGURE_TIMEOUT.saturating_sub(Duration::from_millis(1)),
        );
        assert!(
            matches!(b, ButtonState::Pending { .. }),
            "bmc may still be bringing the AP up"
        );
        b.tick(true, fired + RECONFIGURE_TIMEOUT);
        assert!(matches!(
            b,
            ButtonState::Idle {
                error_since: Some(_)
            }
        ));
    }

    #[test]
    fn wifi_ap_active_moves_pending_to_active() {
        let t0 = Instant::now();
        let mut b = ButtonState::default();
        b.tick(true, t0);
        b.tick(true, t0 + Duration::from_secs(5));
        b.on_wifi_ap(true);
        assert!(matches!(b, ButtonState::Active));
        b.on_wifi_ap(false);
        b.tick(false, t0 + Duration::from_secs(6));
        assert!(matches!(b, ButtonState::Idle { .. }));
    }

    #[test]
    fn restart_hold_five_seconds_sends_once() {
        let t0 = Instant::now();
        let mut r = RestartState::default();
        assert_eq!(r.tick(true, t0), RestartAction::None);
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(4)),
            RestartAction::None,
            "the five-second hold must not fire early"
        );
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(5)),
            RestartAction::SendRestart
        );
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(6)),
            RestartAction::None,
            "pending: a sustained hold must not re-send"
        );
    }

    #[test]
    fn restart_release_before_threshold_snaps_back() {
        let t0 = Instant::now();
        let mut r = RestartState::default();
        r.tick(true, t0);
        assert!(
            matches!(r.phase(t0), Some(Phase::Holding { .. })),
            "a live hold reports itself"
        );
        r.tick(false, t0 + Duration::from_secs(3));
        assert!(
            matches!(
                r,
                RestartState::Idle {
                    message_since: None
                }
            ),
            "early release snaps back to idle"
        );
        assert_eq!(r.phase(t0 + Duration::from_secs(3)), None);
    }

    #[test]
    fn restart_declined_shows_message_then_idles() {
        let t0 = Instant::now();
        let mut r = RestartState::default();
        r.tick(true, t0);
        r.tick(true, t0 + Duration::from_secs(5)); // -> Pending
        r.on_declined(t0 + Duration::from_secs(6));
        assert!(r.shows_message(), "a decline must surface its reason");
        r.tick(false, t0 + Duration::from_secs(10)); // past ERROR_DISPLAY
        assert!(!r.shows_message());
        assert!(matches!(
            r,
            RestartState::Idle {
                message_since: None
            }
        ));
    }

    #[test]
    fn restart_pending_times_out_to_message() {
        let t0 = Instant::now();
        let mut r = RestartState::default();
        r.tick(true, t0);
        r.tick(true, t0 + Duration::from_secs(5)); // -> Pending
        r.tick(true, t0 + Duration::from_secs(16)); // 10s pending timeout
        assert!(r.shows_message());
    }

    #[test]
    fn restart_declined_while_held_persists_and_never_refires() {
        let t0 = Instant::now();
        let mut r = RestartState::default();
        r.tick(true, t0);
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(5)),
            RestartAction::SendRestart
        );
        r.on_declined(t0 + Duration::from_secs(6));
        // The finger is still down when the decline arrives (the common case):
        // the reason must stay visible and the hold must not silently re-arm.
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(7)),
            RestartAction::None
        );
        assert!(
            r.shows_message(),
            "the decline reason survives a sustained hold"
        );
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(13)),
            RestartAction::None,
            "a sustained hold must not re-fire restart after a decline"
        );
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(19)),
            RestartAction::None
        );
        // Only lifting the finger releases the lock-out and clears the message.
        r.tick(false, t0 + Duration::from_secs(20));
        assert!(matches!(
            r,
            RestartState::Idle {
                message_since: None
            }
        ));
    }

    /// Abs-diff float assertion (`float_cmp` is denied by the workspace lints).
    fn assert_close(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "{what}: expected ~{expected}, got {actual}"
        );
    }

    fn progress_at(phase: Option<Phase>) -> f32 {
        match phase {
            Some(Phase::Holding { progress }) => progress,
            other => panic!("expected a hold, got {other:?}"),
        }
    }

    #[test]
    fn progress_grows_while_holding() {
        let t0 = Instant::now();
        let mut b = ButtonState::default();
        b.tick(true, t0);
        assert_close(
            progress_at(b.phase(t0 + Duration::from_millis(2500))),
            0.5,
            "half of five-second hold",
        );
        assert_close(
            progress_at(b.phase(t0 + Duration::from_secs(10))),
            1.0,
            "clamped",
        );

        let mut r = RestartState::default();
        r.tick(true, t0);
        assert_close(
            progress_at(r.phase(t0 + Duration::from_millis(2500))),
            0.5,
            "restart holds for 5s, so 2.5s is half",
        );
    }

    #[test]
    fn reconfigure_reports_every_phase_and_rests_silent() {
        let t0 = Instant::now();
        let mut b = ButtonState::default();
        assert_eq!(b.phase(t0), None);
        b.tick(true, t0);
        assert!(matches!(b.phase(t0), Some(Phase::Holding { .. })));
        let fired = t0 + Duration::from_secs(5);
        b.tick(true, fired);
        assert_eq!(b.phase(fired), Some(Phase::Pending));
        let timed_out = fired + RECONFIGURE_TIMEOUT;
        b.tick(true, timed_out);
        assert_eq!(b.phase(timed_out), Some(Phase::Failed));
        b.on_wifi_ap(true);
        assert_eq!(b.phase(timed_out), None, "setup mode hides the button");
    }

    #[test]
    fn restart_reports_every_phase_and_rests_silent() {
        let t0 = Instant::now();
        let mut r = RestartState::default();
        assert_eq!(r.phase(t0), None);
        r.tick(true, t0);
        assert!(matches!(r.phase(t0), Some(Phase::Holding { .. })));
        let fired = t0 + Duration::from_secs(5);
        r.tick(true, fired);
        assert_eq!(r.phase(fired), Some(Phase::Pending));
        r.on_declined(fired);
        assert_eq!(
            r.phase(fired),
            Some(Phase::Failed),
            "held through a decline"
        );
        let released = fired + Duration::from_secs(1);
        r.tick(false, released);
        assert_eq!(
            r.phase(released),
            Some(Phase::Failed),
            "the message outlives the lift"
        );
    }

    #[test]
    fn restart_pending_timeout_while_held_never_refires() {
        let t0 = Instant::now();
        let mut r = RestartState::default();
        r.tick(true, t0);
        r.tick(true, t0 + Duration::from_secs(5)); // -> Pending, fires once
        r.tick(true, t0 + Duration::from_secs(16)); // 10s timeout -> Cooldown
        r.tick(true, t0 + Duration::from_secs(22)); // still held, still locked
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(27)),
            RestartAction::None,
            "held through a pending timeout must not re-arm and re-fire"
        );
    }
}
