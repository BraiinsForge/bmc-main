// Copyright (C) 2026  Braiins Systems s.r.o.

//! Server state and dispatch for the `deck_alarm_v1` protocol.
//!
//! Tracks the bound `deck_alarm_v1` resources (one per overlay client) and buffers
//! incoming requests as [`AlarmAction`]s the compositor loop drains and
//! forwards to bmc. The `ring`/`stop` event fan-out is a pure helper
//! over the resource list so it is testable without Wayland resources.

use ::deck_alarm_v1::server::deck_alarm_v1::{self, DeckAlarmV1};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::state::CompositorState;

/// A control request from the overlay, drained by the compositor loop and
/// forwarded to bmc through the widget action channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlarmAction {
    Dismiss,
    Snooze,
}

/// Tracks bound alarm resources and buffers overlay requests for the loop to
/// drain.
#[derive(Debug, Default)]
pub struct AlarmState {
    pub resources: Vec<DeckAlarmV1>,
    pub pending_actions: Vec<AlarmAction>,
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

    pub fn ring(&mut self, time: &str, label: &str, snooze_allowed: bool) {
        self.prune();
        for r in &self.resources {
            r.ring_alarm(time.to_owned(), label.to_owned(), u32::from(snooze_allowed));
        }
    }
    pub fn stop(&mut self) {
        self.prune();
        for r in &self.resources {
            r.stop_alarm();
        }
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
                state.alarm.pending_actions.push(AlarmAction::Snooze);
            }
            deck_alarm_v1::Request::DismissAlarm => {
                state.alarm.pending_actions.push(AlarmAction::Dismiss);
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
}
