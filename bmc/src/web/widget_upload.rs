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

//! `POST /api/install_widget_package`, the widget package upload.
//! The body streams into `nix-store --restore`;
//! one gRPC-web message would hold the whole archive in RAM.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use bmc_upgrade::packages::{InstalledWidget, WidgetInstallOutcome};
use bmc_upgrade::widget_package::{
    ArchiveStream, MAX_ARCHIVE_BYTES, WidgetPackageError, WidgetUpload, deadline_archive, shown,
};
use futures::TryStreamExt as _;
use serde::Serialize;
use tokio_util::io::StreamReader;

use super::no_password::NoPasswordLayer;
use super::session::{SessionLayer, extract_session};
use crate::BmcManager;
use crate::session::Manager as SessionManager;
use crate::system_upgrade::boser::UpgradeRoute;
use crate::system_upgrade::{SystemUpgradeError, SystemUpgradeService};
use bmc_upgrade::firmware::FirmwareIndex;

pub(crate) const PATH: &str = "/api/install_widget_package";
/// Required on every upload. On a device without a password the session
/// layer logs any cookie-less request in, so a cross-site form post would
/// install too; a content type outside the CORS safelist makes the browser
/// preflight first, and nothing here answers a preflight.
pub(crate) const ARCHIVE_CONTENT_TYPE: &str = "application/x-nix-archive";

/// A widget package install, free of the upgrade service's generics.
#[async_trait::async_trait]
pub(crate) trait WidgetPackageInstaller: Send + Sync + 'static {
    async fn install(&self, upload: WidgetUpload) -> Result<InstalledWidget, SystemUpgradeError>;
}

#[async_trait::async_trait]
impl<T: FirmwareIndex, U: BmcManager> WidgetPackageInstaller for SystemUpgradeService<T, U> {
    async fn install(&self, upload: WidgetUpload) -> Result<InstalledWidget, SystemUpgradeError> {
        self.install_widget_package(upload).await
    }
}

/// Whether this product installs widgets itself; a Boser-managed one has
/// nowhere to put them.
#[derive(Debug, Clone, Copy)]
pub(crate) enum InstallTarget {
    Local,
    BoserManaged,
}

impl From<&UpgradeRoute> for InstallTarget {
    fn from(route: &UpgradeRoute) -> Self {
        match route {
            UpgradeRoute::Local => Self::Local,
            UpgradeRoute::Boser(_) | UpgradeRoute::BoserUnavailable => Self::BoserManaged,
        }
    }
}

struct Uploads<I> {
    installer: Arc<I>,
    target: InstallTarget,
}

/// The route behind the same session layers as the gRPC services;
/// [`ARCHIVE_CONTENT_TYPE`] guards a device without a password.
pub(crate) fn router<S: SessionManager, I: WidgetPackageInstaller>(
    session_manager: Arc<S>,
    installer: Arc<I>,
    target: InstallTarget,
) -> Router {
    Router::new()
        .route(PATH, post(handle::<S, I>))
        .with_state(Arc::new(Uploads { installer, target }))
        .layer(SessionLayer::new(session_manager.clone()))
        .layer(NoPasswordLayer::new(session_manager))
        .layer(tower_cookies::CookieManagerLayer::new())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Installed,
    Replaced,
    Reinstalled,
    AlreadyInstalled,
}

#[derive(Debug, Serialize)]
struct Installed {
    package_name: String,
    display_name: String,
    version: String,
    outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_version: Option<String>,
}

impl From<InstalledWidget> for Installed {
    fn from(installed: InstalledWidget) -> Self {
        let (outcome, previous_version) = match installed.outcome {
            WidgetInstallOutcome::Installed => (Outcome::Installed, None),
            WidgetInstallOutcome::Replaced { previous_version } => {
                (Outcome::Replaced, Some(shown(&previous_version)))
            }
            WidgetInstallOutcome::Reinstalled => (Outcome::Reinstalled, None),
            WidgetInstallOutcome::AlreadyInstalled => (Outcome::AlreadyInstalled, None),
        };
        Self {
            package_name: installed.package_name,
            display_name: installed.display_name,
            version: installed.version,
            outcome,
            previous_version,
        }
    }
}

#[derive(Debug, Serialize)]
struct Failure {
    error: String,
}

fn failure(status: StatusCode, error: impl Into<String>) -> Response {
    (
        status,
        Json(Failure {
            error: error.into(),
        }),
    )
        .into_response()
}

async fn handle<S: SessionManager, I: WidgetPackageInstaller>(
    State(uploads): State<Arc<Uploads<I>>>,
    request: Request,
) -> Response {
    if let Err(error) = extract_session::<S>(request.extensions()) {
        return failure(StatusCode::UNAUTHORIZED, error.message.unwrap_or_default());
    }
    if request
        .headers()
        .get(header::CONTENT_TYPE)
        .map(HeaderValue::as_bytes)
        != Some(ARCHIVE_CONTENT_TYPE.as_bytes())
    {
        return failure(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            format!("the archive must be sent as {ARCHIVE_CONTENT_TYPE}"),
        );
    }
    if let InstallTarget::BoserManaged = uploads.target {
        return failure(
            StatusCode::CONFLICT,
            "widget packages are installed on the BMC only; this platform is managed by Boser",
        );
    }
    // The stream cap would catch this too;
    // an announced size is refused before the upload starts.
    let announced_bytes = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if announced_bytes.is_some_and(|size| size > MAX_ARCHIVE_BYTES) {
        return failure(
            StatusCode::PAYLOAD_TOO_LARGE,
            WidgetPackageError::TooLarge.to_string(),
        );
    }
    let body: ArchiveStream = Box::pin(StreamReader::new(
        request
            .into_body()
            .into_data_stream()
            // Whatever hyper reports mid-body is the client's connection, not the device.
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::ConnectionAborted, error)),
    ));
    let upload = WidgetUpload {
        nar: deadline_archive(body),
        announced_bytes,
    };
    match uploads.installer.install(upload).await {
        Ok(installed) => Json(Installed::from(installed)).into_response(),
        Err(error) => failure(status_of(&error), error.to_string()),
    }
}

fn status_of(error: &SystemUpgradeError) -> StatusCode {
    match error {
        SystemUpgradeError::WidgetPackageRejected(_) | SystemUpgradeError::WidgetUploadAborted => {
            StatusCode::BAD_REQUEST
        }
        SystemUpgradeError::WidgetPackageTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        SystemUpgradeError::WidgetDowngradeRefused(_)
        | SystemUpgradeError::WidgetFolderTaken(_) => StatusCode::CONFLICT,
        SystemUpgradeError::WidgetUploadStalled => StatusCode::REQUEST_TIMEOUT,
        SystemUpgradeError::UpgradeInProgress | SystemUpgradeError::WidgetInstallBusy => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        SystemUpgradeError::NotEnoughSpace | SystemUpgradeError::WidgetInstallNoSpace => {
            StatusCode::INSUFFICIENT_STORAGE
        }
        SystemUpgradeError::FailedToDetectCurrentVersion
        | SystemUpgradeError::DownloadedImageHashMismatch { .. }
        | SystemUpgradeError::VerifyFailed
        | SystemUpgradeError::FailedToDownload(_)
        | SystemUpgradeError::UnableToCheckForUpgrade(_)
        | SystemUpgradeError::UpgradeFailed
        | SystemUpgradeError::InvalidImage
        | SystemUpgradeError::UpgradeExpired
        | SystemUpgradeError::PackageUpgradeFailed(_)
        | SystemUpgradeError::WidgetInstallFailed
        | SystemUpgradeError::PendingInstallWriteFailed(_)
        | SystemUpgradeError::PackageCheckFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
