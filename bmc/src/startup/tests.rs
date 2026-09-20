// Copyright (C) 2025  Braiins Systems s.r.o.
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

use super::{
    BoserObservation, Destination, Publication, SETUP_AP_REFRESHES, SETUP_URL_CLEAR_AFTER,
    SetupPendingWait, boser_observation, current_access_point, forward_upgrade_display_state,
    post_upgrade_kind, runs_setup_ap, setup_pending_wait,
};
use crate::compositor::{
    AccessPointInfo, CompositorError, UpgradeDisplaySnapshot, UpgradeDisplayState,
    UpgradeGeneration, UpgradeKind,
};
use crate::manager::{BmcState, UpgradeMarker};
use bmc_net::NetworkManager;
use bmc_net::mock::MockNetworkManager;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, watch};

const AP_HOST: &str = "10.0.0.21";

/// A factory-default board with its setup AP up, as the mock seeds it.
fn setup_ap_board() -> MockNetworkManager {
    MockNetworkManager::with_provisioning(true, false).with_captive_portal_host(AP_HOST)
}

fn wired(ip: Ipv4Addr) -> AccessPointInfo {
    AccessPointInfo {
        ssid: String::new(),
        setup_url: format!("http://{ip}/"),
    }
}

/// What the mock's live setup AP advertises.
fn mock_ap() -> AccessPointInfo {
    AccessPointInfo {
        ssid: "MockAP".to_owned(),
        setup_url: format!("http://{AP_HOST}/"),
    }
}

#[tokio::test]
async fn a_live_setup_ap_is_advertised_with_its_ssid() {
    let network = setup_ap_board();
    assert_eq!(current_access_point(&network).await, Some(mock_ap()));
}

#[tokio::test]
async fn an_ap_the_platform_took_down_is_not_advertised() {
    let network = setup_ap_board();
    network.provisioning().publish_setup_ap_active(false);
    assert_eq!(
        current_access_point(&network).await,
        None,
        "ap_ssid still answers, but nothing is bound to the AP address"
    );
}

fn setup_ap() -> AccessPointInfo {
    AccessPointInfo {
        ssid: "Deck setup".to_owned(),
        setup_url: format!("http://{AP_HOST}/"),
    }
}

/// A board booting into `SetupPending`, as the mock seeds it.
fn setup_pending_board() -> MockNetworkManager {
    MockNetworkManager::with_provisioning(false, true)
}

#[tokio::test]
async fn a_board_without_a_port_waits_on_its_join_whatever_is_saved() {
    let board = setup_pending_board();
    assert_eq!(
        setup_pending_wait(&board, false).await,
        SetupPendingWait::WifiJoin,
        "the Deck judges the join it has, saved station or not"
    );
}

#[tokio::test]
async fn a_wired_board_with_no_station_saved_waits_on_its_cable() {
    let board = setup_pending_board();
    assert_eq!(
        setup_pending_wait(&board, true).await,
        SetupPendingWait::Cable
    );
}

#[tokio::test]
async fn a_wired_board_with_a_station_saved_waits_on_its_join() {
    let board = setup_pending_board().with_saved_station("HomeNet");
    assert_eq!(
        setup_pending_wait(&board, true).await,
        SetupPendingWait::WifiJoin
    );
}

#[test]
fn the_overlay_hears_a_destination_once_and_every_change() {
    let mut destination = Destination::default();
    assert_eq!(
        destination.observe(Some(setup_ap()), true),
        Publication::Show(setup_ap())
    );
    assert_eq!(
        destination.observe(Some(setup_ap()), true),
        Publication::Keep
    );
    let cable = wired(Ipv4Addr::new(10, 33, 50, 103));
    assert_eq!(
        destination.observe(Some(cable.clone()), true),
        Publication::Show(cable)
    );
}

#[test]
fn a_lost_destination_is_cleared_only_once_it_stays_lost() {
    let mut destination = Destination::default();
    destination.observe(Some(setup_ap()), true);
    for _ in 1..SETUP_URL_CLEAR_AFTER {
        assert_eq!(
            destination.observe(None, true),
            Publication::Keep,
            "a lease renew or an AP restart must not blank the screen"
        );
    }
    assert_eq!(destination.observe(None, true), Publication::Clear);
    assert_eq!(
        destination.observe(None, true),
        Publication::Keep,
        "cleared once, not on every miss"
    );
    assert_eq!(
        destination.observe(Some(setup_ap()), true),
        Publication::Show(setup_ap()),
        "the AP coming back is shown again"
    );
}

#[test]
fn a_board_whose_ap_never_came_up_gives_up_after_the_ap_window() {
    let mut destination = Destination::default();
    for _ in 1..SETUP_AP_REFRESHES {
        assert_eq!(destination.observe(None, true), Publication::Keep);
    }
    assert_eq!(destination.observe(None, true), Publication::GiveUp);
}

#[test]
fn a_board_that_may_not_give_up_keeps_waiting() {
    let mut destination = Destination::default();
    for _ in 0..(SETUP_AP_REFRESHES * 2) {
        assert_eq!(destination.observe(None, false), Publication::Keep);
    }
}

#[test]
fn a_destination_shown_once_never_turns_into_giving_up() {
    let mut destination = Destination::default();
    destination.observe(Some(setup_ap()), true);
    for _ in 0..(SETUP_AP_REFRESHES * 2) {
        assert_ne!(destination.observe(None, true), Publication::GiveUp);
    }
}

#[tokio::test]
async fn a_wired_address_is_advertised_whatever_the_ap_does() {
    let ip = Ipv4Addr::new(10, 33, 50, 103);
    let network = setup_ap_board();
    network.publish_ethernet_ipv4(Some(ip));
    assert_eq!(current_access_point(&network).await, Some(wired(ip)));
    network.provisioning().publish_setup_ap_active(false);
    assert_eq!(current_access_point(&network).await, Some(wired(ip)));
}

#[tokio::test]
async fn a_pulled_cable_hands_the_screen_back_to_the_setup_ap() {
    let ip = Ipv4Addr::new(10, 33, 50, 103);
    let network = setup_ap_board();
    network.publish_ethernet_ipv4(Some(ip));
    assert_eq!(current_access_point(&network).await, Some(wired(ip)));

    // Hotplug takes the AP down while the cable holds the address,
    // and raises it again once the cable is gone.
    network.provisioning().publish_setup_ap_active(false);
    network.publish_ethernet_ipv4(None);
    assert_eq!(
        current_access_point(&network).await,
        None,
        "between the cable going and the AP coming back there is nothing to advertise"
    );

    network.provisioning().publish_setup_ap_active(true);
    assert_eq!(current_access_point(&network).await, Some(mock_ap()));
}

fn snapshot(generation: usize) -> UpgradeDisplaySnapshot {
    UpgradeDisplaySnapshot {
        generation: UpgradeGeneration::new(generation),
        state: UpgradeDisplayState::Succeeded {
            kind: UpgradeKind::Firmware,
        },
    }
}

#[tokio::test]
async fn upgrade_bridge_replays_a_terminal_snapshot_present_before_its_first_poll() {
    let (sender, receiver) = watch::channel(None);
    let second = snapshot(2);
    sender
        .send(Some(second.clone()))
        .expect("BUG: receiver is live");
    drop(sender);
    let received = Arc::new(Mutex::new(Vec::new()));

    forward_upgrade_display_state(receiver, {
        let received = Arc::clone(&received);
        move |state| {
            received
                .lock()
                .expect("BUG: received lock poisoned")
                .push(state);
            Ok(())
        }
    })
    .await;

    assert_eq!(
        *received.lock().expect("BUG: received lock poisoned"),
        vec![Some(second)]
    );
}

#[tokio::test]
async fn upgrade_bridge_coalesces_to_the_latest_authoritative_snapshot() {
    let (sender, receiver) = watch::channel(None);
    sender
        .send(Some(snapshot(1)))
        .expect("BUG: receiver is live");
    let second = snapshot(2);
    sender
        .send(Some(second.clone()))
        .expect("BUG: receiver is live");
    drop(sender);
    let received = Arc::new(Mutex::new(Vec::new()));

    forward_upgrade_display_state(receiver, {
        let received = Arc::clone(&received);
        move |state| {
            received
                .lock()
                .expect("BUG: received lock poisoned")
                .push(state);
            Ok(())
        }
    })
    .await;

    assert_eq!(
        *received.lock().expect("BUG: received lock poisoned"),
        vec![Some(second)]
    );
}

#[tokio::test]
async fn upgrade_bridge_continues_after_a_compositor_error() {
    let (sender, receiver) = watch::channel(None);
    let first = snapshot(1);
    let second = snapshot(2);
    let entered = Arc::new(Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let received = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn(forward_upgrade_display_state(receiver, {
        let entered = Arc::clone(&entered);
        let calls = Arc::clone(&calls);
        let received = Arc::clone(&received);
        move |state| {
            received
                .lock()
                .expect("BUG: received lock poisoned")
                .push(state);
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                entered.notify_one();
                Err(CompositorError::NotStarted)
            } else {
                Ok(())
            }
        }
    }));

    sender.send(Some(first)).expect("BUG: receiver is live");
    entered.notified().await;
    sender
        .send(Some(second.clone()))
        .expect("BUG: receiver is live");
    drop(sender);
    task.await.expect("BUG: bridge task must finish");

    assert_eq!(
        *received.lock().expect("BUG: received lock poisoned"),
        vec![Some(snapshot(1)), Some(second)]
    );
}

#[tokio::test]
async fn upgrade_bridge_relays_a_clear_after_a_snapshot() {
    let (sender, receiver) = watch::channel(None);
    let first = snapshot(1);
    let entered = Arc::new(Notify::new());
    let received = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn(forward_upgrade_display_state(receiver, {
        let entered = Arc::clone(&entered);
        let received = Arc::clone(&received);
        move |state| {
            received
                .lock()
                .expect("BUG: received lock poisoned")
                .push(state);
            entered.notify_one();
            Ok(())
        }
    }));

    entered.notified().await;
    sender.send(Some(first)).expect("BUG: receiver is live");
    entered.notified().await;
    sender.send(None).expect("BUG: receiver is live");
    drop(sender);
    task.await.expect("BUG: bridge task must finish");

    // An execution whose outcome was never observed must leave the display,
    // not linger as a stale in-progress picture.
    assert_eq!(
        *received.lock().expect("BUG: received lock poisoned"),
        vec![None, Some(snapshot(1)), None]
    );
}

#[test]
fn service_marker_alone_reports_a_package_upgrade() {
    assert_eq!(
        post_upgrade_kind(UpgradeMarker::Absent, UpgradeMarker::Consumed),
        Some(UpgradeKind::Packages),
        "a restart without a firmware marker can only come from a package upgrade"
    );
}

#[test]
fn firmware_marker_wins_even_when_removal_fails() {
    assert_eq!(
        post_upgrade_kind(UpgradeMarker::RemovalFailed, UpgradeMarker::Consumed),
        Some(UpgradeKind::Firmware),
        "the marker's existence proves the firmware upgrade; \
         a failed removal must not demote it to packages"
    );
}

#[test]
fn both_setup_states_run_the_access_point() {
    for state in [BmcState::FactoryDefault, BmcState::WifiReconfiguration] {
        assert!(
            runs_setup_ap(state),
            "{state:?} shows the setup screen, which needs an access point to advertise"
        );
    }
}

#[test]
fn the_states_without_a_setup_screen_run_no_access_point() {
    for state in [
        BmcState::SetupPending,
        BmcState::Operational,
        BmcState::Unsupported,
    ] {
        assert!(
            !runs_setup_ap(state),
            "{state:?} joins a network as a station, so no AP is up to publish"
        );
    }
}

#[test]
fn absent_firmware_and_unconsumed_service_marker_report_no_upgrade() {
    for service in [UpgradeMarker::Absent, UpgradeMarker::RemovalFailed] {
        assert_eq!(post_upgrade_kind(UpgradeMarker::Absent, service), None);
    }
}

#[test]
fn only_a_boser_managed_product_observes_boser() {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], 8080));

    assert_eq!(
        boser_observation(true, Some(address)),
        BoserObservation::Observe(address)
    );
    // On a Deck the observer would compete with the device's local upgrade state.
    assert_eq!(
        boser_observation(false, Some(address)),
        BoserObservation::SelfManaged
    );
    assert_eq!(
        boser_observation(true, None),
        BoserObservation::AddressMissing
    );
}
