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

use super::*;
use crate::session;
use axum::body::Body;
use axum::http::{Request, header};
use bmc_upgrade::packages::WidgetInstallOutcome;
use tokio::io::AsyncReadExt as _;
use tower::ServiceExt as _;
use tower_cookies::Cookie;

const SESSION_COOKIE: &str = "session=valid";

#[derive(Debug, Clone)]
struct StubSession;

impl session::Handle for StubSession {
    fn is_valid(&self) -> bool {
        true
    }
    fn id(&self) -> String {
        "test-session".to_owned()
    }
}

/// Knows one session cookie. With a password set it refuses the
/// no-password layer's login, so a request without the cookie stays
/// anonymous; without one, the layer logs every request in.
#[derive(Debug, Default)]
struct OneSession {
    password_set: bool,
}

impl OneSession {
    fn locked() -> Arc<Self> {
        Arc::new(Self { password_set: true })
    }

    fn open() -> Arc<Self> {
        Arc::new(Self {
            password_set: false,
        })
    }
}

#[async_trait::async_trait]
impl session::Manager for OneSession {
    type Error = std::io::Error;
    type Session = StubSession;
    const SESSION_TIMEOUT: u32 = 0;

    async fn login(&self, _password: &str) -> Result<Cookie<'static>, Self::Error> {
        if self.password_set {
            Err(std::io::Error::other("a password is set"))
        } else {
            Ok(Cookie::parse(SESSION_COOKIE).expect("BUG: static cookie"))
        }
    }
    async fn logout(&self, _session: Self::Session) -> Result<Cookie<'static>, Self::Error> {
        unimplemented!("BUG: the upload never logs out")
    }
    async fn logout_all_related(&self, _session: Self::Session) -> Result<(), Self::Error> {
        unimplemented!("BUG: the upload never logs out")
    }
    async fn extend(&self, _session: Self::Session) -> Result<Cookie<'static>, Self::Error> {
        Ok(Cookie::parse(SESSION_COOKIE).expect("BUG: static cookie"))
    }
    async fn find(&self, cookies: &[Cookie<'_>]) -> Result<Self::Session, Self::Error> {
        let expected = Cookie::parse(SESSION_COOKIE).expect("BUG: static cookie");
        cookies
            .iter()
            .any(|cookie| cookie.name() == expected.name() && cookie.value() == expected.value())
            .then_some(StubSession)
            .ok_or_else(|| std::io::Error::other("no session"))
    }
}

/// Drains the upload and answers with a scripted result.
#[derive(Debug)]
struct ScriptedInstaller {
    result: std::sync::Mutex<Option<Result<InstalledWidget, SystemUpgradeError>>>,
    received: std::sync::Mutex<Vec<Vec<u8>>>,
}

impl ScriptedInstaller {
    fn answering(result: Result<InstalledWidget, SystemUpgradeError>) -> Arc<Self> {
        Arc::new(Self {
            result: std::sync::Mutex::new(Some(result)),
            received: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn received(&self) -> Vec<Vec<u8>> {
        self.received
            .lock()
            .expect("BUG: received poisoned")
            .clone()
    }
}

#[async_trait::async_trait]
impl WidgetPackageInstaller for ScriptedInstaller {
    async fn install(
        &self,
        mut upload: WidgetUpload,
    ) -> Result<InstalledWidget, SystemUpgradeError> {
        let mut bytes = Vec::new();
        match upload.nar.read_to_end(&mut bytes).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                return Err(SystemUpgradeError::WidgetUploadStalled);
            }
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionAborted => {
                return Err(SystemUpgradeError::WidgetUploadAborted);
            }
            Err(error) => panic!("BUG: the test body is in memory: {error}"),
        }
        self.received
            .lock()
            .expect("BUG: received poisoned")
            .push(bytes);
        self.result
            .lock()
            .expect("BUG: result poisoned")
            .take()
            .expect("BUG: the installer answers one upload")
    }
}

fn installed() -> InstalledWidget {
    InstalledWidget {
        package_name: "widget-flip-clock".to_owned(),
        display_name: "Flip clock".to_owned(),
        version: "1.2.0".to_owned(),
        outcome: WidgetInstallOutcome::Replaced {
            previous_version: "1.1.0".to_owned(),
        },
    }
}

fn upload(cookie: Option<&str>, body: &'static [u8]) -> Request<Body> {
    let mut request = Request::post(PATH).header(header::CONTENT_TYPE, ARCHIVE_CONTENT_TYPE);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    request.body(Body::from(body)).expect("BUG: static request")
}

async fn send(
    installer: Arc<ScriptedInstaller>,
    target: InstallTarget,
    request: Request<Body>,
) -> (StatusCode, serde_json::Value) {
    send_to(OneSession::locked(), installer, target, request).await
}

async fn send_to(
    sessions: Arc<OneSession>,
    installer: Arc<ScriptedInstaller>,
    target: InstallTarget,
    request: Request<Body>,
) -> (StatusCode, serde_json::Value) {
    let response = router(sessions, installer, target)
        .oneshot(request)
        .await
        .expect("BUG: the router is infallible");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("BUG: in-memory body");
    let json = serde_json::from_slice(&body).expect("BUG: every answer is JSON");
    (status, json)
}

#[tokio::test]
async fn an_anonymous_upload_is_turned_away_before_the_body_is_read() {
    let installer = ScriptedInstaller::answering(Ok(installed()));

    let (status, body) = send(
        installer.clone(),
        InstallTarget::Local,
        upload(None, b"nar"),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].is_string(), "{body}");
    assert!(installer.received().is_empty());
}

#[tokio::test]
async fn a_logged_in_upload_streams_the_body_to_the_installer() {
    let installer = ScriptedInstaller::answering(Ok(installed()));

    let (status, body) = send(
        installer.clone(),
        InstallTarget::Local,
        upload(Some(SESSION_COOKIE), b"nar bytes"),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({
            "package_name": "widget-flip-clock",
            "display_name": "Flip clock",
            "version": "1.2.0",
            "outcome": "replaced",
            "previous_version": "1.1.0",
        })
    );
    assert_eq!(installer.received(), vec![b"nar bytes".to_vec()]);
}

#[tokio::test]
async fn an_announced_oversized_archive_is_refused_without_uploading() {
    let installer = ScriptedInstaller::answering(Ok(installed()));
    let request = Request::post(PATH)
        .header(header::COOKIE, SESSION_COOKIE)
        .header(header::CONTENT_TYPE, ARCHIVE_CONTENT_TYPE)
        .header(header::CONTENT_LENGTH, (MAX_ARCHIVE_BYTES + 1).to_string())
        .body(Body::from(&b"short"[..]))
        .expect("BUG: static request");

    let (status, _body) = send(installer.clone(), InstallTarget::Local, request).await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(installer.received().is_empty());
}

#[tokio::test]
async fn a_rejected_archive_is_the_clients_fault() {
    let installer = ScriptedInstaller::answering(Err(SystemUpgradeError::WidgetPackageRejected(
        "bin/flip-clock is a native binary".to_owned(),
    )));

    let (status, body) = send(
        installer,
        InstallTarget::Local,
        upload(Some(SESSION_COOKIE), b"elf"),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        "Widget package rejected: bin/flip-clock is a native binary"
    );
}

#[tokio::test]
async fn a_refused_downgrade_conflicts_with_the_installed_version() {
    let installer = ScriptedInstaller::answering(Err(SystemUpgradeError::WidgetDowngradeRefused(
        "widget-flip-clock 2.0.0 is installed; uploading the older 1.0.0 would downgrade it"
            .to_owned(),
    )));

    let (status, _body) = send(
        installer,
        InstallTarget::Local,
        upload(Some(SESSION_COOKIE), b"nar"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the archive is fine; the installed version stands in the way"
    );
}

#[tokio::test]
async fn a_taken_folder_conflicts_with_the_installed_widget() {
    let installer = ScriptedInstaller::answering(Err(SystemUpgradeError::WidgetFolderTaken(
        "acme/clock cannot be installed next to the installed widget acme".to_owned(),
    )));

    let (status, _body) = send(
        installer,
        InstallTarget::Local,
        upload(Some(SESSION_COOKIE), b"nar"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the archive is fine; an installed widget stands in the way"
    );
}

#[tokio::test]
async fn a_taken_uid_conflicts_with_the_installed_widget() {
    let installer = ScriptedInstaller::answering(Err(SystemUpgradeError::WidgetUidTaken(
        "clock has the uid of the installed widget acme/clock".to_owned(),
    )));

    let (status, _body) = send(
        installer,
        InstallTarget::Local,
        upload(Some(SESSION_COOKIE), b"nar"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the archive is fine; an installed widget stands in the way"
    );
}

#[tokio::test]
async fn a_boser_managed_platform_has_nowhere_to_install() {
    let installer = ScriptedInstaller::answering(Ok(installed()));

    let (status, _body) = send(
        installer.clone(),
        InstallTarget::BoserManaged,
        upload(Some(SESSION_COOKIE), b"nar"),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(installer.received().is_empty());
}

#[tokio::test]
async fn on_an_open_device_only_a_preflighted_content_type_may_install() {
    let installer = ScriptedInstaller::answering(Ok(installed()));
    let form_post = Request::post(PATH)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(&b"nar"[..]))
        .expect("BUG: static request");

    let (status, body) = send_to(
        OneSession::open(),
        installer.clone(),
        InstallTarget::Local,
        form_post,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "a cross-site form can send text/plain without a preflight: {body}"
    );
    assert!(installer.received().is_empty());

    let (status, _body) = send_to(
        OneSession::open(),
        installer.clone(),
        InstallTarget::Local,
        upload(None, b"nar"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the open device logs the browser in by itself; only the content type was missing"
    );
}

#[tokio::test(start_paused = true)]
async fn an_upload_that_stalls_is_cut_off_at_the_deadline() {
    let installer = ScriptedInstaller::answering(Ok(installed()));
    let never_finishes = futures::stream::pending::<Result<axum::body::Bytes, std::io::Error>>();
    let request = Request::post(PATH)
        .header(header::COOKIE, SESSION_COOKIE)
        .header(header::CONTENT_TYPE, ARCHIVE_CONTENT_TYPE)
        .body(Body::from_stream(never_finishes))
        .expect("BUG: static request");

    let (status, body) = send(installer.clone(), InstallTarget::Local, request).await;

    assert_eq!(status, StatusCode::REQUEST_TIMEOUT, "{body}");
    assert_eq!(body["error"], "upload did not finish within 5 minutes");
    assert!(installer.received().is_empty());
}

#[tokio::test]
async fn a_body_that_breaks_off_reaches_the_installer_as_the_clients_disconnect() {
    let installer = ScriptedInstaller::answering(Ok(installed()));
    let breaks_off = futures::stream::iter([
        Ok(axum::body::Bytes::from_static(b"half an archi")),
        Err(std::io::Error::other("connection reset by peer")),
    ]);
    let request = Request::post(PATH)
        .header(header::COOKIE, SESSION_COOKIE)
        .header(header::CONTENT_TYPE, ARCHIVE_CONTENT_TYPE)
        .body(Body::from_stream(breaks_off))
        .expect("BUG: static request");

    let (status, body) = send(installer.clone(), InstallTarget::Local, request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"],
        "the upload ended before the archive was complete"
    );
    assert!(installer.received().is_empty());
}

/// The version an install replaced came from the profile,
/// which the upload's checks never saw.
#[tokio::test]
async fn an_echoed_previous_version_is_cut_to_what_a_toast_shows() {
    let long = format!("1.0.0-{}", "x".repeat(4096));
    let installer = ScriptedInstaller::answering(Ok(InstalledWidget {
        outcome: WidgetInstallOutcome::Replaced {
            previous_version: long,
        },
        ..installed()
    }));

    let (status, body) = send(
        installer,
        InstallTarget::Local,
        upload(Some(SESSION_COOKIE), b"nar bytes"),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let echoed = body["previous_version"]
        .as_str()
        .expect("BUG: versions are strings");
    assert!(
        echoed.chars().count() < 100 && echoed.ends_with('…'),
        "the previous version is cut with an ellipsis: {echoed}"
    );
}

#[tokio::test]
async fn a_device_side_failure_answers_with_its_own_server_status() {
    for (error, expected) in [
        (
            SystemUpgradeError::WidgetInstallBusy,
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            SystemUpgradeError::WidgetInstallNoSpace,
            StatusCode::INSUFFICIENT_STORAGE,
        ),
        (
            SystemUpgradeError::WidgetInstallFailed,
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ] {
        let message = error.to_string();

        let (status, body) = send(
            ScriptedInstaller::answering(Err(error)),
            InstallTarget::Local,
            upload(Some(SESSION_COOKIE), b"nar"),
        )
        .await;

        assert_eq!(
            status, expected,
            "busy, full and failed are not the uploader's fault: {message}"
        );
        assert_eq!(body["error"], message.as_str());
    }
}
