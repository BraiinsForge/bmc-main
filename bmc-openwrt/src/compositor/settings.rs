// Copyright (C) 2026  Braiins Systems s.r.o.

//! Server state and dispatch for the vendored `deck_settings_v1` protocol.
//!
//! Tracks the bound `deck_settings_v1` resources (one per overlay client), the
//! last effective brightness, volume, night-mode state, and setup-AP SSID
//! (replayed on bind), and buffers incoming requests as [`SettingsAction`]s the
//! compositor loop drains and forwards to bmc. The cached state and request
//! buffer are testable without live Wayland resources.

use ::deck_settings_v1::server::deck_settings_v1::{self, Capability, DeckSettingsV1};
use bmc_platform::Product;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::state::CompositorState;

/// A control request from the overlay, drained by the compositor loop and
/// forwarded to bmc through the widget action channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    SetBrightness(u8),
    SetVolume(u8),
    ToggleNightMode,
    Restart,
    ReconfigureWifi,
}

/// Tracks bound settings resources and the values replayed to late binders.
#[derive(Debug)]
pub struct SettingsState {
    pub resources: Vec<DeckSettingsV1>,
    /// Static per-product capability set, emitted first on every bind.
    pub caps: Capability,
    pub last_brightness: Option<u8>,
    pub last_volume: Option<u8>,
    pub last_wifi_ap: Option<String>,
    pub last_night_mode: Option<(bool, Option<String>)>,
    pub pending_actions: Vec<SettingsAction>,
}

/// The wl_seat-style capability set for a hardware product. Sound hardware
/// exists only on BMC100; WiFi setup matches the tray's mac80211-only gate
/// (the BMM boards drive their ESP32 AP through a separate firmware path).
pub fn caps_for_product(product: Product) -> Capability {
    match product {
        Product::Bmc100 => Capability::Brightness | Capability::Sound | Capability::WifiSetup,
        Product::Bfm100 => Capability::Brightness | Capability::WifiSetup,
        Product::Bmm100 | Product::Bmm101 => Capability::Brightness,
    }
}

impl SettingsState {
    pub fn new(caps: Capability) -> Self {
        Self {
            resources: Vec::new(),
            caps,
            last_brightness: None,
            last_volume: None,
            last_wifi_ap: None,
            last_night_mode: None,
            pending_actions: Vec::new(),
        }
    }

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

    pub fn drain_actions(&mut self) -> Vec<SettingsAction> {
        std::mem::take(&mut self.pending_actions)
    }

    /// Record the effective brightness and emit it to every live resource.
    pub fn set_brightness(&mut self, value: u8) {
        self.last_brightness = Some(value);
        self.prune();
        for r in &self.resources {
            r.brightness(u32::from(value));
        }
    }

    /// Record the setup-AP SSID (`None` = inactive) and emit it; an inactive
    /// state is sent as the empty string.
    pub fn set_wifi_ap(&mut self, ssid: Option<String>) {
        self.last_wifi_ap.clone_from(&ssid);
        self.prune();
        let s = ssid.unwrap_or_default();
        for r in &self.resources {
            r.wifi_ap(s.clone());
        }
    }

    /// Record the effective volume and emit it to every live v2 resource with
    /// the sound capability; on non-sound platforms the emit dies here so bmc
    /// stays platform-agnostic.
    pub fn set_volume(&mut self, value: u8) {
        if !self.caps.contains(Capability::Sound) {
            return;
        }
        self.last_volume = Some(value);
        self.prune();
        for r in self.resources.iter().filter(|r| r.version() >= 2) {
            r.volume(u32::from(value));
        }
    }

    /// Record the night-mode state and emit it to every live v2 resource.
    pub fn set_night_mode(&mut self, active: bool, until: Option<String>) {
        self.prune();
        for r in self.resources.iter().filter(|r| r.version() >= 2) {
            r.night_mode(u32::from(active), until.clone());
        }
        self.last_night_mode = Some((active, until));
    }

    /// One-shot decline notification: no cache, no bind replay.
    pub fn restart_declined(&mut self, reason: &str) {
        self.prune();
        for r in self.resources.iter().filter(|r| r.version() >= 2) {
            r.restart_declined(reason.to_owned());
        }
    }

    /// Emit the preemption state to every live v3 resource: `true` while a
    /// modal full-screen overlay (alarm, startup) is covering the scene. The
    /// tray retracts on `true`. Edge-driven by the loop (see
    /// `CompositorState::modal_overlay_active`); no cache, no bind replay.
    pub fn set_preempted(&mut self, active: bool) {
        self.prune();
        for r in self.resources.iter().filter(|r| r.version() >= 3) {
            r.preempted(u32::from(active));
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
        // Capabilities go out first, before any value replay, so the overlay
        // can gate its controls before rendering. Replay current values so a
        // late binder learns the state immediately. Only emit brightness and
        // volume once bmc has reported a real value — emitting 0 on a cold
        // cache would snap the slider to minimum until the first broadcast.
        // wifi_ap empty-on-unknown is fine: empty legitimately means "setup
        // inactive".
        if resource.version() >= 2 {
            resource.capabilities(state.settings.caps);
        }
        if let Some(b) = state.settings.last_brightness {
            resource.brightness(u32::from(b));
        }
        if resource.version() >= 2 {
            if state.settings.caps.contains(Capability::Sound)
                && let Some(v) = state.settings.last_volume
            {
                resource.volume(u32::from(v));
            }
            if let Some((active, until)) = state.settings.last_night_mode.clone() {
                resource.night_mode(u32::from(active), until);
            }
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
                if !state.settings.caps.contains(Capability::Brightness) {
                    tracing::warn!("ignoring set_brightness: no brightness capability");
                    return;
                }
                let clamped = u8::try_from(value.min(100))
                    .expect("BUG: value.min(100) is in 0..=100, which fits u8");
                state
                    .settings
                    .pending_actions
                    .push(SettingsAction::SetBrightness(clamped));
            }
            deck_settings_v1::Request::SetVolume { value } => {
                if !state.settings.caps.contains(Capability::Sound) {
                    tracing::warn!("ignoring set_volume: no sound capability");
                    return;
                }
                let clamped = u8::try_from(value.min(100))
                    .expect("BUG: value.min(100) is in 0..=100, which fits u8");
                state
                    .settings
                    .pending_actions
                    .push(SettingsAction::SetVolume(clamped));
            }
            deck_settings_v1::Request::ToggleNightMode => {
                state
                    .settings
                    .pending_actions
                    .push(SettingsAction::ToggleNightMode);
            }
            deck_settings_v1::Request::Restart => {
                state.settings.pending_actions.push(SettingsAction::Restart);
            }
            deck_settings_v1::Request::ReconfigureWifi => {
                if !state.settings.caps.contains(Capability::WifiSetup) {
                    tracing::warn!("ignoring reconfigure_wifi: no wifi_setup capability");
                    return;
                }
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
    display.create_global::<CompositorState, DeckSettingsV1, ()>(3, ());
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
        let mut s = SettingsState::new(caps_for_product(Product::Bmc100));
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
        let mut s = SettingsState::new(caps_for_product(Product::Bmc100));
        s.pending_actions.push(SettingsAction::ReconfigureWifi);
        assert_eq!(s.drain_actions(), vec![SettingsAction::ReconfigureWifi]);
        assert!(s.drain_actions().is_empty());
    }

    #[test]
    fn caches_volume_and_night_mode_for_late_bind() {
        let mut s = SettingsState::new(caps_for_product(Product::Bmc100));
        s.set_volume(35);
        s.set_night_mode(true, Some("06:30".to_owned()));
        assert_eq!(s.last_volume, Some(35));
        assert_eq!(s.last_night_mode, Some((true, Some("06:30".to_owned()))));
    }

    #[test]
    fn does_not_cache_volume_without_sound_capability() {
        let mut s = SettingsState::new(caps_for_product(Product::Bfm100));
        s.set_volume(35);
        assert_eq!(s.last_volume, None);
    }

    // restart_declined is one-shot: bind replays exactly the caches below, so
    // proving they all stay empty proves a late binder never sees a decline.
    #[test]
    fn restart_declined_populates_no_replay_cache() {
        let mut s = SettingsState::new(caps_for_product(Product::Bmc100));
        s.restart_declined("upgrade in progress");
        assert_eq!(s.last_brightness, None);
        assert_eq!(s.last_volume, None);
        assert_eq!(s.last_wifi_ap, None);
        assert_eq!(s.last_night_mode, None);
    }

    #[test]
    fn caps_match_product_gates() {
        let all = caps_for_product(Product::Bmc100);
        assert!(all.contains(Capability::Brightness));
        assert!(all.contains(Capability::Sound));
        assert!(all.contains(Capability::WifiSetup));

        let bfm = caps_for_product(Product::Bfm100);
        assert!(bfm.contains(Capability::Brightness));
        assert!(!bfm.contains(Capability::Sound));
        assert!(bfm.contains(Capability::WifiSetup));

        for p in [Product::Bmm100, Product::Bmm101] {
            let bmm = caps_for_product(p);
            assert!(bmm.contains(Capability::Brightness));
            assert!(!bmm.contains(Capability::Sound));
            assert!(!bmm.contains(Capability::WifiSetup));
        }
    }
}
