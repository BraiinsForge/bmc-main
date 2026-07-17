// Copyright (C) 2026  Braiins Systems s.r.o.

//! Server state and dispatch for the `deck_alarm_v1` protocol.
//!
//! Tracks the bound `deck_alarm_v1` resources (one per overlay client) and buffers
//! incoming requests as [`AlarmAction`]s the compositor loop drains and
//! forwards to bmc. The `ring`/`stop` event fan-out is a pure helper
//! over the resource list so it is testable without Wayland resources.

use ::deck_alarm_v1::server::deck_alarm_v1::{self, DeckAlarmV1, Snooze};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::state::CompositorState;

/// Map the internal snooze-allowed bool to the wire enum.
fn snooze_flag(allowed: bool) -> Snooze {
    if allowed {
        Snooze::Allowed
    } else {
        Snooze::NotAllowed
    }
}

/// A control request from the overlay, drained by the compositor loop and
/// forwarded to bmc through the widget action channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlarmAction {
    Dismiss,
    Snooze,
}

/// The alarm currently ringing, retained so a late-binding overlay can be
/// replayed the `alarm_ringing` event on bind. `Some` between a `ring` and its
/// `stop`.
#[derive(Debug, Clone)]
struct Ring {
    time: String,
    label: String,
    snooze_allowed: bool,
}

/// Tracks bound alarm resources and buffers overlay requests for the loop to
/// drain.
#[derive(Debug, Default)]
pub struct AlarmState {
    pub resources: Vec<DeckAlarmV1>,
    pub pending_actions: Vec<AlarmAction>,
    /// The active ring, `Some` between a `ring` and its `stop`. Lets the loop's
    /// no-overlay fallback and touch-to-dismiss act only while an alarm is
    /// actually firing, and carries the payload replayed to a late binder.
    ringing: Option<Ring>,
}

impl AlarmState {
    /// Drop dead resources. Pure over the resource list. The disconnect
    /// backstop: a client that vanishes without a `destroy` is reaped here on
    /// the next emit.
    pub fn prune(&mut self) {
        self.resources.retain(Resource::is_alive);
    }

    /// Remove a session by resource identity, on its `destroy` request. Mirrors
    /// `screen_edge.rs` rather than depending on `is_alive` during the
    /// destructor.
    pub fn remove(&mut self, resource: &DeckAlarmV1) {
        self.resources.retain(|r| r != resource);
    }

    pub fn drain_actions(&mut self) -> Vec<AlarmAction> {
        std::mem::take(&mut self.pending_actions)
    }

    /// Buffer an overlay request, collapsing a duplicate of the still-pending
    /// tail. Stop/snooze are idempotent per ring, so two identical requests in
    /// one drain window (double-reported tap, impatient re-tap) are one intent
    /// — forwarding both would only flood the bmc command channel.
    fn push_action(&mut self, action: AlarmAction) {
        if self.pending_actions.last() != Some(&action) {
            self.pending_actions.push(action);
        }
    }

    /// Whether an alarm is currently ringing (between `ring` and `stop`).
    pub fn is_ringing(&self) -> bool {
        self.ringing.is_some()
    }

    pub fn ring(&mut self, time: &str, label: &str, snooze_allowed: bool) {
        self.prune();
        self.ringing = Some(Ring {
            time: time.to_owned(),
            label: label.to_owned(),
            snooze_allowed,
        });
        for r in &self.resources {
            r.alarm_ringing(
                time.to_owned(),
                label.to_owned(),
                snooze_flag(snooze_allowed),
            );
        }
    }
    pub fn stop(&mut self) {
        self.prune();
        self.ringing = None;
        for r in &self.resources {
            r.alarm_stopped();
        }
    }

    /// Replay the active ring to a freshly bound resource. Called from `bind` so
    /// an overlay that (re)binds mid-ring — e.g. a host that was absent at fire
    /// time or crashed and restarted within the fallback grace window — maps the
    /// alarm immediately instead of sitting idle while `has_live_overlay`
    /// already reports it live, which would otherwise satisfy the watchdog and
    /// suppress touch-to-dismiss with nothing on screen.
    fn replay_ring(&self, resource: &DeckAlarmV1) {
        if let Some(ring) = &self.ringing {
            resource.alarm_ringing(
                ring.time.clone(),
                ring.label.clone(),
                snooze_flag(ring.snooze_allowed),
            );
        }
    }

    /// Whether at least one overlay resource is still bound and alive. `false`
    /// means no overlay is present to render/dismiss a ringing alarm — either
    /// none bound at fire time or the overlay client died. Does not prune, so it
    /// is a cheap `&self` probe usable from the touch path.
    pub fn has_live_overlay(&self) -> bool {
        self.resources.iter().any(Resource::is_alive)
    }

    /// Queue a dismiss as if the overlay had requested it; drained by the loop
    /// into `AlarmCommand::Dismiss`. Used by the no-overlay fallback.
    pub fn request_dismiss(&mut self) {
        self.push_action(AlarmAction::Dismiss);
        self.ringing = None;
    }
}

impl GlobalDispatch<DeckAlarmV1, ()> for CompositorState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<DeckAlarmV1>,
        (): &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let resource = data_init.init(resource, ());
        // Replay the active ring to this new resource so a mid-ring (re)bind
        // maps immediately instead of defeating the watchdog; see `replay_ring`.
        state.alarm.replay_ring(&resource);
        state.alarm.resources.push(resource);
    }
}

impl Dispatch<DeckAlarmV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &DeckAlarmV1,
        request: deck_alarm_v1::Request,
        (): &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            deck_alarm_v1::Request::SnoozeAlarm => {
                state.alarm.push_action(AlarmAction::Snooze);
            }
            deck_alarm_v1::Request::DismissAlarm => {
                state.alarm.push_action(AlarmAction::Dismiss);
            }
            deck_alarm_v1::Request::Destroy => state.alarm.remove(resource),
            other => tracing::warn!("Unknown deck_alarm_v1 request: {other:?}"),
        }
    }
}

/// Advertise the `deck_alarm_v1` global.
pub fn create_global(display: &DisplayHandle) {
    display.create_global::<CompositorState, DeckAlarmV1, ()>(1, ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_actions_empties_the_buffer() {
        let mut s = AlarmState::default();
        s.pending_actions.push(AlarmAction::Snooze);
        assert_eq!(s.drain_actions(), vec![AlarmAction::Snooze]);
        assert!(s.drain_actions().is_empty());
    }

    #[test]
    fn push_action_collapses_same_tick_duplicates() {
        let mut s = AlarmState::default();
        s.push_action(AlarmAction::Dismiss);
        s.push_action(AlarmAction::Dismiss);
        assert_eq!(s.drain_actions(), vec![AlarmAction::Dismiss]);
        // A repeat after the drain is a fresh intent and goes through.
        s.push_action(AlarmAction::Dismiss);
        assert_eq!(s.drain_actions(), vec![AlarmAction::Dismiss]);
    }

    #[test]
    fn ring_and_stop_toggle_ringing() {
        let mut s = AlarmState::default();
        assert!(!s.is_ringing());
        s.ring("07:30", "Wake up", true);
        assert!(s.is_ringing());
        s.stop();
        assert!(!s.is_ringing());
    }

    #[test]
    fn ring_retains_replay_payload_until_cleared() {
        // The payload `replay_ring` re-sends to a late-binding overlay is kept
        // for the whole ring and dropped on stop, so a mid-ring (re)bind maps
        // the alarm instead of defeating the no-overlay watchdog. The wire
        // fan-out itself needs a live resource and is covered on-device.
        let mut s = AlarmState::default();
        assert!(s.ringing.is_none());
        s.ring("07:30", "Wake up", true);
        let ring = s.ringing.as_ref().expect("BUG: ringing after ring()");
        assert_eq!(
            (ring.time.as_str(), ring.label.as_str(), ring.snooze_allowed),
            ("07:30", "Wake up", true)
        );
        s.stop();
        assert!(s.ringing.is_none());
    }

    #[test]
    fn no_resources_means_no_live_overlay() {
        let s = AlarmState::default();
        assert!(!s.has_live_overlay());
    }

    #[test]
    fn request_dismiss_queues_dismiss_and_clears_ringing() {
        let mut s = AlarmState::default();
        s.ring("07:30", "", false);
        s.request_dismiss();
        assert!(!s.is_ringing());
        assert_eq!(s.drain_actions(), vec![AlarmAction::Dismiss]);
    }
}
