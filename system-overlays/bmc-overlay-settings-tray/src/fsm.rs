// Copyright (C) 2026  Braiins Systems s.r.o.

//! Pure hold-to-confirm state machines ported from the `settings-stub`:
//! the WiFi-reconfigure button (with a completion event) and the bare WiFi
//! reconnect button (fire-and-forget). Kept GPU-free so the hold/timeout edges
//! are unit-testable.

use std::time::{Duration, Instant};

/// Hold duration to confirm a WiFi action.
const HOLD: Duration = Duration::from_secs(3);
/// Max wait after firing reconfigure before giving up and showing the error.
const PENDING_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the transient failure message stays up.
const ERROR_DISPLAY: Duration = Duration::from_secs(3);

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
                if now.duration_since(since) >= PENDING_TIMEOUT {
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

    /// Caption for the reconfigure button, conveying hold progress and the
    /// transient error.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ButtonState::Idle {
                error_since: Some(_),
            } => "Couldn't start WiFi setup",
            ButtonState::Idle { error_since: None } => "Reconfigure WiFi",
            ButtonState::Holding { .. } => "Keep holding…",
            ButtonState::Pending { .. } => "Starting WiFi setup…",
            ButtonState::Active => "",
        }
    }
}

/// Hold-to-confirm state machine for the bare WiFi reconnect button. Unlike the
/// reconfigure button there is no completion event, so the sequence is fired
/// and forgotten; `Cooldown` only locks out a re-fire until the finger lifts so
/// a single sustained hold cannot spawn the sequence on every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReconnectState {
    #[default]
    Idle,
    Holding {
        since: Instant,
    },
    Cooldown,
}

/// Side effect the reconnect FSM asks the caller to perform on a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectAction {
    None,
    Spawn,
}

impl ReconnectState {
    /// Advance on a touch/timer tick. `pressed` = finger down on the button
    /// this frame. Mutates in place and returns any side effect.
    pub fn tick(&mut self, pressed: bool, now: Instant) -> ReconnectAction {
        let (next, action) = match *self {
            ReconnectState::Idle => {
                if pressed {
                    (
                        ReconnectState::Holding { since: now },
                        ReconnectAction::None,
                    )
                } else {
                    (ReconnectState::Idle, ReconnectAction::None)
                }
            }
            ReconnectState::Holding { since } => {
                if !pressed {
                    (ReconnectState::Idle, ReconnectAction::None)
                } else if now.duration_since(since) >= HOLD {
                    (ReconnectState::Cooldown, ReconnectAction::Spawn)
                } else {
                    (ReconnectState::Holding { since }, ReconnectAction::None)
                }
            }
            ReconnectState::Cooldown => {
                if pressed {
                    (ReconnectState::Cooldown, ReconnectAction::None)
                } else {
                    (ReconnectState::Idle, ReconnectAction::None)
                }
            }
        };
        *self = next;
        action
    }

    /// Keep ticking/repainting (short poll timeout) while the hold accrues.
    #[must_use]
    pub fn is_animating(self) -> bool {
        matches!(self, ReconnectState::Holding { .. })
    }
}

/// Hold duration to confirm a restart — deliberately longer than the WiFi
/// holds: it is the most destructive action in the tray.
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
    /// Mirrors `ReconnectState::Cooldown`.
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
                if now.duration_since(since) >= PENDING_TIMEOUT {
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

    /// Caption for the restart button. A declined reason (owned by the
    /// overlay) replaces the generic message while shows_message().
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            RestartState::Idle {
                message_since: Some(_),
            }
            | RestartState::Cooldown { .. } => "Restart failed",
            RestartState::Idle {
                message_since: None,
            } => "Restart",
            RestartState::Holding { .. } => "Keep holding…",
            RestartState::Pending { .. } => "Restarting…",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn hold_three_seconds_sends_reconfigure() {
        let t0 = Instant::now();
        let mut b = ButtonState::default();
        assert_eq!(b.tick(true, t0), FsmAction::None);
        assert_eq!(
            b.tick(true, t0 + Duration::from_secs(3)),
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
        b.tick(true, t0 + Duration::from_secs(3)); // -> Pending
        b.tick(true, t0 + Duration::from_secs(13)); // 10s pending timeout
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
        b.tick(true, t0 + Duration::from_secs(3)); // -> Pending
        b.on_wifi_ap(true);
        assert!(matches!(b, ButtonState::Active));
        b.on_wifi_ap(false); // setup ended
        b.tick(false, t0 + Duration::from_secs(4));
        assert!(matches!(b, ButtonState::Idle { .. }));
    }

    #[test]
    fn reconnect_hold_spawns_once_then_cooldown() {
        let t0 = Instant::now();
        let mut r = ReconnectState::default();
        assert_eq!(r.tick(true, t0), ReconnectAction::None);
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(3)),
            ReconnectAction::Spawn
        );
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(6)),
            ReconnectAction::None,
            "sustained hold does not re-spawn"
        );
    }

    #[test]
    fn restart_hold_five_seconds_sends_once() {
        let t0 = Instant::now();
        let mut r = RestartState::default();
        assert_eq!(r.tick(true, t0), RestartAction::None);
        assert_eq!(
            r.tick(true, t0 + Duration::from_secs(4)),
            RestartAction::None,
            "the 3s WiFi hold must not fire a restart"
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
        assert_eq!(r.label(), "Keep holding…", "a live hold swaps the caption");
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
        assert_eq!(r.label(), "Restart");
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
