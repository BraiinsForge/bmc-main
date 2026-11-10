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

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bmc_grpc::web::{
    EncryptionType, NetworkService as _, SetHostnameRequest, SetWifiRequest, SignalStrength,
};
use bmc_net::mock::MockNetworkManager;
use bmc_net_types::wifi::{
    EncryptionType as WifiEncryption, WifiConfiguration, WifiLinkState, WifiMode, WifiStatus,
};
use buffa::EnumValue;
use buffa_types::google::protobuf::Empty;
use connectrpc::{ErrorCode, ServiceResult};
use futures::FutureExt as _;
use strum::IntoEnumIterator as _;
use tokio::sync::{Notify, watch};

use super::boser_managed::StubBmcManager;
use super::{no_session, wire_request};
use crate::BmcManager as _;
use crate::initial_setup::{InitSetupState, StateService, Uplink, WifiJoins};
use crate::web::grpc::network::{NetworkService, into_grpc_wifi_status, try_into_encryption_type};

/// How long a test waits for a step on the feed; only a step that never comes reaches it.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

fn service(network: MockNetworkManager, progress: &StateService) -> NetworkService<StubBmcManager> {
    let manager = Arc::new(StubBmcManager::with_network(network));
    let wifi_joins = WifiJoins::new(
        manager.clone(),
        Arc::new(AtomicBool::new(false)),
        progress.clone(),
    );
    NetworkService::new(manager, wifi_joins)
}

async fn join_home_net(service: &NetworkService<StubBmcManager>) -> ServiceResult<Empty> {
    wire_request!(
        request,
        SetWifiRequest {
            ssid: "HomeNet".to_owned(),
            password: Some("secret".to_owned()),
            encryption_type: EncryptionType::Wpa2.into(),
        }
    );
    service.set_wifi(no_session(), request).await
}

async fn wait_for_step(feed: &mut watch::Receiver<Option<InitSetupState>>, step: &InitSetupState) {
    tokio::time::timeout(
        STEP_TIMEOUT,
        feed.wait_for(|seen| seen.as_ref() == Some(step)),
    )
    .await
    .unwrap_or_else(|_| panic!("the feed never reported {step:?}"))
    .expect("BUG: the feed's sender lives in the test");
}

#[tokio::test]
async fn a_web_ui_join_names_the_network_before_joining() {
    let gate = Arc::new(Notify::new());
    let progress = StateService::new();
    let mut feed = progress.subscribe();
    let service = service(
        MockNetworkManager::default().with_join_gate(gate.clone()),
        &progress,
    );
    let call = tokio::spawn(async move { join_home_net(&service).await });

    wait_for_step(
        &mut feed,
        &InitSetupState::SwitchingUplink {
            uplink: Uplink::Wifi {
                ssid: "HomeNet".to_owned(),
            },
        },
    )
    .await;
    gate.notify_one();

    assert!(
        call.await.expect("BUG: the call does not panic").is_ok(),
        "the join succeeds"
    );
    assert_eq!(*feed.borrow(), Some(InitSetupState::WifiConnectionSuccess));
}

#[tokio::test]
async fn a_failed_web_ui_join_reports_the_failure_and_errors() {
    let progress = StateService::new();
    let feed = progress.subscribe();
    let service = service(MockNetworkManager::default().with_failing_join(), &progress);

    let error = join_home_net(&service)
        .await
        .expect_err("the mock join fails");

    assert_eq!(error.code, ErrorCode::Internal);
    assert_eq!(*feed.borrow(), Some(InitSetupState::WifiConnectionFailed));
}

#[tokio::test]
async fn a_join_outlives_a_client_that_hangs_up() {
    let gate = Arc::new(Notify::new());
    let progress = StateService::new();
    let mut feed = progress.subscribe();
    let service = service(
        MockNetworkManager::default().with_join_gate(gate.clone()),
        &progress,
    );

    assert!(
        join_home_net(&service).now_or_never().is_none(),
        "the join waits on the gate, so the call is dropped mid-join"
    );
    gate.notify_one();

    wait_for_step(&mut feed, &InitSetupState::WifiConnectionSuccess).await;
}

#[tokio::test]
async fn a_second_join_is_refused_until_the_first_ends() {
    let gate = Arc::new(Notify::new());
    let progress = StateService::new();
    let mut feed = progress.subscribe();
    let service = Arc::new(service(
        MockNetworkManager::default().with_join_gate(gate.clone()),
        &progress,
    ));
    let first = tokio::spawn({
        let service = service.clone();
        async move { join_home_net(&service).await }
    });
    wait_for_step(
        &mut feed,
        &InitSetupState::SwitchingUplink {
            uplink: Uplink::Wifi {
                ssid: "HomeNet".to_owned(),
            },
        },
    )
    .await;

    let error = join_home_net(&service)
        .now_or_never()
        .expect("a refused join answers without waiting on the radio")
        .expect_err("the first join still holds the guard");
    assert_eq!(error.code, ErrorCode::FailedPrecondition);

    gate.notify_one();
    assert!(
        first.await.expect("BUG: the call does not panic").is_ok(),
        "the refused join leaves the first one alone"
    );
    gate.notify_one();
    assert!(
        join_home_net(&service).await.is_ok(),
        "the first join released the guard"
    );
}

#[tokio::test]
async fn a_failed_join_releases_the_guard() {
    let progress = StateService::new();
    let service = service(MockNetworkManager::default().with_failing_join(), &progress);

    let _ = join_home_net(&service).await;

    let error = join_home_net(&service)
        .await
        .expect_err("the mock join fails");
    assert_eq!(
        error.code,
        ErrorCode::Internal,
        "the second join ran instead of being refused"
    );
}

#[test]
fn a_saved_network_reports_the_encryption_type_a_join_sends_back() {
    for saved in WifiEncryption::iter() {
        let status = into_grpc_wifi_status(WifiStatus {
            enabled: true,
            configuration: Some(WifiConfiguration {
                mode: WifiMode::Station,
                ssid: "HomeNet".to_owned(),
                encryption_type: saved,
            }),
            sta_link_state: None,
        });
        let reported = status
            .network
            .into_option()
            .expect("BUG: a saved network is reported with its network")
            .encryption_type;
        assert_eq!(
            try_into_encryption_type(reported).ok(),
            Some(saved),
            "{saved} must come back as itself"
        );
    }
}

#[test]
fn a_status_without_a_configuration_reports_no_network() {
    let status = into_grpc_wifi_status(WifiStatus {
        enabled: false,
        configuration: None,
        sta_link_state: None,
    });
    assert!(
        status.network.into_option().is_none(),
        "an unconfigured status must not invent an open network"
    );
}

#[test]
fn a_saved_network_reports_its_signal_as_a_strength_not_a_level() {
    let reported_with = |sta_link_state| {
        into_grpc_wifi_status(WifiStatus {
            enabled: true,
            configuration: Some(WifiConfiguration {
                mode: WifiMode::Station,
                ssid: "HomeNet".to_owned(),
                encryption_type: WifiEncryption::Wpa2,
            }),
            sta_link_state,
        })
        .network
        .into_option()
        .expect("BUG: a saved network is reported with its network")
        .signal_strength
    };

    assert_eq!(
        reported_with(None),
        EnumValue::Known(SignalStrength::Unspecified),
        "a saved network without a link has no strength to report"
    );
    for (level_dbm, strength) in [
        (-34, SignalStrength::Strong),
        (-67, SignalStrength::Moderate),
        (-90, SignalStrength::Weak),
    ] {
        assert_eq!(
            reported_with(Some(WifiLinkState::new("HomeNet", level_dbm))),
            EnumValue::Known(strength),
            "{level_dbm} dBm is reported as {strength:?}, not as the level"
        );
    }
}

/// A provisioned device over `network`, with the manager kept so a test can
/// read back what the call left in the network configuration.
fn provisioned(
    network: MockNetworkManager,
) -> (NetworkService<StubBmcManager>, Arc<StubBmcManager>) {
    let manager = Arc::new(StubBmcManager::with_network(network));
    let wifi_joins = WifiJoins::new(
        manager.clone(),
        Arc::new(AtomicBool::new(false)),
        StateService::new(),
    );
    (NetworkService::new(manager.clone(), wifi_joins), manager)
}

async fn rename(service: &NetworkService<StubBmcManager>, hostname: &str) -> ServiceResult<Empty> {
    wire_request!(
        request,
        SetHostnameRequest {
            hostname: hostname.to_owned(),
        }
    );
    service.set_hostname(no_session(), request).await
}

#[tokio::test]
async fn a_rename_reaches_the_network_configuration() {
    let (service, manager) = provisioned(MockNetworkManager::with_provisioning(false, false));

    rename(&service, "kitchen-deck")
        .await
        .expect("a valid name is accepted");

    assert_eq!(
        manager.network_manager().hostname().await.as_deref(),
        Some("kitchen-deck"),
        "the name the device answers to on the network is the one the user typed"
    );
}

#[tokio::test]
async fn a_name_the_network_cannot_carry_is_refused_and_the_old_one_stays() {
    let (service, manager) = provisioned(MockNetworkManager::with_provisioning(false, false));

    let error = rename(&service, "kitchen deck")
        .await
        .expect_err("a name with a space is not a hostname");

    assert_eq!(error.code, ErrorCode::InvalidArgument);
    assert_eq!(
        manager.network_manager().hostname().await.as_deref(),
        Some("mock"),
        "a refused rename must not leave the device unreachable under either name"
    );
}

#[tokio::test]
async fn a_rename_is_refused_while_the_device_is_still_being_set_up() {
    let (service, manager) = provisioned(MockNetworkManager::with_provisioning(true, false));

    let error = rename(&service, "kitchen-deck")
        .await
        .expect_err("the setup flow owns the network until it finishes");

    assert_eq!(error.code, ErrorCode::FailedPrecondition);
    assert_eq!(
        manager.network_manager().hostname().await.as_deref(),
        Some("mock")
    );
}
