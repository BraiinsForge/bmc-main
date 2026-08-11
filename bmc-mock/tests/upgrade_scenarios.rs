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

//! End-to-end upgrade-scenario tests over gRPC. The FE cannot exercise
//! these yet (its generated protos predate the new upgrade.proto), so the
//! wire contract is verified here directly.

mod common;

use std::time::Duration;

use bmc_grpc::web::scene_management_service_client::SceneManagementServiceClient;
use bmc_grpc::web::upgrade_service_client::UpgradeServiceClient;
use bmc_grpc::web::{
    CheckForUpgradeRequest, FirmwareUpgradePhase, PackageUpgradePhase, StartUpgradeRequest,
    UpgradeDisruption, UpgradeProgress, WidgetCategory, WidgetSize, upgrade_progress,
};
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

use common::{MockInstance, spawn_mock};

const STREAM_TIMEOUT: Duration = Duration::from_mins(1);

fn spawn_mock_with_index(scenario_json: &str, index_json: &str) -> MockInstance {
    common::spawn_mock_inner(scenario_json, Some(index_json), true)
}

fn spawn_mock_realistic(scenario_json: &str) -> MockInstance {
    common::spawn_mock_inner(scenario_json, None, false)
}

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

async fn scene_client(
    mock: &mut MockInstance,
) -> SceneManagementServiceClient<
    tonic::service::interceptor::InterceptedService<Channel, CookieAuth>,
> {
    let (channel, cookie) = common::authenticated_channel(mock).await;
    SceneManagementServiceClient::with_interceptor(channel, CookieAuth(cookie))
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
    // Firmware is available, so the check skips the size estimate: the
    // packages preview carries no download size.
    assert!(packages.download_size_bytes.is_none());
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
async fn precondition_failure_surfaces_failed_precondition() {
    let mut mock = spawn_mock(r#"{"firmware": "up-to-date", "packages": "precondition-failed"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let status = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
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
    // Packages-only: the check runs the size estimate, so the preview carries
    // a download size.
    let packages = response.packages.expect("BUG: packages offered");
    assert!(packages.download_size_bytes.is_some());
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

    // The default action is a no-op: without `package_action: restart` the mock
    // must keep running. Wait well past the 200ms instant shutdown delay so a
    // regression that started exiting on the default action would be caught.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        mock.child
            .try_wait()
            .expect("BUG: try_wait failed")
            .is_none(),
        "the default package action must not stop the app"
    );
}

#[tokio::test]
async fn packages_restart_action_stops_the_app_after_activation() {
    let mut mock = spawn_mock(
        r#"{"firmware": "up-to-date", "packages": "available", "package_action": "restart"}"#,
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

    let mut firmware_phases = Vec::new();
    let mut package_phases = Vec::new();
    let mut downloads = 0;
    tokio::time::timeout(Duration::from_mins(2), async {
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
async fn firmware_success_closes_the_stream_cleanly_then_reboots() {
    let mut mock = spawn_mock(
        r#"{"firmware": "available", "packages": "available", "package_action": "restart"}"#,
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

    let deadline = Duration::from_mins(2);
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
async fn firmware_success_with_install_consumes_the_handoff() {
    // A firmware upgrade that also installs a widget: the run writes a
    // pending-install handoff, and the simulated sysupgrade consumes it
    // (unshadowing the widget) before the reboot-exit.
    let mut mock = spawn_mock(
        r#"{"firmware": "available", "packages": "unavailable", "shadowed_packages": ["widget-flip-clock"]}"#,
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

    tokio::time::timeout(Duration::from_mins(2), async {
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

    // Wait for the simulated reboot (process exit).
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

    // The handoff was consumed, not left behind.
    let handoff = mock.mockfs.join("dev/shm/bmc-nix-pending-install.json");
    assert!(
        !handoff.exists(),
        "consumed handoff must be removed: {}",
        handoff.display()
    );
    // Consuming the handoff unshadowed (installed) the widget.
    let scenario = std::fs::read_to_string(mock.mockfs.join("etc/upgrade-scenario.json"))
        .expect("BUG: read scenario");
    assert!(
        !scenario.contains("widget-flip-clock"),
        "installed widget must be unshadowed after the reboot: {scenario}"
    );
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
    let handoff = mock.mockfs.join("dev/shm/bmc-nix-pending-install.json");
    assert!(
        !handoff.exists(),
        "stale pending-install handoff left after a failed firmware upgrade: {}",
        handoff.display()
    );

    // The requested widget stays shadowed (uninstalled) after the failed run.
    let after = client
        .get_installable_widgets(())
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
async fn second_check_evicts_the_first_upgrade_id() {
    let mut mock = spawn_mock(r#"{"firmware": "up-to-date", "packages": "available"}"#);
    let mut client = upgrade_client(&mut mock).await;

    let first = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
        .await
        .expect("BUG: first check failed")
        .into_inner()
        .upgrade_id
        .expect("BUG: first check must offer an id");

    // A second check clears the prior offer set, so the first id is gone.
    client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
        .await
        .expect("BUG: second check failed");

    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id: first })
        .await
        .expect("BUG: start failed")
        .into_inner();
    let (events, error) = drain_until_error(&mut stream).await;
    assert!(
        events.is_empty(),
        "an evicted id must fail before any progress event"
    );
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
}

#[tokio::test]
async fn check_during_a_running_upgrade_reports_in_progress() {
    let mut mock = spawn_mock_realistic(r#"{"firmware": "up-to-date", "packages": "available"}"#);
    let mut client = upgrade_client(&mut mock).await;
    let mut concurrent = client.clone();

    let upgrade_id = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
        .await
        .expect("BUG: check failed")
        .into_inner()
        .upgrade_id
        .expect("BUG: upgrade id");
    let mut stream = client
        .start_upgrade(StartUpgradeRequest { upgrade_id })
        .await
        .expect("BUG: start failed")
        .into_inner();
    let first = stream
        .message()
        .await
        .expect("BUG: stream errored before first event")
        .expect("BUG: stream ended before first event");
    assert!(first.event.is_some(), "expected a progress event");

    let status = concurrent
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec![],
        })
        .await
        .expect_err("a check during a running upgrade must be refused");
    assert_eq!(status.code(), tonic::Code::Unavailable);
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
async fn lists_shadowed_widgets_with_supported_sizes_over_grpc() {
    let mut mock = spawn_mock(MULTI_SHADOWED_SCENARIO);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .get_installable_widgets(())
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
    assert_eq!(
        widget.supported_sizes,
        vec![
            WidgetSize::Small as i32,
            WidgetSize::Medium as i32,
            WidgetSize::Large as i32,
            WidgetSize::Full as i32,
        ],
        "manifest constraints must become BMC100 sizes over gRPC"
    );
    // The preview set crosses the wire intact: each entry carries a scene
    // size and a renderable image.
    assert!(!widget.previews.is_empty(), "previews must cross the wire");
    assert!(
        widget
            .previews
            .iter()
            .all(|p| !p.size.is_empty() && !p.image.is_empty()),
        "each preview needs a size and image: {:?}",
        widget.previews
    );

    let weather = response
        .widgets
        .iter()
        .find(|w| w.package_name == "widget-weather")
        .expect("BUG: weather not offered as installable");
    assert_eq!(
        weather.supported_sizes,
        vec![
            WidgetSize::Medium as i32,
            WidgetSize::Large as i32,
            WidgetSize::Full as i32,
        ],
        "weather manifest constraints must exclude the BMC100 small size"
    );
}

const REAL_INDEX_JSON: &str = r#"{
  "version": 1, "provenance": null, "indexes": [], "caches": [],
  "packages": [
    {"name": "widget-flip-clock", "version": "2.5.0", "store_path": "/nix/store/x",
     "category": "widget", "metadata": {"widget": {"uid": "real-flip-uid",
     "display_name": "Real Flip Clock", "category": "clock"},
     "assets": {"icon": "/nonexistent/icon.svg"}}},
    {"name": "widget-weather", "version": "1.9.0", "store_path": "/nix/store/y",
     "category": "widget", "metadata": {"widget": {"uid": "real-weather-uid",
     "display_name": "Real Weather"}}}
  ]
}"#;

#[tokio::test]
async fn serves_real_package_index_over_grpc() {
    let scenario = r#"{"firmware": "up-to-date", "packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#;
    let mut mock = spawn_mock_with_index(scenario, REAL_INDEX_JSON);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .get_installable_widgets(())
        .await
        .expect("BUG: list failed")
        .into_inner();
    assert_eq!(response.widgets.len(), 1, "widgets: {:?}", response.widgets);
    let widget = &response.widgets[0];
    assert_eq!(widget.package_name, "widget-flip-clock");
    assert_eq!(widget.uid, "real-flip-uid");
    assert_eq!(widget.display_name, "Real Flip Clock");
    assert_eq!(widget.category, i32::from(WidgetCategory::Clock));
    assert!(widget.icon.is_none());
    assert!(
        widget.supported_sizes.is_empty(),
        "legacy index metadata without viewport constraints must fit nowhere"
    );
}

const UNKNOWN_CATEGORY_INDEX_JSON: &str = r#"{
  "version": 1, "provenance": null, "indexes": [], "caches": [],
  "packages": [{"name": "widget-flip-clock", "version": "2.5.0",
    "store_path": "/nix/store/x", "category": "widget",
    "metadata": {"widget": {"uid": "u", "display_name": "Flip",
    "category": "teleportation"}}}]
}"#;

#[tokio::test]
async fn unknown_index_category_serves_as_unspecified_over_grpc() {
    let scenario = r#"{"firmware": "up-to-date", "packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#;
    let mut mock = spawn_mock_with_index(scenario, UNKNOWN_CATEGORY_INDEX_JSON);
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .get_installable_widgets(())
        .await
        .expect("BUG: list failed")
        .into_inner();
    assert_eq!(response.widgets.len(), 1, "widgets: {:?}", response.widgets);
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
async fn unknown_install_target_is_rejected_over_grpc() {
    let mut mock = spawn_mock(SHADOWED_SCENARIO);
    let mut client = upgrade_client(&mut mock).await;
    // widget-nope is not in the catalog (only widget-flip-clock is shadowed),
    // so the check must fail loud rather than silently offer a doomed install.
    let status = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec!["widget-nope".to_owned()],
        })
        .await
        .expect_err("an unknown install target must surface as a gRPC error");
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    assert!(
        status
            .message()
            .contains("requested package to install is unavailable"),
        "unexpected message: {}",
        status.message()
    );
}

#[tokio::test]
async fn install_when_up_to_date_offers_the_widget() {
    // Firmware and packages are both up to date; only the explicit install
    // drives the plan.
    let mut mock = spawn_mock(
        r#"{"firmware": "up-to-date", "packages": "unavailable", "shadowed_packages": ["widget-flip-clock"]}"#,
    );
    let mut client = upgrade_client(&mut mock).await;
    let response = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec!["widget-flip-clock".to_owned()],
        })
        .await
        .expect("BUG: check failed")
        .into_inner();
    assert!(
        response.upgrade_id.is_some(),
        "an install request must yield an upgrade id even when up to date"
    );
    assert_eq!(
        response.disruption,
        UpgradeDisruption::AppRestart as i32,
        "an install-only upgrade only restarts the app"
    );
    let packages = response.packages.expect("BUG: packages offered");
    assert_eq!(
        packages.changes.len(),
        1,
        "the plan must be exactly the installed widget: {:?}",
        packages.changes
    );
    assert_eq!(packages.changes[0].name, "widget-flip-clock");
}

const MULTI_SHADOWED_SCENARIO: &str = r#"{"firmware": "up-to-date", "packages": "available", "shadowed_packages": ["widget-flip-clock", "widget-weather"]}"#;

#[tokio::test]
async fn multi_package_install_marks_all_installed_over_grpc() {
    let mut mock = spawn_mock(MULTI_SHADOWED_SCENARIO);
    let mut client = upgrade_client(&mut mock).await;

    let before = client
        .get_installable_widgets(())
        .await
        .expect("BUG: list failed")
        .into_inner();
    assert!(
        before
            .widgets
            .iter()
            .any(|w| w.package_name == "widget-flip-clock")
    );
    assert!(
        before
            .widgets
            .iter()
            .any(|w| w.package_name == "widget-weather")
    );

    let response = client
        .check_for_upgrade(CheckForUpgradeRequest {
            install_packages: vec!["widget-flip-clock".to_owned(), "widget-weather".to_owned()],
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
    assert!(finished, "multi-install run did not finish");

    let after = client
        .get_installable_widgets(())
        .await
        .expect("BUG: list failed")
        .into_inner();
    assert!(
        !after
            .widgets
            .iter()
            .any(|w| w.package_name == "widget-flip-clock"),
        "flip-clock must be installed and no longer offered"
    );
    assert!(
        !after
            .widgets
            .iter()
            .any(|w| w.package_name == "widget-weather"),
        "weather must be installed and no longer offered"
    );
}

#[tokio::test]
async fn install_run_marks_widget_installed_over_grpc() {
    let mut mock = spawn_mock(SHADOWED_SCENARIO);
    let mut client = upgrade_client(&mut mock).await;

    let before = client
        .get_installable_widgets(())
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
        .get_installable_widgets(())
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
