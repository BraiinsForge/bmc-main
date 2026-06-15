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
}
