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

//! Boser's upgrade REST API, called on behalf of a logged-in user.

use std::net::SocketAddr;
use std::time::Duration;

use bmc_upgrade_types::{ExecutionId, InstallablePackage, wire};
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio::sync::watch;
use tracing::warn;

use crate::compositor::UpgradeRunSnapshot;
use crate::system_upgrade::RunStatusService;

/// Boser listens on this device, so a connect that is not immediate will not come.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Covers a check whose firmware index (10 s), package index (30 s a request)
/// and closure size estimate (2 min) are all slow, but not several slow index requests.
const CHECK_TIMEOUT: Duration = Duration::from_mins(3);
/// Boser answers a start once it admitted the run, without waiting for the run itself.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// What Boser's `error` code means to a caller of the upgrade RPCs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejection {
    Busy,
    Expired,
    NotEnoughSpace,
    PackagesUnavailable,
    InvalidArgument,
    Internal,
}

impl Rejection {
    fn from_code(code: &str) -> Self {
        match code {
            "BUSY" => Self::Busy,
            "EXPIRED" => Self::Expired,
            "NOT_ENOUGH_SPACE" => Self::NotEnoughSpace,
            "UNSUPPORTED" | "ABSENT" | "UNHEALTHY" => Self::PackagesUnavailable,
            "INVALID_ARGUMENT" => Self::InvalidArgument,
            _ => Self::Internal,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ClientError {
    /// Nothing was sent, so Boser saw nothing of the request.
    #[error("Boser is unreachable")]
    Unreachable,
    #[error("Boser did not answer")]
    NoAnswer,
    #[error("Boser did not answer the start request, the upgrade may have started")]
    StartUnconfirmed,
    #[error("Boser rejected the session")]
    Unauthenticated,
    #[error("{message}")]
    Rejected {
        rejection: Rejection,
        message: String,
    },
    #[error("unexpected Boser response (HTTP {0})")]
    Unexpected(StatusCode),
}

#[derive(Debug)]
pub(crate) struct BoserUpgrade {
    address: SocketAddr,
    client: Client,
    display: RunStatusService,
}

impl BoserUpgrade {
    pub(crate) fn new(address: SocketAddr, display: RunStatusService) -> Self {
        // The requests carry the user's session id: no environment proxy may see them,
        // and no redirect may carry them elsewhere.
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .expect("BUG: a static reqwest client configuration builds");
        Self {
            address,
            client,
            display,
        }
    }

    /// A receiver that reports only the snapshots published from now on.
    pub(crate) fn subscribe(&self) -> watch::Receiver<Option<UpgradeRunSnapshot>> {
        self.display.subscribe()
    }

    pub(crate) async fn check(
        &self,
        session_id: &str,
        packages: Vec<String>,
    ) -> Result<wire::CheckUpgradeResponse, ClientError> {
        let request = self.post(
            "/api/v1/upgrade/check",
            session_id,
            CHECK_TIMEOUT,
            &wire::CheckUpgradeRequest { packages },
        );
        decode(answer(request).await?).await
    }

    pub(crate) async fn installable(
        &self,
        session_id: &str,
    ) -> Result<Vec<InstallablePackage>, ClientError> {
        let request = self
            .client
            .get(self.url("/api/v1/upgrade/packages/installable"))
            .bearer_auth(session_id)
            .timeout(CHECK_TIMEOUT);
        let listed: wire::InstallablePackages = decode(answer(request).await?).await?;
        Ok(listed.packages)
    }

    pub(crate) async fn start(
        &self,
        session_id: &str,
        offer: ExecutionId,
    ) -> Result<(), ClientError> {
        let request = self.post(
            "/api/v1/upgrade/start",
            session_id,
            START_TIMEOUT,
            &wire::StartUpgradeRequest { offer_id: offer },
        );
        match answer(request).await {
            Ok(_no_content) => Ok(()),
            Err(ClientError::NoAnswer) => Err(ClientError::StartUnconfirmed),
            Err(error) => Err(error),
        }
    }

    fn url(&self, path: &str) -> String {
        // Plain http carries the session id: the address is Boser on this same device.
        format!("http://{}{path}", self.address)
    }

    fn post<B: Serialize>(
        &self,
        path: &str,
        session_id: &str,
        timeout: Duration,
        body: &B,
    ) -> RequestBuilder {
        let body = serde_json::to_vec(body).expect("BUG: upgrade wire requests serialize");
        self.client
            .post(self.url(path))
            .bearer_auth(session_id)
            .timeout(timeout)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
    }
}

async fn answer(request: RequestBuilder) -> Result<Response, ClientError> {
    let response = request.send().await.map_err(|error| {
        // The cause names Boser's address, so it goes to the log and not to the caller.
        warn!("Boser upgrade request failed: {error:?}");
        if error.is_connect() || error.is_builder() {
            ClientError::Unreachable
        } else {
            ClientError::NoAnswer
        }
    })?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    // Boser's 401 body carries a gRPC code description, not one of its upgrade codes.
    if status == StatusCode::UNAUTHORIZED {
        return Err(ClientError::Unauthenticated);
    }
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(error) => {
            warn!("Boser's HTTP {status} answer broke off: {error:?}");
            return Err(ClientError::Unexpected(status));
        }
    };
    match serde_json::from_slice::<wire::ErrorBody>(&body) {
        Ok(body) => Err(ClientError::Rejected {
            rejection: Rejection::from_code(&body.error),
            message: body.message,
        }),
        Err(error) => {
            warn!("Boser's HTTP {status} answer is not an upgrade error: {error}");
            Err(ClientError::Unexpected(status))
        }
    }
}

async fn decode<T: DeserializeOwned>(response: Response) -> Result<T, ClientError> {
    let status = response.status();
    let body = response.bytes().await.map_err(|error| {
        warn!("Boser's HTTP {status} answer broke off: {error:?}");
        ClientError::Unexpected(status)
    })?;
    serde_json::from_slice(&body).map_err(|error| {
        warn!("Boser's HTTP {status} answer does not match the upgrade contract: {error}");
        ClientError::Unexpected(status)
    })
}
