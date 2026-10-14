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

//! The upgrade RPCs of a Boser-managed product, through the production routes.

use super::boser_managed::{upgrade_client, upgrade_router, with_session};
use crate::compositor::{UpgradeGeneration, UpgradePhase, UpgradeRunSnapshot, UpgradeRunStatus};
use crate::system_upgrade::boser::UpgradeRoute;
use crate::system_upgrade::boser::client::BoserUpgrade;
use crate::system_upgrade::boser::client::tests::{MockBoser, Recorded, hanging_up};
use crate::system_upgrade::{RunStatusService, UnseenOutcome};
use axum::http::StatusCode;
use bmc_grpc::web::{
    CheckForUpgradeRequest, FirmwareUpgradePhase, PackageUpgradePhase, StartUpgradeRequest,
    UpgradeProgress, UpgradeProgressView, upgrade_progress,
};
use bmc_platform::Product;
use bmc_upgrade_types::{Disruption, ExecutionId, InstallablePackage, UpgradeKind, wire};
use buffa_types::google::protobuf::Empty;
use connectrpc::client::{ClientTransport, ServerStream, ServiceTransport};
use connectrpc::{ConnectError, ErrorCode};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

/// `StubSession::id()` in `boser_managed.rs`.
const CALLER: &str = "Bearer test-session";
const NEXT_ITEM_WITHIN: Duration = Duration::from_secs(10);

type ProgressStream = ServerStream<
    <ServiceTransport<axum::Router> as ClientTransport>::ResponseBody,
    UpgradeProgressView<'static>,
>;

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("BUG: wire types serialize")
}

fn message(error: &ConnectError) -> &str {
    error.message.as_deref().unwrap_or_default()
}

async fn routes(product: Product, boser: Option<SocketAddr>) -> (tempfile::TempDir, axum::Router) {
    let (tempdir, router) = upgrade_router(product, boser, None).await;
    (tempdir, with_session(router))
}

/// Managed routes that follow a run on `display`, which the test publishes by hand.
async fn routes_following(
    boser: SocketAddr,
    display: RunStatusService,
) -> (tempfile::TempDir, axum::Router) {
    let route = UpgradeRoute::Boser(BoserUpgrade::new(boser, display));
    let (tempdir, router) = upgrade_router(Product::Bfm100, None, Some(route)).await;
    (tempdir, with_session(router))
}

fn displayed(offer: ExecutionId, state: UpgradeRunStatus) -> UpgradeRunSnapshot {
    UpgradeRunSnapshot {
        generation: UpgradeGeneration::new(0),
        id: Some(offer),
        state,
    }
}

fn running(kind: UpgradeKind, phase: UpgradePhase) -> UpgradeRunStatus {
    UpgradeRunStatus::Running {
        kind,
        phase: Some(phase),
        progress: None,
    }
}

async fn start(routes: axum::Router, upgrade_id: String) -> ProgressStream {
    upgrade_client(&routes)
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: a start reports its failures on the stream")
}

async fn only_item(mut stream: ProgressStream) -> ConnectError {
    let error = next(&mut stream)
        .await
        .expect_err("BUG: the stream's only item is the failure");
    assert_eq!(
        next(&mut stream).await.map_err(|replayed| replayed.code),
        Err(error.code),
        "the stream ends at its failure, which a further read replays"
    );
    error
}

/// A stream that stops making progress fails the test rather than hanging it.
async fn next(stream: &mut ProgressStream) -> Result<Option<UpgradeProgress>, ConnectError> {
    tokio::time::timeout(NEXT_ITEM_WITHIN, stream.message())
        .await
        .expect("BUG: the stream yields its next item or ends")
        .map(|item| item.map(|message| message.to_owned_message()))
}

async fn event(stream: &mut ProgressStream) -> Option<upgrade_progress::Event> {
    next(stream)
        .await
        .expect("BUG: the run is followed without an error")
        .and_then(|progress| progress.event)
}

/// A managed product must never run the local upgrade path, even when Boser cannot be reached.
#[tokio::test]
async fn a_managed_product_without_a_boser_address_answers_unavailable() {
    let (_tempdir, routes) = routes(Product::Bfm100, None).await;
    let upgrade = upgrade_client(&routes);

    let check = upgrade
        .check_for_upgrade(CheckForUpgradeRequest::default())
        .await
        .expect_err("BUG: there is no Boser to ask");
    let installable = upgrade
        .get_installable_widgets(Empty::default())
        .await
        .expect_err("BUG: there is no Boser to ask");
    let started = only_item(start(routes, ExecutionId::new().to_string()).await).await;

    assert_eq!(check.code, ErrorCode::Unavailable);
    assert_eq!(installable.code, ErrorCode::Unavailable);
    assert_eq!(started.code, ErrorCode::Unavailable);
}

/// Boser's local token cannot POST, so the check has to go out as the caller.
#[tokio::test]
async fn a_managed_check_reaches_boser_as_the_caller() {
    let boser = MockBoser::default();
    let id = ExecutionId::new();
    let address = boser
        .serve(
            StatusCode::OK,
            json(&wire::CheckUpgradeResponse {
                offer: Some(wire::Offer {
                    id,
                    kind: UpgradeKind::Packages,
                    disruption: Disruption::AppRestart,
                }),
                firmware: None,
                packages: None,
                package_capability: wire::PackageCapability::Ready,
            }),
        )
        .await;
    let (_tempdir, routes) = routes(Product::Bfm100, Some(address)).await;

    let response = upgrade_client(&routes)
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec!["widget-clock".to_owned()],
        })
        .await
        .expect("BUG: the mock answers the check")
        .into_owned();

    assert_eq!(response.upgrade_id, Some(id.to_string()));
    assert_eq!(
        boser.requests(),
        vec![Recorded {
            method: "POST".to_owned(),
            path: "/api/v1/upgrade/check".to_owned(),
            authorization: Some(CALLER.to_owned()),
            body: r#"{"packages":["widget-clock"]}"#.to_owned(),
        }]
    );
}

/// The widget picker lists what Boser can install, shaped for this product like on Deck.
#[tokio::test]
async fn managed_installable_widgets_come_from_boser() {
    let boser = MockBoser::default();
    let address = boser
        .serve(
            StatusCode::OK,
            json(&wire::InstallablePackages {
                packages: vec![InstallablePackage {
                    name: "widget-clock".to_owned(),
                    version: "1.2.0".to_owned(),
                    category: Some("widget".to_owned()),
                    description: None,
                    metadata: BTreeMap::from([(
                        "widget".to_owned(),
                        serde_json::json!({ "uid": "clock" }),
                    )]),
                }],
            }),
        )
        .await;
    let (_tempdir, routes) = routes(Product::Bfm100, Some(address)).await;

    let widgets = upgrade_client(&routes)
        .get_installable_widgets(Empty::default())
        .await
        .expect("BUG: the mock answers the listing")
        .into_owned()
        .widgets;

    assert_eq!(widgets.len(), 1);
    assert_eq!(widgets[0].uid, "clock");
    assert_eq!(widgets[0].package_name, "widget-clock");
    assert_eq!(boser.requests()[0].authorization, Some(CALLER.to_owned()));
}

/// The whole managed install: Boser admits the start as the caller,
/// and what the display shows of that run becomes the progress the frontend draws.
#[tokio::test]
async fn a_managed_package_start_runs_to_finished() {
    let boser = MockBoser::default();
    let address = boser.serve(StatusCode::NO_CONTENT, "").await;
    let display = RunStatusService::new();
    let (_tempdir, routes) = routes_following(address, display.clone()).await;
    let offer = ExecutionId::new();

    let mut stream = start(routes, offer.to_string()).await;
    assert_eq!(
        boser.requests(),
        vec![Recorded {
            method: "POST".to_owned(),
            path: "/api/v1/upgrade/start".to_owned(),
            authorization: Some(CALLER.to_owned()),
            body: format!(r#"{{"offer_id":"{offer}"}}"#),
        }]
    );

    display.publish(displayed(
        offer,
        running(UpgradeKind::Packages, UpgradePhase::PackageRealizing),
    ));
    assert_eq!(
        event(&mut stream).await,
        Some(upgrade_progress::Event::PackagePhase(
            PackageUpgradePhase::Realizing.into()
        ))
    );

    display.publish(displayed(
        offer,
        UpgradeRunStatus::Succeeded {
            kind: UpgradeKind::Packages,
        },
    ));
    assert_eq!(
        event(&mut stream).await,
        Some(upgrade_progress::Event::Finished(Empty::default()))
    );
    assert_eq!(event(&mut stream).await, None);
}

/// A short run can end before Boser answers the start, and before it was ever seen running:
/// the subscription has to precede the request or the outcome is never seen.
#[tokio::test]
async fn a_run_that_ends_before_boser_answers_is_not_lost() {
    let display = RunStatusService::new();
    let offer = ExecutionId::new();
    let boser = MockBoser::default();
    let published = display.clone();
    let address = boser
        .serve_with(StatusCode::NO_CONTENT, "", move || {
            published.publish_unseen_outcome(UnseenOutcome {
                id: offer,
                result: Ok(()),
            });
        })
        .await;
    let (_tempdir, routes) = routes_following(address, display).await;

    let mut stream = start(routes, offer.to_string()).await;

    assert_eq!(
        event(&mut stream).await,
        Some(upgrade_progress::Event::Finished(Empty::default()))
    );
    assert_eq!(event(&mut stream).await, None);
}

/// A start Boser took and never answered may still have been admitted:
/// the run is followed instead of being reported as a failure.
#[tokio::test]
async fn an_unanswered_start_is_followed() {
    let display = RunStatusService::new();
    let offer = ExecutionId::new();
    let (address, mut requests) = hanging_up().await;
    let (_tempdir, routes) = routes_following(address, display.clone()).await;

    let mut stream = start(routes, offer.to_string()).await;
    tokio::time::timeout(NEXT_ITEM_WITHIN, requests.recv())
        .await
        .expect("BUG: the start reaches the server within the deadline")
        .expect("BUG: the mock server outlives the test");
    display.publish(displayed(
        offer,
        running(UpgradeKind::Packages, UpgradePhase::PackageRealizing),
    ));

    assert_eq!(
        event(&mut stream).await,
        Some(upgrade_progress::Event::PackagePhase(
            PackageUpgradePhase::Realizing.into()
        )),
        "the run Boser admitted is followed although its answer never came"
    );
}

/// The proto's firmware rule holds on a managed product too: `APPLYING` at the handoff
/// to the reboot, then a clean end.
#[tokio::test]
async fn a_managed_firmware_start_ends_cleanly_at_the_reboot() {
    let boser = MockBoser::default();
    let address = boser.serve(StatusCode::NO_CONTENT, "").await;
    let display = RunStatusService::new();
    let (_tempdir, routes) = routes_following(address, display.clone()).await;
    let offer = ExecutionId::new();

    let mut stream = start(routes, offer.to_string()).await;
    display.publish(displayed(
        offer,
        running(UpgradeKind::Firmware, UpgradePhase::PackageRealizing),
    ));
    assert_eq!(
        event(&mut stream).await,
        Some(upgrade_progress::Event::PackagePhase(
            PackageUpgradePhase::Realizing.into()
        )),
        "the packages staged before the reboot are reported on a firmware run"
    );
    display.publish(displayed(
        offer,
        UpgradeRunStatus::Rebooting {
            kind: UpgradeKind::Firmware,
        },
    ));

    assert_eq!(
        event(&mut stream).await,
        Some(upgrade_progress::Event::FirmwarePhase(
            FirmwareUpgradePhase::Applying.into()
        ))
    );
    assert_eq!(event(&mut stream).await, None);
}

/// The caller is told why Boser's run failed, in the terms the local path uses for its own.
#[tokio::test]
async fn a_managed_run_that_fails_reports_the_reason() {
    let boser = MockBoser::default();
    let address = boser.serve(StatusCode::NO_CONTENT, "").await;
    let display = RunStatusService::new();
    let (_tempdir, routes) = routes_following(address, display.clone()).await;
    let offer = ExecutionId::new();

    let stream = start(routes, offer.to_string()).await;
    display.publish(displayed(
        offer,
        UpgradeRunStatus::Failed {
            kind: UpgradeKind::Packages,
            reason: "build failed".to_owned(),
        },
    ));
    let error = only_item(stream).await;

    assert_eq!(error.code, ErrorCode::Internal);
    assert_eq!(message(&error), "build failed");
}

/// A stale or mangled id is "expired", as on Deck, and Boser is not bothered with it.
#[tokio::test]
async fn an_unparseable_upgrade_id_is_the_streams_only_item() {
    let boser = MockBoser::default();
    let address = boser.serve(StatusCode::NO_CONTENT, "").await;
    let (_tempdir, routes) = routes_following(address, RunStatusService::new()).await;

    let error = only_item(start(routes, "not-an-id".to_owned()).await).await;

    assert_eq!(error.code, ErrorCode::FailedPrecondition);
    assert_eq!(boser.requests(), Vec::new());
}

/// Start failures arrive on the stream, where the frontend already looks for them.
#[tokio::test]
async fn a_start_boser_refuses_is_the_streams_only_item() {
    let boser = MockBoser::default();
    let address = boser
        .serve(
            StatusCode::CONFLICT,
            json(&wire::ErrorBody {
                error: "BUSY".to_owned(),
                message: "an upgrade is already running".to_owned(),
            }),
        )
        .await;
    let (_tempdir, routes) = routes(Product::Bfm100, Some(address)).await;
    let offer = ExecutionId::new();

    let error = only_item(start(routes, offer.to_string()).await).await;

    assert_eq!(error.code, ErrorCode::Unavailable);
    assert_eq!(message(&error), "an upgrade is already running");
    assert_eq!(
        boser.requests(),
        [Recorded {
            method: "POST".to_owned(),
            path: "/api/v1/upgrade/start".to_owned(),
            authorization: Some(CALLER.to_owned()),
            body: json(&wire::StartUpgradeRequest { offer_id: offer }),
        }]
    );
}

/// A Deck with a Boser address configured still upgrades itself: nothing is sent to Boser.
#[tokio::test]
async fn a_self_managed_product_never_calls_boser() {
    let boser = MockBoser::default();
    let address = boser.serve(StatusCode::NO_CONTENT, "").await;
    let (_tempdir, routes) = routes(Product::Bmc100, Some(address)).await;

    let error = only_item(start(routes, ExecutionId::new().to_string()).await).await;

    assert_eq!(
        error.code,
        ErrorCode::FailedPrecondition,
        "the local offer cache does not know the id"
    );
    assert_eq!(boser.requests(), Vec::new());
}
