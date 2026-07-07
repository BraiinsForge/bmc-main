// Copyright (C) 2026  Braiins Systems s.r.o.

//! End-to-end upgrade-scenario tests over gRPC. The FE cannot exercise
//! these yet (its generated protos predate the new upgrade.proto), so the
//! wire contract is verified here directly.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use bmc_grpc::web::authentication_service_client::AuthenticationServiceClient;
use bmc_grpc::web::scene_management_service_client::SceneManagementServiceClient;
use bmc_grpc::web::upgrade_service_client::UpgradeServiceClient;
use bmc_grpc::web::{
    CheckForUpgradeRequest, FirmwareUpgradePhase, LoginRequest, PackageUpgradePhase,
    StartUpgradeRequest, UpgradeDisruption, UpgradeProgress, WidgetCategory, upgrade_progress,
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
    index_path: Option<PathBuf>,
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
    index_path: Option<&std::path::Path>,
) -> Child {
    let mut args = vec![
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
    ];
    if let Some(index) = index_path {
        args.push(format!("--package-index={}", index.display()));
    }
    Command::new(env!("CARGO_BIN_EXE_bmc-mock"))
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("BUG: spawn bmc-mock")
}

fn spawn_mock(scenario_json: &str) -> MockInstance {
    spawn_mock_inner(scenario_json, None)
}

/// Spawn a mock whose installable catalog is served from a real package
/// index file (the `--package-index` path), not the fabricated fallback.
fn spawn_mock_with_index(scenario_json: &str, index_json: &str) -> MockInstance {
    spawn_mock_inner(scenario_json, Some(index_json))
}

fn spawn_mock_inner(scenario_json: &str, index_json: Option<&str>) -> MockInstance {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let template = dir.path().join("template/etc");
    std::fs::create_dir_all(&template).expect("BUG: create template");
    std::fs::write(template.join("upgrade-scenario.json"), scenario_json)
        .expect("BUG: write scenario");
    let index_path = index_json.map(|json| {
        let path = dir.path().join("package-index.json");
        std::fs::write(&path, json).expect("BUG: write index");
        path
    });
    // The fallback catalog is derived from the --widgets-path tree, so give
    // the mock one widget (widget-flip-clock) with a renderable icon for the
    // shadowed-scenario tests to discover.
    let widget_dir = dir.path().join("widgets/flip-clock");
    std::fs::create_dir_all(&widget_dir).expect("BUG: create widget dir");
    std::fs::write(widget_dir.join("icon.svg"), "<svg/>").expect("BUG: write icon");
    // Registry discovery requires an executable binary at the manifest's
    // `binary` path, so the staged widget is discoverable once installed.
    let bin_dir = widget_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("BUG: create bin dir");
    let binary = bin_dir.join("flip-clock");
    std::fs::write(&binary, "#!/bin/sh\n").expect("BUG: write binary");
    std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("BUG: chmod binary");
    std::fs::write(
        widget_dir.join("manifest.json"),
        r#"{"uid":"7cb584a8-1f26-42a0-867e-955aadd2391c","version":"1.0.0",
           "name":"Flip Clock","description":"A retro split-flap clock face.",
           "binary":"bin/flip-clock","icon":"icon.svg","category":"clock",
           "supported_viewports":[{"type":"rectangular","min_width":100,
           "max_width":200,"min_height":100,"max_height":200}]}"#,
    )
    .expect("BUG: write manifest");
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
    let child = spawn_child(&dir, &mockfs, port, &password, index_path.as_deref());

    MockInstance {
        child,
        port,
        mockfs,
        dir,
        password,
        index_path,
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
            mock.child = spawn_child(
                &mock.dir,
                &mock.mockfs,
                mock.port,
                &mock.password,
                mock.index_path.as_deref(),
            );
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
async fn authenticated(mock: &mut MockInstance) -> (Channel, CookieAuth) {
    let mut last_err = None;
    for _ in 0..4 {
        let channel = connect(mock).await;
        match login(channel.clone(), &mock.password).await {
            Ok(cookie) => return (channel, CookieAuth(cookie)),
            Err(err) => {
                last_err = Some(err);
                _ = mock.child.kill();
                _ = mock.child.wait();
                mock.port = free_port();
                mock.child = spawn_child(
                    &mock.dir,
                    &mock.mockfs,
                    mock.port,
                    &mock.password,
                    mock.index_path.as_deref(),
                );
            }
        }
    }
    panic!("BUG: login kept failing after respawns: {last_err:?}");
}

async fn upgrade_client(
    mock: &mut MockInstance,
) -> UpgradeServiceClient<tonic::service::interceptor::InterceptedService<Channel, CookieAuth>> {
    let (channel, auth) = authenticated(mock).await;
    UpgradeServiceClient::with_interceptor(channel, auth)
}

async fn scene_client(
    mock: &mut MockInstance,
) -> SceneManagementServiceClient<
    tonic::service::interceptor::InterceptedService<Channel, CookieAuth>,
> {
    let (channel, auth) = authenticated(mock).await;
    SceneManagementServiceClient::with_interceptor(channel, auth)
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
        .await
        .expect_err("check must fail");
    assert_eq!(status.code(), tonic::Code::Internal);
}

#[tokio::test]
async fn package_failure_is_not_masked_by_available_firmware() {
    let mut mock = spawn_mock(r#"{"firmware": "available", "packages": "fetch-failed"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let status = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
async fn failed_firmware_apply_leaves_no_pending_install_handoff() {
    let mut mock = spawn_mock(
        r#"{"firmware": "available", "packages": "unavailable", "run": "apply-fail", "shadowed_packages": ["widget-flip-clock"]}"#,
    );
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec!["widget-flip-clock".to_owned()],
        })
        .await
        .expect("BUG: check failed")
        .into_inner();
    let upgrade_id = response.upgrade_id.expect("BUG: upgrade id");

    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();

    let (_, error) = drain_until_error(&mut stream).await;
    assert_eq!(error.code(), tonic::Code::Internal);

    // The failed firmware run must not leave a pending-install handoff on
    // disk. A later, unrelated successful firmware upgrade that requested no
    // install would otherwise consume it and install widgets nobody asked for.
    let handoff = mock.mockfs.join("tmp/bmc-nix-pending-install.json");
    assert!(
        !handoff.exists(),
        "stale pending-install handoff left after a failed firmware upgrade: {}",
        handoff.display()
    );

    // The requested widget stays shadowed (uninstalled) after the failed run.
    let after = client
        .list_installable_widgets(())
        .await
        .expect("BUG: list failed")
        .into_inner();
    assert!(
        after
            .widgets
            .iter()
            .any(|w| w.package_name == "widget-flip-clock"),
        "flip-clock must remain installable after a failed firmware upgrade"
    );
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
        .await
        .expect("BUG: check failed")
        .into_inner();
    assert!(response.firmware.is_none());
    assert!(response.upgrade_id.is_none());
}

const SHADOWED_SCENARIO: &str = r#"{"firmware": "up-to-date", "packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#;

#[tokio::test]
async fn lists_shadowed_widget_as_installable_over_grpc() {
    let mut mock = spawn_mock(SHADOWED_SCENARIO);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .list_installable_widgets(())
        .await
        .expect("BUG: list failed")
        .into_inner();
    let widget = response
        .widgets
        .iter()
        .find(|w| w.package_name == "widget-flip-clock")
        .expect("BUG: flip-clock not offered as installable");
    assert!(!widget.uid.is_empty());
    assert!(!widget.display_name.is_empty());
    assert!(widget.icon.is_some());
}

// A real index carrying two widgets; only the shadowed one is installable.
// The uids are distinct from the fabricated fallback's ("flip-clock"), so a
// regression that ignored --package-index would surface the wrong uid. The
// icon path does not exist, so it drops to None rather than failing the call.
const REAL_INDEX_JSON: &str = r#"{
  "version": 1,
  "provenance": null,
  "indexes": [],
  "caches": [],
  "packages": [
    {"name": "widget-flip-clock", "version": "2.5.0", "store_path": "/nix/store/x",
     "category": "widget",
     "metadata": {"widget": {"uid": "real-flip-uid", "display_name": "Real Flip Clock", "category": "clock"},
                  "assets": {"icon": "/nonexistent/icon.svg"}}},
    {"name": "widget-weather", "version": "1.9.0", "store_path": "/nix/store/y",
     "category": "widget",
     "metadata": {"widget": {"uid": "real-weather-uid", "display_name": "Real Weather"}}}
  ]
}"#;

#[tokio::test]
async fn serves_real_package_index_over_grpc() {
    let scenario = r#"{"firmware": "up-to-date", "packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#;
    let mut mock = spawn_mock_with_index(scenario, REAL_INDEX_JSON);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .list_installable_widgets(())
        .await
        .expect("BUG: list failed")
        .into_inner();
    // Only the shadowed widget, carrying the REAL uid from the index — proving
    // the index-backed path served it (not the fabricated fallback), that the
    // shadow filter excluded the non-shadowed weather, and that an unreadable
    // icon drops to None instead of failing discovery.
    assert_eq!(response.widgets.len(), 1, "widgets: {:?}", response.widgets);
    let widget = &response.widgets[0];
    assert_eq!(widget.package_name, "widget-flip-clock");
    assert_eq!(widget.uid, "real-flip-uid");
    assert_eq!(widget.display_name, "Real Flip Clock");
    // The index's "clock" string is mapped to the known enum value over the wire.
    assert_eq!(widget.category, i32::from(WidgetCategory::Clock));
    assert!(widget.icon.is_none());
}

// An index category this build does not recognize must not break discovery.
// A newer release may add categories; the tolerant deserializer folds the
// unknown string to Unknown, which crosses the wire as UNSPECIFIED.
const UNKNOWN_CATEGORY_INDEX_JSON: &str = r#"{
  "version": 1,
  "provenance": null,
  "indexes": [],
  "caches": [],
  "packages": [
    {"name": "widget-flip-clock", "version": "2.5.0", "store_path": "/nix/store/x",
     "category": "widget",
     "metadata": {"widget": {"uid": "u", "display_name": "Flip", "category": "teleportation"}}}
  ]
}"#;

#[tokio::test]
async fn unknown_index_category_serves_as_unspecified_over_grpc() {
    let scenario = r#"{"firmware": "up-to-date", "packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#;
    let mut mock = spawn_mock_with_index(scenario, UNKNOWN_CATEGORY_INDEX_JSON);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .list_installable_widgets(())
        .await
        .expect("BUG: list failed")
        .into_inner();
    assert_eq!(response.widgets.len(), 1, "widgets: {:?}", response.widgets);
    // The unrecognized "teleportation" category folds to UNSPECIFIED rather
    // than failing the listing — one new category cannot break discovery.
    assert_eq!(
        response.widgets[0].category,
        i32::from(WidgetCategory::Unspecified),
        "unknown index category must serve as UNSPECIFIED"
    );
}

#[tokio::test]
async fn check_surfaces_requested_install_over_grpc() {
    let mut mock = spawn_mock(SHADOWED_SCENARIO);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec!["widget-flip-clock".to_owned()],
        })
        .await
        .expect("BUG: check failed")
        .into_inner();
    let packages = response.packages.expect("BUG: packages offered");
    assert!(
        packages
            .changes
            .iter()
            .any(|c| c.name == "widget-flip-clock"),
        "requested install missing from plan: {:?}",
        packages.changes
    );
}

#[tokio::test]
async fn install_run_marks_widget_installed_over_grpc() {
    let mut mock = spawn_mock(SHADOWED_SCENARIO);
    let mut client = upgrade_client(&mut mock).await;

    let before = client
        .list_installable_widgets(())
        .await
        .expect("BUG: list failed")
        .into_inner();
    assert!(
        before
            .widgets
            .iter()
            .any(|w| w.package_name == "widget-flip-clock"),
        "flip-clock should be installable before the run"
    );

    let response = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec!["widget-flip-clock".to_owned()],
        })
        .await
        .expect("BUG: check failed")
        .into_inner();
    let upgrade_id = response.upgrade_id.expect("BUG: upgrade id");

    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();

    let mut phases = Vec::new();
    let mut finished = false;
    tokio::time::timeout(STREAM_TIMEOUT, async {
        while let Some(progress) = stream.message().await.expect("BUG: stream errored") {
            match progress.event.expect("BUG: event set") {
                upgrade_progress::Event::PackagePhase(phase) => phases.push(phase),
                upgrade_progress::Event::Finished(()) => finished = true,
                upgrade_progress::Event::Download(_)
                | upgrade_progress::Event::FirmwarePhase(_) => {}
            }
        }
    })
    .await
    .expect("stream did not finish within the timeout");
    assert!(finished, "install run did not finish");
    assert_eq!(
        phases,
        vec![
            PackageUpgradePhase::Realizing as i32,
            PackageUpgradePhase::Verifying as i32,
            PackageUpgradePhase::Building as i32,
            PackageUpgradePhase::Activating as i32,
        ]
    );

    let after = client
        .list_installable_widgets(())
        .await
        .expect("BUG: list failed")
        .into_inner();
    assert!(
        !after
            .widgets
            .iter()
            .any(|w| w.package_name == "widget-flip-clock"),
        "flip-clock should be unshadowed (installed) and no longer offered after the run"
    );
}

#[tokio::test]
async fn installed_widget_becomes_available_without_restart() {
    let mut mock = spawn_mock(SHADOWED_SCENARIO);
    let mut scenes = scene_client(&mut mock).await;

    // A shadowed widget is offered as installable but is not yet in the
    // Add-a-widget list: the registry discovers only the staged, non-shadowed
    // widgets.
    let before = scenes
        .get_available_widgets(())
        .await
        .expect("BUG: list available failed")
        .into_inner();
    assert!(
        !before.widgets.iter().any(|w| w.name == "Flip Clock"),
        "shadowed widget must not be available before install: {:?}",
        before.widgets
    );

    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec!["widget-flip-clock".to_owned()],
        })
        .await
        .expect("BUG: check failed")
        .into_inner();
    let upgrade_id = response.upgrade_id.expect("BUG: upgrade id");
    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();
    let mut finished = false;
    tokio::time::timeout(STREAM_TIMEOUT, async {
        while let Some(progress) = stream.message().await.expect("BUG: stream errored") {
            if matches!(progress.event, Some(upgrade_progress::Event::Finished(()))) {
                finished = true;
            }
        }
    })
    .await
    .expect("stream did not finish within the timeout");
    assert!(finished, "install run did not finish");

    // Without restarting the mock, the just-installed widget appears in the
    // Add-a-widget list: the completed run re-staged the tree and refreshed
    // the registry in-process.
    let after = scenes
        .get_available_widgets(())
        .await
        .expect("BUG: list available failed")
        .into_inner();
    assert!(
        after.widgets.iter().any(|w| w.name == "Flip Clock"),
        "installed widget must become available without a restart: {:?}",
        after.widgets
    );
}
