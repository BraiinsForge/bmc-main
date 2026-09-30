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

use bmc_grpc::web::{EncryptionType, SetWifiRequest, network_service_server::NetworkService as _};
use bmc_net::mock::MockNetworkManager;
use futures::FutureExt as _;
use tokio::sync::{Notify, watch};
use tonic::{Code, Request};

use super::boser_managed::StubBmcManager;
use crate::initial_setup::{InitSetupState, StateService, Uplink, WifiJoins};
use crate::web::grpc::network::NetworkService;

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

fn join_home_net() -> Request<SetWifiRequest> {
    Request::new(SetWifiRequest {
        ssid: "HomeNet".to_owned(),
        password: Some("secret".to_owned()),
        encryption_type: EncryptionType::Wpa2.into(),
    })
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
    let call = tokio::spawn(async move { service.set_wifi(join_home_net()).await });

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

    let status = service
        .set_wifi(join_home_net())
        .await
        .expect_err("the mock join fails");

    assert_eq!(status.code(), Code::Internal);
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
        service.set_wifi(join_home_net()).now_or_never().is_none(),
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
        async move { service.set_wifi(join_home_net()).await }
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

    let status = service
        .set_wifi(join_home_net())
        .now_or_never()
        .expect("a refused join answers without waiting on the radio")
        .expect_err("the first join still holds the guard");
    assert_eq!(status.code(), Code::FailedPrecondition);

    gate.notify_one();
    assert!(
        first.await.expect("BUG: the call does not panic").is_ok(),
        "the refused join leaves the first one alone"
    );
    gate.notify_one();
    assert!(
        service.set_wifi(join_home_net()).await.is_ok(),
        "the first join released the guard"
    );
}

#[tokio::test]
async fn a_failed_join_releases_the_guard() {
    let progress = StateService::new();
    let service = service(MockNetworkManager::default().with_failing_join(), &progress);

    let _ = service.set_wifi(join_home_net()).await;

    let status = service
        .set_wifi(join_home_net())
        .await
        .expect_err("the mock join fails");
    assert_eq!(
        status.code(),
        Code::Internal,
        "the second join ran instead of being refused"
    );
}
