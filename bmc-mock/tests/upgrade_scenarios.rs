// Copyright (C) 2026  Braiins Systems s.r.o.

//! End-to-end upgrade-scenario tests over gRPC. The FE cannot exercise
//! these yet (its generated protos predate the new upgrade.proto), so the
//! wire contract is verified here directly.

mod common;

use std::time::Duration;

use bmc_grpc::web::upgrade_service_client::UpgradeServiceClient;
use bmc_grpc::web::{
    FirmwareUpgradePhase, PackageUpgradePhase, StartUpgradeRequest, UpgradeDisruption,
    UpgradeProgress, upgrade_progress,
};
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

use common::{MockInstance, spawn_mock};

const STREAM_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct CookieAuth(MetadataValue<tonic::metadata::Ascii>);

impl tonic::service::Interceptor for CookieAuth {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, tonic::Status> {
        req.metadata_mut().insert("cookie", self.0.clone());
        Ok(req)
    }
}

async fn upgrade_client(
    mock: &mut MockInstance,
) -> UpgradeServiceClient<tonic::service::interceptor::InterceptedService<Channel, CookieAuth>> {
    let (channel, cookie) = common::authenticated_channel(mock).await;
    UpgradeServiceClient::with_interceptor(channel, CookieAuth(cookie))
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
async fn precondition_failure_surfaces_failed_precondition() {
    let mut mock = spawn_mock(r#"{"firmware": "up-to-date", "packages": "precondition-failed"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let status = client
        .check_for_upgrade(())
        .await
        .expect_err("a non-transient package precondition must surface as a gRPC error");
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
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
async fn packages_restart_action_stops_the_app_after_activation() {
    let mut mock = spawn_mock(
        r#"{"firmware": "up-to-date", "packages": "available", "package_action": "restart"}"#,
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
    assert!(finished, "the packages-only run must finish cleanly");
    assert!(
        phases.contains(&(PackageUpgradePhase::Activating as i32)),
        "the restart action must follow a completed activation"
    );

    // The restart action schedules an asynchronous application stop, so the
    // mock runs its graceful shutdown and exits successfully on its own.
    let exit_deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = mock.child.try_wait().expect("BUG: try_wait failed") {
            assert!(status.success(), "mock exited with failure: {status}");
            break;
        }
        assert!(
            std::time::Instant::now() < exit_deadline,
            "mock process did not exit after the restart action"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn firmware_with_packages_streams_nested_package_stages() {
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

    let mut firmware_phases = Vec::new();
    let mut package_phases = Vec::new();
    let mut downloads = 0;
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let progress = stream
                .message()
                .await
                .expect("BUG: stream errored before FirmwareApplying")
                .expect("BUG: stream ended before FirmwareApplying");
            match progress.event.expect("BUG: event set") {
                upgrade_progress::Event::FirmwarePhase(phase) => {
                    firmware_phases.push(phase);
                    if phase == FirmwareUpgradePhase::Applying as i32 {
                        return;
                    }
                }
                upgrade_progress::Event::PackagePhase(phase) => package_phases.push(phase),
                upgrade_progress::Event::Download(_) => downloads += 1,
                upgrade_progress::Event::Finished(()) => {}
            }
        }
    })
    .await
    .expect("timed out before FirmwareApplying");

    assert!(
        firmware_phases.contains(&(FirmwareUpgradePhase::Downloading as i32)),
        "the firmware pipeline must run its download phase"
    );
    assert!(
        downloads >= 2,
        "the client must receive firmware and package download progress"
    );
    assert_eq!(
        package_phases,
        vec![
            PackageUpgradePhase::Realizing as i32,
            PackageUpgradePhase::Verifying as i32,
            PackageUpgradePhase::Building as i32,
        ],
        "firmware sysupgrade must surface the nested package upgrade stages"
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
async fn firmware_success_closes_the_stream_cleanly_then_reboots() {
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

    // The run closes the stream cleanly right after Applying: the client sees
    // end-of-stream (Ok(None)) with an OK status, not more events and not a
    // transport error, and it arrives before the mock's reboot-exit.
    let outcome = tokio::time::timeout(Duration::from_secs(30), stream.message())
        .await
        .expect("stream did not close after FirmwareApplying");
    match outcome {
        Ok(None) => {}
        Ok(Some(event)) => panic!("unexpected event after Applying: {event:?}"),
        Err(status) => panic!("stream errored instead of closing cleanly: {status}"),
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
