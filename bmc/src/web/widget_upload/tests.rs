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
use crate::web::grpc::tests::{assert_worded, pseudo_locale, sampled};
use axum::body::Body;
use axum::http::{Request, header};
use bmc_upgrade::packages::WidgetInstallErrorDiscriminants as InstallName;
use bmc_upgrade::widget_package::{
    FolderClash, FolderClashDiscriminants as ClashName, MAX_SHOWN_CHARS,
    WidgetPackageErrorDiscriminants as RejectionName,
};
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

    async fn login(
        &self,
        _password: &str,
    ) -> Result<Cookie<'static>, session::LoginError<Self::Error>> {
        if self.password_set {
            Err(session::LoginError::BadCredentials)
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
    result: std::sync::Mutex<Option<Result<InstalledWidget, WidgetInstallError>>>,
    received: std::sync::Mutex<Vec<Vec<u8>>>,
}

impl ScriptedInstaller {
    fn answering(result: Result<InstalledWidget, WidgetInstallError>) -> Arc<Self> {
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
    ) -> Result<InstalledWidget, WidgetInstallError> {
        let mut bytes = Vec::new();
        match upload.nar.read_to_end(&mut bytes).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                return Err(WidgetInstallError::UploadStalled);
            }
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionAborted => {
                return Err(WidgetInstallError::UploadAborted);
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
    let installer = ScriptedInstaller::answering(Err(WidgetInstallError::Rejected(
        WidgetPackageError::NativeBinary("bin/flip-clock".to_owned()),
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
        "Widget package rejected: \"bin/flip-clock\" is a native widget binary; \
         only WASM widgets can be installed."
    );
}

#[tokio::test]
async fn a_refused_downgrade_conflicts_with_the_installed_version() {
    let installer = ScriptedInstaller::answering(Err(WidgetInstallError::Downgrade {
        package_name: "widget-flip-clock".to_owned(),
        installed: "2.0.0".to_owned(),
        uploaded: "1.0.0".to_owned(),
    }));

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
    let installer = ScriptedInstaller::answering(Err(WidgetInstallError::FolderTaken {
        uploaded: "acme/clock".into(),
        installed: FolderClash::FlatWidget("acme".to_owned()),
    }));

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
    let installer = ScriptedInstaller::answering(Err(WidgetInstallError::UidTaken {
        uploaded: "clock".into(),
        installed: "acme/clock".into(),
    }));

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
    assert_eq!(body["error"], "Upload did not finish within 5 minutes!");
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
        "The upload ended before the archive was complete."
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
    for (error, expected, message) in [
        (
            WidgetInstallError::ProfileBusy,
            StatusCode::SERVICE_UNAVAILABLE,
            "Another install or upgrade is in progress.",
        ),
        (
            not_enough_space(),
            StatusCode::INSUFFICIENT_STORAGE,
            "Not enough space on the device to install the widget!",
        ),
        (
            WidgetInstallError::Failed("1".to_owned()),
            StatusCode::INTERNAL_SERVER_ERROR,
            "Widget install failed. See the device log for details.",
        ),
    ] {
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
        assert_eq!(body["error"], message);
    }
}

fn not_enough_space() -> WidgetInstallError {
    WidgetInstallError::Install(bmc_nix::upgrade::InstallError::NotEnoughSpace {
        free_bytes: 1,
        required_bytes: 2,
        unpacked_bytes: 2,
    })
}

#[test]
fn a_device_fault_keeps_its_detail_out_of_the_answer() {
    const DETAIL: &str = "/mnt/data/bmc/staging/x: Permission denied";
    let english = bmc_intl::shared_formatter(bmc_intl::Language::EN);
    for error in [
        WidgetInstallError::Store(bmc_nix::store::StoreCommandError::Exited {
            command: "--restore",
            status: std::process::ExitStatus::default(),
            stderr: DETAIL.to_owned(),
        }),
        WidgetInstallError::Staging(std::io::Error::other(DETAIL)),
        WidgetInstallError::ReadProfile(bmc_nix::manifest::ReadManifestError::Read(
            std::io::Error::other(DETAIL),
        )),
        WidgetInstallError::Install(bmc_nix::upgrade::InstallError::ReadManifest(
            bmc_nix::manifest::ReadManifestError::Read(std::io::Error::other(DETAIL)),
        )),
        WidgetInstallError::Failed(DETAIL.to_owned()),
    ] {
        let answer = wording::install_failure(&english, &error);
        assert!(!answer.contains(DETAIL), "{error:?} leaks into {answer}");
    }
}

#[test]
fn a_rejected_read_keeps_the_systems_own_text_out_of_the_answer() {
    const OS_TEXT: &str = "Permission denied";
    let english = bmc_intl::shared_formatter(bmc_intl::Language::EN);
    let error = WidgetPackageError::Io {
        path: "1".to_owned(),
        source: std::io::Error::other(OS_TEXT),
    };

    let answer = wording::package_rejected(&english, &error);

    assert!(!answer.contains(OS_TEXT), "{answer}");
}

#[test]
fn a_broken_manifest_quotes_its_detail_for_the_widgets_author() {
    let parse_failure = serde_json::from_str::<u8>("x").expect_err("BUG: x is no number");
    let manifest = bmc_widget_manifest::ManifestError::from(parse_failure);
    let detail = shown(&manifest.to_string());

    assert_eq!(
        wording::package_rejected(&pseudo_locale(), &WidgetPackageError::Manifest(manifest)),
        bmc_intl_pseudo::respell("Widget package rejected: manifest.json is invalid: {detail}.")
            .replace("{detail}", &detail),
        "the sentence is worded and the detail stays as written"
    );
}

/// A sample for each way a rejection is worded,
/// such as finding no widget and finding others.
fn every_rejection() -> Vec<WidgetPackageError> {
    let path = || "1".to_owned();
    sampled(|name| match name {
        RejectionName::TooLarge => vec![WidgetPackageError::TooLarge],
        RejectionName::Archive => vec![WidgetPackageError::Archive(path())],
        RejectionName::TooManyArchiveEntries => vec![WidgetPackageError::TooManyArchiveEntries],
        RejectionName::Io => vec![WidgetPackageError::Io {
            path: path(),
            source: std::io::Error::other("1"),
        }],
        RejectionName::FileTooLarge => vec![WidgetPackageError::FileTooLarge {
            path: path(),
            limit: 7,
        }],
        RejectionName::NotOneWidget => vec![
            WidgetPackageError::NotOneWidget(path()),
            WidgetPackageError::NotOneWidget(String::new()),
        ],
        RejectionName::NotADirectory => vec![
            WidgetPackageError::NotADirectory(path()),
            WidgetPackageError::NotADirectory("2".repeat(MAX_SHOWN_CHARS + 1)),
        ],
        RejectionName::BadWidgetName => vec![WidgetPackageError::BadWidgetName(path())],
        RejectionName::NotUtf8Name => vec![WidgetPackageError::NotUtf8Name(path())],
        RejectionName::Missing => vec![WidgetPackageError::Missing(path())],
        RejectionName::NativeBinary => vec![WidgetPackageError::NativeBinary(path())],
        RejectionName::Symlink => vec![WidgetPackageError::Symlink(path())],
        RejectionName::TooDeep => vec![WidgetPackageError::TooDeep(path())],
        RejectionName::TooManyEntries => vec![WidgetPackageError::TooManyEntries(path())],
        RejectionName::Unexpected => vec![WidgetPackageError::Unexpected(path())],
        RejectionName::UnexpectedExecutable => {
            vec![WidgetPackageError::UnexpectedExecutable(path())]
        }
        RejectionName::Manifest => {
            let parse_failure = serde_json::from_str::<u8>("x").expect_err("BUG: x is no number");
            vec![WidgetPackageError::Manifest(parse_failure.into())]
        }
        RejectionName::BinaryPath => vec![WidgetPackageError::BinaryPath {
            declared: path(),
            expected: "2".to_owned(),
        }],
        RejectionName::Programs => vec![WidgetPackageError::Programs],
        RejectionName::Icon => vec![WidgetPackageError::Icon(path())],
        RejectionName::WrapperNotExecutable => {
            vec![WidgetPackageError::WrapperNotExecutable(path())]
        }
        RejectionName::ProtocolMajor => vec![WidgetPackageError::ProtocolMajor {
            package: 1,
            device: 2,
        }],
        RejectionName::Wrapper => vec![WidgetPackageError::Wrapper(path())],
        RejectionName::NotWasm => vec![WidgetPackageError::NotWasm(path())],
    })
}

/// A sample of every failure, and one for each way a failure is worded,
/// such as a full disk and any other install fault.
fn every_install_failure() -> Vec<WidgetInstallError> {
    let os_error = || std::io::Error::other("1");
    sampled(|name| match name {
        InstallName::Rejected => vec![WidgetInstallError::Rejected(WidgetPackageError::TooLarge)],
        InstallName::Downgrade => vec![WidgetInstallError::Downgrade {
            package_name: "1".to_owned(),
            installed: "2".to_owned(),
            uploaded: "3".to_owned(),
        }],
        InstallName::FolderTaken => sampled(|clash| {
            vec![match clash {
                ClashName::FlatWidget => FolderClash::FlatWidget("2".to_owned()),
                ClashName::MaintainerFolder => FolderClash::MaintainerFolder("2".to_owned()),
            }]
        })
        .into_iter()
        .map(|installed| WidgetInstallError::FolderTaken {
            uploaded: "1".into(),
            installed,
        })
        .collect(),
        InstallName::UidTaken => vec![WidgetInstallError::UidTaken {
            uploaded: "1".into(),
            installed: "2".into(),
        }],
        InstallName::UploadStalled => vec![WidgetInstallError::UploadStalled],
        InstallName::ProfileBusy => vec![WidgetInstallError::ProfileBusy],
        InstallName::UploadAborted => vec![WidgetInstallError::UploadAborted],
        InstallName::ReadProfile => vec![WidgetInstallError::ReadProfile(
            bmc_nix::manifest::ReadManifestError::Read(os_error()),
        )],
        InstallName::Staging => vec![WidgetInstallError::Staging(os_error())],
        InstallName::Store => vec![WidgetInstallError::Store(
            bmc_nix::store::StoreCommandError::Exited {
                command: "1",
                status: std::process::ExitStatus::default(),
                stderr: "1".to_owned(),
            },
        )],
        InstallName::Install => vec![
            not_enough_space(),
            WidgetInstallError::Install(bmc_nix::upgrade::InstallError::ReadManifest(
                bmc_nix::manifest::ReadManifestError::Read(os_error()),
            )),
        ],
        InstallName::Failed => vec![WidgetInstallError::Failed("1".to_owned())],
    })
}

/// What `install_failure` should answer where another sample already yields that sentence:
/// the carried rejection's, or the generic one `Failed` stands for.
fn repeated_sentence(intl: &bmc_intl::Intl, error: &WidgetInstallError) -> Option<String> {
    match error {
        WidgetInstallError::Install(bmc_nix::upgrade::InstallError::NotEnoughSpace { .. })
        | WidgetInstallError::Downgrade { .. }
        | WidgetInstallError::FolderTaken { .. }
        | WidgetInstallError::UidTaken { .. }
        | WidgetInstallError::UploadStalled
        | WidgetInstallError::ProfileBusy
        | WidgetInstallError::UploadAborted
        | WidgetInstallError::Failed(_) => None,
        WidgetInstallError::Rejected(rejected) => Some(wording::package_rejected(intl, rejected)),
        WidgetInstallError::ReadProfile(_)
        | WidgetInstallError::Staging(_)
        | WidgetInstallError::Store(_)
        | WidgetInstallError::Install(_) => Some(wording::install_failure(
            intl,
            &WidgetInstallError::Failed("1".to_owned()),
        )),
    }
}

#[test]
fn every_refused_upload_is_worded() {
    let rejected = every_rejection();
    let failed = every_install_failure();
    assert_worded(|intl| {
        let mut refusals = vec![wording::wrong_content_type(intl, "1")];
        refusals.extend(
            rejected
                .iter()
                // Quotes the parser's English, which en-XA leaves as written.
                .filter(|error| !matches!(error, WidgetPackageError::Manifest(_)))
                .map(|error| wording::package_rejected(intl, error)),
        );
        for error in &failed {
            let answer = wording::install_failure(intl, error);
            match repeated_sentence(intl, error) {
                Some(repeated) => assert_eq!(answer, repeated, "{error:?}"),
                None => refusals.push(answer),
            }
        }
        refusals
    });
}

fn as_sentence(display: &str) -> String {
    let mut chars = display.chars();
    let first = chars.next().map(|c| c.to_uppercase().to_string());
    let sentence = format!("{}{}", first.unwrap_or_default(), chars.as_str());
    if sentence.ends_with('.') {
        sentence
    } else {
        format!("{sentence}.")
    }
}

/// True where the wording drops the parser's or the OS's detail `Display` appends.
fn leaves_detail_to_the_log(error: &WidgetPackageError) -> bool {
    match error {
        WidgetPackageError::Archive(_) | WidgetPackageError::Io { .. } => true,
        WidgetPackageError::TooLarge
        | WidgetPackageError::TooManyArchiveEntries
        | WidgetPackageError::FileTooLarge { .. }
        | WidgetPackageError::NotOneWidget(_)
        | WidgetPackageError::NotADirectory(_)
        | WidgetPackageError::BadWidgetName(_)
        | WidgetPackageError::NotUtf8Name(_)
        | WidgetPackageError::Missing(_)
        | WidgetPackageError::NativeBinary(_)
        | WidgetPackageError::Symlink(_)
        | WidgetPackageError::TooDeep(_)
        | WidgetPackageError::TooManyEntries(_)
        | WidgetPackageError::Unexpected(_)
        | WidgetPackageError::UnexpectedExecutable(_)
        | WidgetPackageError::Manifest(_)
        | WidgetPackageError::BinaryPath { .. }
        | WidgetPackageError::Programs
        | WidgetPackageError::Icon(_)
        | WidgetPackageError::WrapperNotExecutable(_)
        | WidgetPackageError::ProtocolMajor { .. }
        | WidgetPackageError::Wrapper(_)
        | WidgetPackageError::NotWasm(_) => false,
    }
}

/// False for those reworded on purpose or worded generically with their detail in the log,
/// and for `Rejected`, which the rejection test pins.
fn restates_its_display(error: &WidgetInstallError) -> bool {
    match error {
        WidgetInstallError::Downgrade { .. }
        | WidgetInstallError::FolderTaken { .. }
        | WidgetInstallError::UidTaken { .. }
        | WidgetInstallError::UploadAborted => true,
        WidgetInstallError::Rejected(_)
        | WidgetInstallError::UploadStalled
        | WidgetInstallError::ProfileBusy
        | WidgetInstallError::ReadProfile(_)
        | WidgetInstallError::Staging(_)
        | WidgetInstallError::Store(_)
        | WidgetInstallError::Install(_)
        | WidgetInstallError::Failed(_) => false,
    }
}

#[test]
fn every_rejection_words_its_display_in_english() {
    let english = bmc_intl::shared_formatter(bmc_intl::Language::EN);
    for error in every_rejection() {
        let display = error.to_string();
        let worded = if leaves_detail_to_the_log(&error) {
            display
                .split_once(": ")
                .expect("BUG: the detail follows a colon")
                .0
        } else {
            &display
        };
        assert_eq!(
            wording::package_rejected(&english, &error),
            as_sentence(&format!("Widget package rejected: {worded}")),
        );
    }
}

#[test]
fn every_install_failure_restating_its_display_words_it_in_english() {
    let english = bmc_intl::shared_formatter(bmc_intl::Language::EN);
    for error in every_install_failure()
        .iter()
        .filter(|error| restates_its_display(error))
    {
        assert_eq!(
            wording::install_failure(&english, error),
            as_sentence(&error.to_string()),
        );
    }
}
