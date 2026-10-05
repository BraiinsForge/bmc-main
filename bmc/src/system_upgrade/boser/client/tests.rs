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

use super::{BoserUpgrade, ClientError, Rejection};
use crate::compositor::{UpgradeGeneration, UpgradePhase, UpgradeRunSnapshot, UpgradeRunStatus};
use crate::system_upgrade::RunStatusService;
use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use bmc_upgrade_types::{Disruption, ExecutionId, InstallablePackage, UpgradeKind, wire};
use std::collections::BTreeMap;
use std::future::IntoFuture;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt as _;

const SESSION: &str = "0123456789abcdef";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) authorization: Option<String>,
    pub(crate) body: String,
}

/// A Boser that gives every request the same answer and remembers what it was asked.
#[derive(Clone, Default)]
pub(crate) struct MockBoser {
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl MockBoser {
    pub(crate) async fn serve(&self, status: StatusCode, body: impl Into<String>) -> SocketAddr {
        self.serve_with(status, body, || {}).await
    }

    /// Like `serve`, running `before_answer` once the request is recorded and before it is answered.
    pub(crate) async fn serve_with(
        &self,
        status: StatusCode,
        body: impl Into<String>,
        before_answer: impl Fn() + Send + Sync + 'static,
    ) -> SocketAddr {
        let mock = self.clone();
        let body = body.into();
        let before_answer = Arc::new(before_answer);
        let router = Router::new().fallback(move |request: Request| {
            let mock = mock.clone();
            let body = body.clone();
            let before_answer = Arc::clone(&before_answer);
            async move {
                let (parts, request_body) = request.into_parts();
                let bytes = axum::body::to_bytes(request_body, usize::MAX)
                    .await
                    .expect("BUG: a loopback request body reads");
                mock.requests
                    .lock()
                    .expect("BUG: no test panics while holding the request log")
                    .push(Recorded {
                        method: parts.method.to_string(),
                        path: parts.uri.path().to_owned(),
                        authorization: parts.headers.get(header::AUTHORIZATION).map(|value| {
                            value
                                .to_str()
                                .expect("BUG: the client sends an ASCII credential")
                                .to_owned()
                        }),
                        body: String::from_utf8(bytes.to_vec())
                            .expect("BUG: the client sends UTF-8 bodies"),
                    });
                before_answer();
                (status, body)
            }
        });
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("BUG: a loopback listener binds");
        let address = listener
            .local_addr()
            .expect("BUG: a bound listener has an address");
        tokio::spawn(axum::serve(listener, router).into_future());
        address
    }

    pub(crate) fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .expect("BUG: no test panics while holding the request log")
            .clone()
    }
}

/// An address nothing listens on.
async fn dead_address() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("BUG: a loopback listener binds");
    listener
        .local_addr()
        .expect("BUG: a bound listener has an address")
}

/// A server that takes each request and hangs up without answering it.
pub(crate) async fn hanging_up() -> (SocketAddr, tokio::sync::mpsc::UnboundedReceiver<()>) {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("BUG: a loopback listener binds");
    let address = listener
        .local_addr()
        .expect("BUG: a bound listener has an address");
    let (received, requests) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((mut connection, _peer)) = listener.accept().await {
            let mut request = [0; 1024];
            if matches!(connection.read(&mut request).await, Ok(read) if read > 0) {
                let _receiver_gone = received.send(());
            }
        }
    });
    (address, requests)
}

fn client(address: SocketAddr) -> BoserUpgrade {
    BoserUpgrade::new(address, RunStatusService::new())
}

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("BUG: wire types serialize")
}

fn offer_response(id: ExecutionId) -> wire::CheckUpgradeResponse {
    wire::CheckUpgradeResponse {
        offer: Some(wire::Offer {
            id,
            kind: UpgradeKind::Packages,
            disruption: Disruption::AppRestart,
        }),
        firmware: None,
        packages: None,
        package_capability: wire::PackageCapability::Ready,
    }
}

fn error_body(code: &str) -> String {
    json(&wire::ErrorBody {
        error: code.to_owned(),
        message: format!("{code} happened"),
    })
}

/// Boser's local token authenticates only GET; a check must be the user's own request.
#[tokio::test]
async fn check_posts_the_packages_as_the_caller() {
    let boser = MockBoser::default();
    let response = offer_response(ExecutionId::new());
    let address = boser.serve(StatusCode::OK, json(&response)).await;

    let answer = client(address)
        .check(SESSION, vec!["widget-a".to_owned()])
        .await;

    assert_eq!(answer, Ok(response));
    assert_eq!(
        boser.requests(),
        vec![Recorded {
            method: "POST".to_owned(),
            path: "/api/v1/upgrade/check".to_owned(),
            authorization: Some(format!("Bearer {SESSION}")),
            body: r#"{"packages":["widget-a"]}"#.to_owned(),
        }]
    );
}

/// The installable list is session-authenticated like the rest, although it is a GET.
#[tokio::test]
async fn installable_returns_the_packages_boser_lists() {
    let boser = MockBoser::default();
    let package = InstallablePackage {
        name: "widget-clock".to_owned(),
        version: "1.2.0".to_owned(),
        category: Some("widget".to_owned()),
        description: None,
        metadata: BTreeMap::new(),
    };
    let body = json(&wire::InstallablePackages {
        packages: vec![package.clone()],
    });
    let address = boser.serve(StatusCode::OK, body).await;

    let answer = client(address).installable(SESSION).await;

    assert_eq!(answer, Ok(vec![package]));
    let requests = boser.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/api/v1/upgrade/packages/installable");
    assert_eq!(requests[0].authorization, Some(format!("Bearer {SESSION}")));
}

/// A start must be attributable to the logged-in user and name the offer the check returned.
#[tokio::test]
async fn start_posts_the_offer_as_the_caller() {
    let boser = MockBoser::default();
    let address = boser.serve(StatusCode::NO_CONTENT, "").await;
    let offer = ExecutionId::new();

    let answer = client(address).start(SESSION, offer).await;

    assert_eq!(answer, Ok(()));
    assert_eq!(
        boser.requests(),
        vec![Recorded {
            method: "POST".to_owned(),
            path: "/api/v1/upgrade/start".to_owned(),
            authorization: Some(format!("Bearer {SESSION}")),
            body: format!(r#"{{"offer_id":"{offer}"}}"#),
        }]
    );
}

/// An unreachable Boser is a retryable condition, not a server fault of the BMC application.
#[tokio::test]
async fn a_refused_connection_is_unreachable() {
    let address = dead_address().await;

    let answer = client(address).check(SESSION, Vec::new()).await;

    assert_eq!(answer, Err(ClientError::Unreachable));
}

/// A start that never connected reached nobody: saying it may have started would be a false alarm.
#[tokio::test]
async fn a_start_that_never_connected_did_not_start() {
    let address = dead_address().await;

    let answer = client(address).start(SESSION, ExecutionId::new()).await;

    assert_eq!(answer, Err(ClientError::Unreachable));
}

/// A request that could not even be built was never sent either.
#[tokio::test]
async fn a_start_that_was_never_sent_did_not_start() {
    let (address, mut requests) = hanging_up().await;

    let answer = client(address)
        .start("not\na header value", ExecutionId::new())
        .await;

    assert_eq!(answer, Err(ClientError::Unreachable));
    assert!(requests.try_recv().is_err(), "nothing reached the server");
}

/// A start whose answer never came may have been admitted, so it is set apart from a definite failure.
#[tokio::test]
async fn an_unanswered_start_may_have_started() {
    let (address, mut requests) = hanging_up().await;

    let error = client(address)
        .start(SESSION, ExecutionId::new())
        .await
        .expect_err("BUG: the server hangs up without an answer");

    assert!(
        requests.try_recv().is_ok(),
        "the start reached the server, so it may have been admitted"
    );

    assert_eq!(error, ClientError::StartUnconfirmed);
    assert!(error.to_string().contains("may have started"), "{error}");
}

/// Only a missing answer leaves the start in doubt; what Boser refused did not start.
#[tokio::test]
async fn a_refused_start_is_not_unconfirmed() {
    let boser = MockBoser::default();
    let address = boser.serve(StatusCode::CONFLICT, error_body("BUSY")).await;

    let answer = client(address).start(SESSION, ExecutionId::new()).await;

    assert!(
        matches!(
            answer,
            Err(ClientError::Rejected {
                rejection: Rejection::Busy,
                ..
            })
        ),
        "{answer:?}"
    );
}

/// Boser's 401 body carries a gRPC code description, so the status alone decides.
#[tokio::test]
async fn a_rejected_session_is_unauthenticated_whatever_the_body() {
    for body in [error_body("BUSY"), "Unauthenticated".to_owned()] {
        let boser = MockBoser::default();
        let address = boser.serve(StatusCode::UNAUTHORIZED, body.clone()).await;

        let answer = client(address).check(SESSION, Vec::new()).await;

        assert_eq!(answer, Err(ClientError::Unauthenticated), "{body}");
    }
}

/// The gRPC code a client sees depends on this classification.
#[tokio::test]
async fn each_boser_error_code_is_classified() {
    let cases = [
        ("BUSY", StatusCode::CONFLICT, Rejection::Busy),
        ("EXPIRED", StatusCode::NOT_FOUND, Rejection::Expired),
        (
            "NOT_ENOUGH_SPACE",
            StatusCode::INSUFFICIENT_STORAGE,
            Rejection::NotEnoughSpace,
        ),
        (
            "UNSUPPORTED",
            StatusCode::CONFLICT,
            Rejection::PackagesUnavailable,
        ),
        (
            "ABSENT",
            StatusCode::CONFLICT,
            Rejection::PackagesUnavailable,
        ),
        (
            "UNHEALTHY",
            StatusCode::CONFLICT,
            Rejection::PackagesUnavailable,
        ),
        (
            "INVALID_ARGUMENT",
            StatusCode::BAD_REQUEST,
            Rejection::InvalidArgument,
        ),
        (
            "FIRMWARE_CHECK_FAILED",
            StatusCode::BAD_GATEWAY,
            Rejection::Internal,
        ),
        (
            "INTERNAL",
            StatusCode::INTERNAL_SERVER_ERROR,
            Rejection::Internal,
        ),
        (
            "A_CODE_FROM_THE_FUTURE",
            StatusCode::IM_A_TEAPOT,
            Rejection::Internal,
        ),
    ];
    for (code, status, rejection) in cases {
        let boser = MockBoser::default();
        let address = boser.serve(status, error_body(code)).await;

        let answer = client(address).check(SESSION, Vec::new()).await;

        assert_eq!(
            answer,
            Err(ClientError::Rejected {
                rejection,
                message: format!("{code} happened"),
            }),
            "{code}"
        );
    }
}

/// Boser answers a malformed request in plain text; that is still a definite answer.
#[tokio::test]
async fn an_error_without_an_error_body_names_the_status() {
    let boser = MockBoser::default();
    let address = boser
        .serve(StatusCode::UNPROCESSABLE_ENTITY, "not json")
        .await;

    let answer = client(address).check(SESSION, Vec::new()).await;

    // reqwest 0.11 and axum 0.8 sit on different `http` majors, so the two `StatusCode`s differ.
    assert_eq!(
        answer,
        Err(ClientError::Unexpected(
            reqwest::StatusCode::UNPROCESSABLE_ENTITY
        ))
    );
}

/// A success this build cannot read must not look like "nothing to upgrade".
#[tokio::test]
async fn an_undecodable_success_names_the_status() {
    let boser = MockBoser::default();
    let address = boser.serve(StatusCode::OK, r#"{"offer":7}"#).await;

    let answer = client(address).check(SESSION, Vec::new()).await;

    assert_eq!(
        answer,
        Err(ClientError::Unexpected(reqwest::StatusCode::OK))
    );
}

/// Boser publishes the run before it answers the start,
/// so the subscription must already ignore the snapshot that preceded it.
#[tokio::test]
async fn a_subscription_reports_only_later_snapshots() {
    let display = RunStatusService::new();
    let boser = BoserUpgrade::new(dead_address().await, display.clone());
    display.publish(snapshot(UpgradePhase::PackageRealizing));

    let subscription = boser.subscribe().display;

    assert!(
        !subscription
            .has_changed()
            .expect("BUG: the display outlives the test"),
        "what was displayed before the subscription is not news"
    );
    display.publish(snapshot(UpgradePhase::PackageBuilding));
    assert!(
        subscription
            .has_changed()
            .expect("BUG: the display outlives the test"),
        "a snapshot published after the subscription is reported"
    );
}

fn snapshot(phase: UpgradePhase) -> UpgradeRunSnapshot {
    UpgradeRunSnapshot {
        generation: UpgradeGeneration::new(0),
        id: Some(ExecutionId::new()),
        state: UpgradeRunStatus::Running {
            kind: UpgradeKind::Packages,
            phase: Some(phase),
            progress: None,
        },
    }
}
