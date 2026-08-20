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

//! Server state and dispatch for the `deck_device_info_v1` protocol.
//!
//! Tracks the bound `deck_device_info_v1` resources (one per overlay client)
//! and the last device state, setup progress, and access point,
//! replayed on bind so a late-binding overlay starts from the complete picture.
//! The interface is event-only, so there is no request buffer.

use ::deck_device_info_v1::server::deck_device_info_v1::{
    self, DeckDeviceInfoV1, DeviceState, SetupState,
};
use bmc::compositor::{AccessPointInfo, SetupProgress};
use bmc::manager::BmcState;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::state::CompositorState;

fn device_state_wire(state: BmcState) -> DeviceState {
    match state {
        BmcState::FactoryDefault => DeviceState::FactoryDefault,
        BmcState::SetupPending => DeviceState::SetupPending,
        BmcState::Operational => DeviceState::Operational,
        BmcState::WifiReconfiguration => DeviceState::WifiReconfiguration,
    }
}

/// Map the internal progress to the wire enum and its SSID argument.
fn setup_progress_wire(progress: &SetupProgress) -> (SetupState, String) {
    match progress {
        SetupProgress::Idle => (SetupState::Idle, String::new()),
        SetupProgress::ConnectingToWifi { wifi_ssid } => {
            (SetupState::ConnectingToWifi, wifi_ssid.clone())
        }
        SetupProgress::WifiConnectionSuccess => (SetupState::WifiConnectionSuccess, String::new()),
        SetupProgress::WifiConnectionFailed => (SetupState::WifiConnectionFailed, String::new()),
        SetupProgress::WifiReconfigSuccess => (SetupState::WifiReconfigSuccess, String::new()),
        SetupProgress::DeviceSetupSuccess => (SetupState::DeviceSetupSuccess, String::new()),
        SetupProgress::UnexpectedError { restarting: false } => {
            (SetupState::UnexpectedError, String::new())
        }
        SetupProgress::UnexpectedError { restarting: true } => {
            (SetupState::UnexpectedErrorRestarting, String::new())
        }
    }
}

/// SSID and setup-URL wire arguments; both empty means the AP is down.
fn access_point_wire(ap: Option<&AccessPointInfo>) -> (String, String) {
    ap.map_or_else(
        || (String::new(), String::new()),
        |ap| (ap.ssid.clone(), ap.setup_url.clone()),
    )
}

/// Whether the on-bind replay carries `progress`, or downgrades it to `idle`.
///
/// A replay carries only steps a client cannot reconstruct on its own.
/// A finished setup or reconfiguration is an announcement: replaying it makes
/// a later binder congratulate the user again, long after the fact.
fn replayable(progress: &SetupProgress) -> bool {
    match progress {
        // Nothing else on the wire says the device is stuck.
        SetupProgress::UnexpectedError { .. }
        // Mid-join the lifecycle still reads FactoryDefault, whose screen
        // advertises an access point the join has already taken down.
        | SetupProgress::ConnectingToWifi { .. } => true,
        SetupProgress::Idle
        | SetupProgress::WifiConnectionSuccess
        | SetupProgress::WifiConnectionFailed
        | SetupProgress::WifiReconfigSuccess
        | SetupProgress::DeviceSetupSuccess => false,
    }
}

/// Whether delivering `state` hands out the operational boot sequence.
/// Only an operational boot has connect screens to run; the setup states drive
/// screens that reflect a standing condition, re-derived on every bind.
fn delivers_boot_flow(state: BmcState) -> bool {
    match state {
        BmcState::Operational => true,
        BmcState::FactoryDefault | BmcState::SetupPending | BmcState::WifiReconfiguration => false,
    }
}

/// Tracks bound device-info resources and the values replayed to late binders.
#[derive(Debug)]
pub struct DeviceInfoState {
    pub resources: Vec<DeckDeviceInfoV1>,
    /// `None` until bmc reports the first state; nothing is replayed then,
    /// so an overlay bound before bmc is up keeps waiting
    /// instead of acting on a guessed lifecycle state.
    last_device_state: Option<BmcState>,
    last_setup_progress: SetupProgress,
    last_access_point: Option<AccessPointInfo>,
    /// Latches once an `Operational` state reaches a client, and then rides
    /// every later `device_state` event: a client binding after the boot screens
    /// have run must not restart them. Riding every event, not only the bind,
    /// is what keeps it immune to the client-side latest-wins slots.
    boot_flow_delivered: bool,
}

impl Default for DeviceInfoState {
    fn default() -> Self {
        Self {
            resources: Vec::new(),
            last_device_state: None,
            last_setup_progress: SetupProgress::Idle,
            last_access_point: None,
            boot_flow_delivered: false,
        }
    }
}

impl DeviceInfoState {
    /// Drop dead resources. The disconnect backstop: a client that vanishes
    /// without a `destroy` is reaped here on the next emit.
    fn prune(&mut self) {
        self.resources.retain(Resource::is_alive);
    }

    /// Remove a session by resource identity, on its `destroy` request.
    fn remove(&mut self, resource: &DeckDeviceInfoV1) {
        self.resources.retain(|r| r != resource);
    }

    pub fn set_device_state(&mut self, state: BmcState) {
        self.prune();
        self.last_device_state = Some(state);
        let delivered = self.boot_flow_delivered;
        for r in &self.resources {
            r.device_state(device_state_wire(state), u32::from(delivered));
        }
        self.mark_boot_flow_delivered(state);
    }

    /// Latch the flag once a state carrying the boot sequence has actually
    /// reached someone. Gated on a live resource, so a broadcast into the void
    /// — bmc up before the overlay host — does not burn the one boot sequence.
    fn mark_boot_flow_delivered(&mut self, state: BmcState) {
        if delivers_boot_flow(state) && !self.resources.is_empty() {
            self.boot_flow_delivered = true;
        }
    }

    pub fn set_setup_progress(&mut self, progress: SetupProgress) {
        self.prune();
        let (state, ssid) = setup_progress_wire(&progress);
        self.last_setup_progress = progress;
        for r in &self.resources {
            r.setup_progress(state, ssid.clone());
        }
    }

    pub fn set_access_point(&mut self, ap: Option<AccessPointInfo>) {
        self.prune();
        let (ssid, url) = access_point_wire(ap.as_ref());
        self.last_access_point = ap;
        for r in &self.resources {
            r.access_point(ssid.clone(), url.clone());
        }
    }

    /// Replay the cached values to a freshly bound resource so a late binder
    /// starts from the complete picture instead of waiting for the next
    /// change. Announcement steps are downgraded to `idle` (see `replayable`),
    /// and the lifecycle state carries whether a boot sequence already ran.
    fn replay(&mut self, resource: &DeckDeviceInfoV1) {
        if let Some(state) = self.last_device_state {
            resource.device_state(
                device_state_wire(state),
                u32::from(self.boot_flow_delivered),
            );
            // Delivery is certain here: the resource just written to is this bind's own,
            // not one the broadcast path merely hopes is listening.
            if delivers_boot_flow(state) {
                self.boot_flow_delivered = true;
            }
        }
        let progress = if replayable(&self.last_setup_progress) {
            self.last_setup_progress.clone()
        } else {
            SetupProgress::Idle
        };
        let (state, ssid) = setup_progress_wire(&progress);
        resource.setup_progress(state, ssid);
        let (ssid, url) = access_point_wire(self.last_access_point.as_ref());
        resource.access_point(ssid, url);
    }
}

impl GlobalDispatch<DeckDeviceInfoV1, ()> for CompositorState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<DeckDeviceInfoV1>,
        (): &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let resource = data_init.init(resource, ());
        state.device_info.replay(&resource);
        state.device_info.resources.push(resource);
    }
}

impl Dispatch<DeckDeviceInfoV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &DeckDeviceInfoV1,
        request: deck_device_info_v1::Request,
        (): &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            deck_device_info_v1::Request::Destroy => state.device_info.remove(resource),
            other => tracing::warn!("Unknown deck_device_info_v1 request: {other:?}"),
        }
    }
}

/// Advertise the `deck_device_info_v1` global.
pub fn create_global(display: &DisplayHandle) {
    display.create_global::<CompositorState, DeckDeviceInfoV1, ()>(1, ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_retain_replay_payloads() {
        // The wire fan-out needs live resources and is covered on-device;
        // the caches backing `replay` are the testable half.
        let mut s = DeviceInfoState::default();
        s.set_device_state(BmcState::FactoryDefault);
        s.set_setup_progress(SetupProgress::ConnectingToWifi {
            wifi_ssid: "HomeNet".to_owned(),
        });
        s.set_access_point(Some(AccessPointInfo {
            ssid: "Deck setup".to_owned(),
            setup_url: "http://192.168.8.1/".to_owned(),
        }));

        assert_eq!(s.last_device_state, Some(BmcState::FactoryDefault));
        assert_eq!(
            s.last_setup_progress,
            SetupProgress::ConnectingToWifi {
                wifi_ssid: "HomeNet".to_owned()
            }
        );
        assert_eq!(
            s.last_access_point.as_ref().map(|ap| ap.ssid.as_str()),
            Some("Deck setup")
        );
    }

    #[test]
    fn access_point_down_clears_the_cache() {
        let mut s = DeviceInfoState::default();
        s.set_access_point(Some(AccessPointInfo {
            ssid: "Deck setup".to_owned(),
            setup_url: "http://192.168.8.1/".to_owned(),
        }));
        s.set_access_point(None);
        assert_eq!(s.last_access_point, None);
    }

    #[test]
    fn setup_progress_wire_carries_ssid_only_while_connecting() {
        let (state, ssid) = setup_progress_wire(&SetupProgress::ConnectingToWifi {
            wifi_ssid: "HomeNet".to_owned(),
        });
        assert_eq!(
            (state, ssid.as_str()),
            (SetupState::ConnectingToWifi, "HomeNet")
        );

        let (state, ssid) = setup_progress_wire(&SetupProgress::WifiConnectionSuccess);
        assert_eq!(
            (state, ssid.as_str()),
            (SetupState::WifiConnectionSuccess, "")
        );
    }

    #[test]
    fn access_point_wire_sends_empty_strings_when_down() {
        assert_eq!(access_point_wire(None), (String::new(), String::new()));
    }

    #[test]
    fn only_unreconstructable_steps_survive_a_replay() {
        assert!(replayable(&SetupProgress::UnexpectedError {
            restarting: false
        }));
        assert!(replayable(&SetupProgress::UnexpectedError {
            restarting: true
        }));
        assert!(replayable(&SetupProgress::ConnectingToWifi {
            wifi_ssid: "HomeNet".to_owned()
        }));
        assert!(!replayable(&SetupProgress::DeviceSetupSuccess));
        assert!(!replayable(&SetupProgress::WifiReconfigSuccess));
        assert!(!replayable(&SetupProgress::WifiConnectionSuccess));
        assert!(!replayable(&SetupProgress::WifiConnectionFailed));
        assert!(!replayable(&SetupProgress::Idle));
    }

    #[test]
    fn each_fatal_keeps_its_own_wire_entry() {
        assert_eq!(
            setup_progress_wire(&SetupProgress::UnexpectedError { restarting: false }).0,
            SetupState::UnexpectedError
        );
        assert_eq!(
            setup_progress_wire(&SetupProgress::UnexpectedError { restarting: true }).0,
            SetupState::UnexpectedErrorRestarting
        );
    }

    #[test]
    fn a_broadcast_with_no_client_does_not_burn_the_boot_flow() {
        // bmc up before the overlay host: the sequence is still owed.
        let mut s = DeviceInfoState::default();
        s.set_device_state(BmcState::Operational);
        assert!(!s.boot_flow_delivered);
    }

    #[test]
    fn only_an_operational_state_delivers_the_boot_flow() {
        assert!(delivers_boot_flow(BmcState::Operational));
        assert!(!delivers_boot_flow(BmcState::FactoryDefault));
        assert!(!delivers_boot_flow(BmcState::SetupPending));
        assert!(!delivers_boot_flow(BmcState::WifiReconfiguration));
    }
}
