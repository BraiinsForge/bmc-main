// Copyright (C) 2026  Braiins Systems s.r.o.

//! Server state and dispatch for the vendored `deck_settings_v1` protocol.
//!
//! Tracks the bound `deck_settings_v1` resources (one per overlay client), the
//! last effective brightness and setup-AP SSID (replayed on bind), and buffers
//! incoming requests as [`SettingsAction`]s the compositor loop drains and
//! forwards to bmc. The `brightness`/`wifi_ap` event fan-out is a pure helper
//! over the resource list so it is testable without Wayland resources.

use ::deck_settings_v1::server::deck_settings_v1::{self, DeckSettingsV1};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::state::CompositorState;

/// A control request from the overlay, drained by the compositor loop and
/// forwarded to bmc through the widget action channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    SetBrightness(u8),
    ReconfigureWifi,
}

/// Tracks bound settings resources and the values replayed to late binders.
#[derive(Debug, Default)]
pub struct SettingsState {
    pub resources: Vec<DeckSettingsV1>,
    pub last_brightness: Option<u8>,
    pub last_wifi_ap: Option<String>,
    pub pending_actions: Vec<SettingsAction>,
}

impl SettingsState {
    /// Drop dead resources. Pure over the resource list. The disconnect
    /// backstop: a client that vanishes without a `destroy` is reaped here on
    /// the next emit.
    pub fn prune(&mut self) {
        self.resources.retain(Resource::is_alive);
    }

    /// Remove a session by resource identity, on its `destroy` request. Mirrors
    /// `screen_edge.rs` rather than depending on `is_alive` during the
    /// destructor.
    pub fn remove(&mut self, resource: &DeckSettingsV1) {
        self.resources.retain(|r| r != resource);
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "drained and broadcast by the bmc-forwarding wiring step"
        )
    )]
    pub fn drain_actions(&mut self) -> Vec<SettingsAction> {
        std::mem::take(&mut self.pending_actions)
    }

    /// Record the effective brightness and emit it to every live resource.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "drained and broadcast by the bmc-forwarding wiring step"
        )
    )]
    pub fn set_brightness(&mut self, value: u8) {
        self.last_brightness = Some(value);
        self.prune();
        for r in &self.resources {
            r.brightness(u32::from(value));
        }
    }

    /// Record the setup-AP SSID (`None` = inactive) and emit it; an inactive
    /// state is sent as the empty string.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "drained and broadcast by the bmc-forwarding wiring step"
        )
    )]
    pub fn set_wifi_ap(&mut self, ssid: Option<String>) {
        self.last_wifi_ap.clone_from(&ssid);
        self.prune();
        let s = ssid.unwrap_or_default();
        for r in &self.resources {
            r.wifi_ap(s.clone());
        }
    }
}

impl GlobalDispatch<DeckSettingsV1, ()> for CompositorState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<DeckSettingsV1>,
        (): &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let resource = data_init.init(resource, ());
        // Replay current values so a late binder learns the state immediately.
        // Only emit brightness once bmc has reported a real value — emitting
        // brightness(0) on a cold cache would snap the slider to minimum until
        // the first broadcast. wifi_ap empty-on-unknown is fine: empty
        // legitimately means "setup inactive".
        if let Some(b) = state.settings.last_brightness {
            resource.brightness(u32::from(b));
        }
        resource.wifi_ap(state.settings.last_wifi_ap.clone().unwrap_or_default());
        state.settings.resources.push(resource);
    }
}

impl Dispatch<DeckSettingsV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &DeckSettingsV1,
        request: deck_settings_v1::Request,
        (): &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            deck_settings_v1::Request::SetBrightness { value } => {
                let clamped = u8::try_from(value.min(100)).unwrap_or(100);
                state
                    .settings
                    .pending_actions
                    .push(SettingsAction::SetBrightness(clamped));
            }
            deck_settings_v1::Request::ReconfigureWifi => {
                state
                    .settings
                    .pending_actions
                    .push(SettingsAction::ReconfigureWifi);
            }
            deck_settings_v1::Request::Destroy => {
                // Remove this session by resource identity, mirroring
                // screen_edge.rs — do not rely on `is_alive` timing during the
                // destructor (the spec requires the session drop on destroy).
                state.settings.remove(resource);
            }
            other => tracing::warn!("Unknown deck_settings_v1 request: {other:?}"),
        }
    }
}

/// Advertise the `deck_settings_v1` global.
pub fn create_global(display: &DisplayHandle) {
    display.create_global::<CompositorState, DeckSettingsV1, ()>(1, ());
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: the request clamp, `remove`-by-identity, and event fan-out all need
    // live Wayland resources, so they are exercised by the on-device path, not
    // faked here (a unit test that re-implements the clamp can never fail). The
    // two tests below cover the genuinely pure surface.

    #[test]
    fn caches_last_values_for_late_bind() {
        let mut s = SettingsState::default();
        // set_* without resources just records (fan-out is a no-op).
        s.set_brightness(42);
        s.set_wifi_ap(Some("Deck setup".to_owned()));
        assert_eq!(s.last_brightness, Some(42));
        assert_eq!(s.last_wifi_ap.as_deref(), Some("Deck setup"));
        s.set_wifi_ap(None);
        assert_eq!(s.last_wifi_ap, None);
    }

    #[test]
    fn drain_actions_empties_the_buffer() {
        let mut s = SettingsState::default();
        s.pending_actions.push(SettingsAction::ReconfigureWifi);
        assert_eq!(s.drain_actions(), vec![SettingsAction::ReconfigureWifi]);
        assert!(s.drain_actions().is_empty());
    }
}
