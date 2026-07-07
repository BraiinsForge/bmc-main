// Copyright (C) 2026  Braiins Systems s.r.o.

//! End-to-end upgrade-scenario tests over gRPC. The FE cannot exercise
//! these yet (its generated protos predate the new upgrade.proto), so the
//! wire contract is verified here directly.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use bmc_grpc::web::authentication_service_client::AuthenticationServiceClient;
use bmc_grpc::web::upgrade_service_client::UpgradeServiceClient;
use bmc_grpc::web::{
    FirmwareUpgradePhase, LoginRequest, PackageUpgradePhase, StartUpgradeRequest,
    UpgradeDisruption, UpgradeProgress, upgrade_progress,
};
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const STREAM_TIMEOUT: Duration = Duration::from_secs(60);

struct MockInstance {
    child: Child,
    port: u16,
    mockfs: PathBuf,
    dir: tempfile::TempDir,
    password: String,
}

impl Drop for MockInstance {
    fn drop(&mut self) {
        _ = self.child.kill();
        _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("BUG: bind for port pick")
        .local_addr()
        .expect("BUG: local_addr")
        .port()
}

fn spawn_child(
    dir: &tempfile::TempDir,
    mockfs: &std::path::Path,
    port: u16,
    password: &str,
) -> Child {
    Command::new(env!("CARGO_BIN_EXE_bmc-mock"))
        .args([
            format!("--address=127.0.0.1:{port}"),
            format!("--mockfs-path={}", mockfs.display()),
            format!(
                "--mockfs-template={}",
                dir.path().join("template").display()
            ),
            format!("--www-path={}", dir.path().join("www").display()),
            format!("--sounds-dir={}", dir.path().join("sounds").display()),
            format!("--widgets-path={}", dir.path().join("widgets").display()),
            format!("--system-password={password}"),
            "--fast-upgrades".to_owned(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("BUG: spawn bmc-mock")
}

fn spawn_mock(scenario_json: &str) -> MockInstance {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let template = dir.path().join("template/etc");
    std::fs::create_dir_all(&template).expect("BUG: create template");
    std::fs::write(template.join("upgrade-scenario.json"), scenario_json)
        .expect("BUG: write scenario");
    let mockfs = dir.path().join("mockfs");
    let port = free_port();
    // Per-instance password (the unique tempdir name), so logging in on a
    // foreign mock that stole our port fails instead of succeeding.
    let password = format!(
        "pw-{}",
        dir.path()
            .file_name()
            .expect("BUG: tempdir has a name")
            .to_string_lossy()
    );
    let child = spawn_child(&dir, &mockfs, port, &password);

    MockInstance {
        child,
        port,
        mockfs,
        dir,
        password,
    }
}

// free_port() picks a port by bind-and-drop, so another process can steal
// it before the child binds. connect() detects the resulting child death
// and respawns on a fresh port instead of hanging until the deadline.
async fn connect(mock: &mut MockInstance) -> Channel {
    let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
    let mut respawns = 0;
    loop {
        if let Some(status) = mock.child.try_wait().expect("BUG: try_wait failed") {
            assert!(respawns < 3, "mock kept dying at startup: {status}");
            respawns += 1;
            mock.port = free_port();
            mock.child = spawn_child(&mock.dir, &mock.mockfs, mock.port, &mock.password);
        }
        let endpoint = format!("http://127.0.0.1:{}", mock.port);
        match Channel::from_shared(endpoint)
            .expect("BUG: endpoint parse")
            .connect()
            .await
        {
            Ok(channel) => return channel,
            Err(err) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "mock did not come up: {err}"
                );
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

async fn login(
    channel: Channel,
    password: &str,
) -> Result<MetadataValue<tonic::metadata::Ascii>, tonic::Status> {
    let mut auth = AuthenticationServiceClient::new(channel);
    let response = auth
        .login(LoginRequest {
            password: password.to_owned(),
        })
        .await?
        .into_inner();
    Ok(format!("session_id={}", response.token)
        .parse()
        .expect("BUG: cookie metadata parse"))
}

#[derive(Clone)]
struct CookieAuth(MetadataValue<tonic::metadata::Ascii>);

impl tonic::service::Interceptor for CookieAuth {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, tonic::Status> {
        req.metadata_mut().insert("cookie", self.0.clone());
        Ok(req)
    }
}

// A successful connect() can still land on a foreign mock: free_port()'s
// bind-and-drop race (see connect()'s comment) can hand two tests the same
// port, and the loser dies only after we already connected to the winner.
// The per-instance password makes login fail on a foreign mock; retrying
// the whole connect+login on a fresh port then recovers.
async fn upgrade_client(
    mock: &mut MockInstance,
) -> UpgradeServiceClient<tonic::service::interceptor::InterceptedService<Channel, CookieAuth>> {
    let mut last_err = None;
    for _ in 0..4 {
        let channel = connect(mock).await;
        match login(channel.clone(), &mock.password).await {
            Ok(cookie) => {
                return UpgradeServiceClient::with_interceptor(channel, CookieAuth(cookie));
            }
            Err(err) => {
                last_err = Some(err);
                _ = mock.child.kill();
                _ = mock.child.wait();
                mock.port = free_port();
                mock.child = spawn_child(&mock.dir, &mock.mockfs, mock.port, &mock.password);
            }
        }
    }
    panic!("BUG: login kept failing after respawns: {last_err:?}");
}

// Bounded drain so a regression that leaves the stream open fails the
// test instead of hanging it.
async fn drain_until_error(
    stream: &mut tonic::Streaming<UpgradeProgress>,
) -> (Vec<upgrade_progress::Event>, tonic::Status) {
    tokio::time::timeout(STREAM_TIMEOUT, async {
        let mut events = Vec::new();
        loop {
            match stream.message().await {
                Ok(Some(progress)) => events.extend(progress.event),
                Ok(None) => panic!("stream ended without error"),
                Err(status) => return (events, status),
            }
        }
    })
    .await
    .expect("stream did not error within the timeout")
}

#[tokio::test]
async fn default_scenario_offers_firmware_and_packages() {
    let mut mock = spawn_mock(r#"{"firmware": "available", "packages": "available"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    assert!(response.upgrade_id.is_some());
    assert_eq!(
        response.disruption,
        UpgradeDisruption::Reboot as i32,
        "an available firmware upgrade reboots the device"
    );
    let firmware = response.firmware.expect("firmware offered");
    assert!(!firmware.version.is_empty());
    assert!(!firmware.previous_releases.is_empty());
    let packages = response.packages.expect("packages offered");
    assert!(!packages.changes.is_empty());
    assert!(packages.bmc_version.is_some());
}

#[tokio::test]
async fn up_to_date_returns_empty_response() {
    let mut mock = spawn_mock(r#"{"firmware": "up-to-date", "packages": "unavailable"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    assert!(response.upgrade_id.is_none());
    assert!(response.firmware.is_none());
    assert!(response.packages.is_none());
}

#[tokio::test]
async fn fetch_failed_surfaces_grpc_error() {
    let mut mock = spawn_mock(r#"{"firmware": "up-to-date", "packages": "fetch-failed"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let status = client
        .check_for_upgrade(())
        .await
        .expect_err("package fetch failure must surface as a gRPC error");
    assert_eq!(status.code(), tonic::Code::Internal);
    assert!(status.message().starts_with("Cannot check for upgrade:"));
}

#[tokio::test]
async fn check_error_surfaces_grpc_error() {
    let mut mock = spawn_mock(r#"{"firmware": "check-error"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let status = client
        .check_for_upgrade(())
        .await
        .expect_err("check must fail");
    assert_eq!(status.code(), tonic::Code::Internal);
}

#[tokio::test]
async fn package_failure_is_not_masked_by_available_firmware() {
    let mut mock = spawn_mock(r#"{"firmware": "available", "packages": "fetch-failed"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let status = client
        .check_for_upgrade(())
        .await
        .expect_err("package failure must not be masked by firmware availability");
    assert_eq!(status.code(), tonic::Code::Internal);
    assert!(status.message().starts_with("Cannot check for upgrade:"));
}

#[tokio::test]
async fn packages_only_run_completes_all_phases() {
    let mut mock = spawn_mock(r#"{"firmware": "up-to-date", "packages": "available"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    assert_eq!(
        response.disruption,
        UpgradeDisruption::AppRestart as i32,
        "a packages-only upgrade only restarts the app"
    );
    let upgrade_id = response.upgrade_id.expect("upgrade id");

    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();

    let mut phases = Vec::new();
    let mut downloads = 0;
    let mut finished = false;
    tokio::time::timeout(STREAM_TIMEOUT, async {
        while let Some(progress) = stream.message().await.expect("BUG: stream errored") {
            match progress.event.expect("BUG: event set") {
                upgrade_progress::Event::PackagePhase(phase) => phases.push(phase),
                upgrade_progress::Event::Download(_) => downloads += 1,
                upgrade_progress::Event::Finished(()) => finished = true,
                upgrade_progress::Event::FirmwarePhase(_) => {}
            }
        }
    })
    .await
    .expect("stream did not finish within the timeout");
    assert!(finished);
    assert!(
        downloads > 0,
        "the client must receive package download progress"
    );
    assert_eq!(
        phases,
        vec![
            PackageUpgradePhase::Realizing as i32,
            PackageUpgradePhase::Verifying as i32,
            PackageUpgradePhase::Building as i32,
            PackageUpgradePhase::Activating as i32,
        ]
    );
}

#[tokio::test]
async fn firmware_wins_at_start_when_both_are_available() {
    let mut mock = spawn_mock(r#"{"firmware": "available", "packages": "available"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    let upgrade_id = response.upgrade_id.expect("upgrade id");

    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();

    // The firmware pipeline is identified by its FirmwarePhase events, which
    // the packages-only path never emits; the shared apply stage also emits
    // PackagePhase lines, so their presence is not a discriminator. Draining to
    // FirmwareApplying (the firmware run's terminal event before it goes dark)
    // confirms firmware, not packages, won the start.
    let mut firmware_phases = Vec::new();
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let progress = stream
                .message()
                .await
                .expect("BUG: stream errored before FirmwareApplying")
                .expect("BUG: stream ended before FirmwareApplying");
            if let Some(upgrade_progress::Event::FirmwarePhase(p)) = progress.event {
                firmware_phases.push(p);
                if p == FirmwareUpgradePhase::Applying as i32 {
                    return;
                }
            }
        }
    })
    .await
    .expect("timed out before FirmwareApplying");

    assert!(
        firmware_phases.contains(&(FirmwareUpgradePhase::Downloading as i32)),
        "the firmware pipeline must run its download phase"
    );
}

#[tokio::test]
async fn packages_apply_fail_errors_the_stream() {
    let mut mock =
        spawn_mock(r#"{"firmware": "up-to-date", "packages": "available", "run": "apply-fail"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    let upgrade_id = response.upgrade_id.expect("upgrade id");

    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();

    let (events, error) = drain_until_error(&mut stream).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            upgrade_progress::Event::PackagePhase(p)
                if *p == PackageUpgradePhase::Realizing as i32
        )),
        "apply failure must come after the realizing phase"
    );
    assert_eq!(error.code(), tonic::Code::Internal);
}

#[tokio::test]
async fn firmware_success_goes_dark_after_applying() {
    let mut mock = spawn_mock(r#"{"firmware": "available", "packages": "unavailable"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    let upgrade_id = response.upgrade_id.expect("upgrade id");

    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();

    let deadline = Duration::from_secs(120);
    tokio::time::timeout(deadline, async {
        loop {
            let progress = stream
                .message()
                .await
                .expect("BUG: stream errored before FirmwareApplying")
                .expect("BUG: stream ended before FirmwareApplying");
            if matches!(
                progress.event,
                Some(upgrade_progress::Event::FirmwarePhase(p))
                    if p == FirmwareUpgradePhase::Applying as i32
            ) {
                return;
            }
        }
    })
    .await
    .expect("timed out before FirmwareApplying");

    // Radio silence: the mock exits shortly after Applying (200 ms
    // reboot delay under --fast-upgrades), so the stream must break
    // (error or EOF) rather than deliver more events.
    let outcome = tokio::time::timeout(Duration::from_secs(30), stream.message())
        .await
        .expect("stream did not go dark after FirmwareApplying");
    match outcome {
        Err(_) | Ok(None) => {}
        Ok(Some(event)) => panic!("unexpected event after Applying: {event:?}"),
    }

    let exit_deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = mock.child.try_wait().expect("BUG: try_wait failed") {
            assert!(status.success(), "mock exited with failure: {status}");
            break;
        }
        assert!(
            std::time::Instant::now() < exit_deadline,
            "mock process did not exit after simulated reboot"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn firmware_download_fail_errors_the_stream() {
    let mut mock = spawn_mock(
        r#"{"firmware": "available", "packages": "unavailable", "run": "download-fail"}"#,
    );
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    let upgrade_id = response.upgrade_id.expect("upgrade id");

    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();

    let (events, error) = drain_until_error(&mut stream).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            upgrade_progress::Event::FirmwarePhase(p)
                if *p == FirmwareUpgradePhase::Downloading as i32
        )),
        "a download failure must come after the downloading phase"
    );
    assert_eq!(error.code(), tonic::Code::Internal);
}

#[tokio::test]
async fn firmware_hash_mismatch_errors_the_stream() {
    let mut mock = spawn_mock(
        r#"{"firmware": "available", "packages": "unavailable", "run": "hash-mismatch"}"#,
    );
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    let upgrade_id = response.upgrade_id.expect("upgrade id");

    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();

    let (events, error) = drain_until_error(&mut stream).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, upgrade_progress::Event::Download(_))),
        "full download must precede the mismatch"
    );
    assert_eq!(error.code(), tonic::Code::Internal);
}

#[tokio::test]
async fn firmware_apply_fail_errors_after_verify() {
    let mut mock =
        spawn_mock(r#"{"firmware": "available", "packages": "unavailable", "run": "apply-fail"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    let upgrade_id = response.upgrade_id.expect("upgrade id");

    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();

    let (events, error) = drain_until_error(&mut stream).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            upgrade_progress::Event::FirmwarePhase(p)
                if *p == FirmwareUpgradePhase::Verifying as i32
        )),
        "apply failure must come after verify"
    );
    assert_eq!(error.code(), tonic::Code::Internal);
}

#[tokio::test]
async fn unknown_upgrade_id_expires() {
    let mut mock = spawn_mock(r#"{"firmware": "available"}"#);
    let mut client = upgrade_client(&mut mock).await;

    let mut stream = client
        .start_upgrade(StartUpgradeRequest {
            upgrade_id: "bogus".to_owned(),
        })
        .await
        .expect("BUG: start failed")
        .into_inner();

    let (events, error) = drain_until_error(&mut stream).await;
    assert!(
        events.is_empty(),
        "an unknown id must fail before any progress event"
    );
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
}

#[tokio::test]
async fn scenario_flip_changes_next_check() {
    let mut mock = spawn_mock(r#"{"firmware": "available", "packages": "unavailable"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    assert!(response.firmware.is_some());

    std::fs::write(
        mock.mockfs.join("etc/upgrade-scenario.json"),
        r#"{"firmware": "up-to-date", "packages": "unavailable"}"#,
    )
    .expect("BUG: rewrite scenario");

    let response = client
        .check_for_upgrade(())
        .await
        .expect("BUG: check failed")
        .into_inner();
    assert!(response.firmware.is_none());
    assert!(response.upgrade_id.is_none());
}
