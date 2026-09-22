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

use super::{Projection, UPGRADE_OUTAGE_GRACE, spawn_observer};
use crate::boser::{StateSink, StreamConfig, Timing};
use crate::compositor::{
    DownloadProgress, UpgradeDisplaySnapshot, UpgradeDisplayState, UpgradeGeneration, UpgradeKind,
    UpgradePhase,
};
use crate::system_upgrade::{DisplayStateService, StateService, SystemUpgradeState};
use axum::Router;
use axum::body::Body;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use bmc_upgrade_types::{
    ExecutionId, FirmwarePhase, PackagePhase, UpgradePhase as WirePhase, UpgradeState,
};
use futures::{StreamExt, stream};
use std::convert::Infallible;
use std::future::IntoFuture;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::sync::watch;

struct Bench {
    projection: Projection,
    display_watch: watch::Receiver<Option<UpgradeDisplaySnapshot>>,
    state_watch: watch::Receiver<Option<SystemUpgradeState>>,
}

fn bench() -> Bench {
    let display = DisplayStateService::new();
    let state = StateService::new();
    let display_watch = display.subscribe();
    let state_watch = state.subscribe();
    Bench {
        projection: Projection::new(display, state),
        display_watch,
        state_watch,
    }
}

impl Bench {
    fn observe(&mut self, response: &UpgradeState) {
        self.projection.observe(response);
    }

    fn display(&self) -> Option<UpgradeDisplaySnapshot> {
        self.display_watch.borrow().clone()
    }

    fn display_state(&self) -> Option<UpgradeDisplayState> {
        self.display().map(|snapshot| snapshot.state)
    }

    fn generation(&self) -> UpgradeGeneration {
        self.display()
            .expect("BUG: the test expects a presentation")
            .generation
    }

    fn state(&self) -> Option<SystemUpgradeState> {
        self.state_watch.borrow().clone()
    }
}

fn running(id: ExecutionId, phase: WirePhase) -> UpgradeState {
    running_with_kind(id, UpgradeKind::Packages, phase)
}

fn running_with_kind(id: ExecutionId, kind: UpgradeKind, phase: WirePhase) -> UpgradeState {
    UpgradeState::Running {
        id,
        kind,
        phase,
        download: None,
    }
}

fn completed(id: ExecutionId) -> UpgradeState {
    UpgradeState::Completed {
        id,
        kind: UpgradeKind::Packages,
    }
}

fn failed(id: ExecutionId) -> UpgradeState {
    UpgradeState::Failed {
        id,
        kind: UpgradeKind::Packages,
        phase: WirePhase::Packages(PackagePhase::Building),
        reason: "build failed".to_owned(),
    }
}

fn downloading(downloaded_bytes: u64) -> UpgradeState {
    UpgradeState::DownloadingImage {
        download: DownloadProgress {
            downloaded_bytes,
            total_bytes: Some(100),
        },
    }
}

fn download_failed() -> UpgradeState {
    UpgradeState::DownloadFailed {
        reason: "checksum mismatch".to_owned(),
    }
}

fn package_running(phase: UpgradePhase) -> UpgradeDisplayState {
    UpgradeDisplayState::Running {
        kind: UpgradeKind::Packages,
        phase: Some(phase),
        progress: None,
    }
}

fn firmware_downloading(downloaded_bytes: u64) -> UpgradeDisplayState {
    UpgradeDisplayState::Running {
        kind: UpgradeKind::Firmware,
        phase: Some(UpgradePhase::FirmwareDownloading),
        progress: Some(DownloadProgress {
            downloaded_bytes,
            total_bytes: Some(100),
        }),
    }
}

#[test]
fn the_same_execution_reuses_its_generation() {
    let mut bench = bench();
    let id = ExecutionId::new();
    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Realizing)));
    let generation = bench.generation();

    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Building)));

    // The compositor keys its terminal deadline per generation, so one execution
    // must present under one generation from start to end.
    assert_eq!(bench.generation(), generation);
    assert_eq!(
        bench.display_state(),
        Some(package_running(UpgradePhase::PackageBuilding))
    );
    assert_eq!(bench.state(), Some(SystemUpgradeState::UpgradeStarted));
}

#[test]
fn a_terminal_continuing_the_current_execution_presents_and_unblocks() {
    let mut bench = bench();
    let id = ExecutionId::new();
    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Building)));
    let generation = bench.generation();

    bench.observe(&completed(id));

    assert_eq!(bench.generation(), generation);
    assert_eq!(
        bench.display_state(),
        Some(UpgradeDisplayState::Succeeded {
            kind: UpgradeKind::Packages
        })
    );
    let state = bench.state().expect("BUG: a terminal notifies the state");
    assert_eq!(state, SystemUpgradeState::Finished);
    assert!(!state.blocks_restart());
}

#[test]
fn a_failed_terminal_continuing_the_current_execution_presents_the_failure() {
    let mut bench = bench();
    let id = ExecutionId::new();
    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Building)));
    let generation = bench.generation();

    bench.observe(&failed(id));

    assert_eq!(bench.generation(), generation);
    assert_eq!(
        bench.display_state(),
        Some(UpgradeDisplayState::Failed {
            kind: UpgradeKind::Packages
        })
    );
    let state = bench.state().expect("BUG: a terminal notifies the state");
    assert_eq!(state, SystemUpgradeState::Failed);
    assert!(!state.blocks_restart());
}

#[test]
fn a_stale_terminal_at_boot_is_ignored() {
    let mut bench = bench();

    bench.observe(&completed(ExecutionId::new()));

    // A terminal is presented only when it continues an execution seen running;
    // Boser keeps the last terminal on its stream for hours.
    assert_eq!(bench.display(), None);
    assert_eq!(bench.state(), None);
}

#[test]
fn download_failed_counts_only_after_downloading() {
    let mut bench = bench();
    bench.observe(&download_failed());
    assert_eq!(bench.display(), None);

    bench.observe(&downloading(10));
    let generation = bench.generation();
    bench.observe(&download_failed());

    assert_eq!(bench.generation(), generation);
    assert_eq!(
        bench.display_state(),
        Some(UpgradeDisplayState::Failed {
            kind: UpgradeKind::Firmware
        })
    );
    assert_eq!(bench.state(), Some(SystemUpgradeState::Failed));
}

#[test]
fn a_retried_download_gets_a_fresh_generation() {
    let mut bench = bench();
    bench.observe(&downloading(10));
    let first = bench.generation();
    bench.observe(&download_failed());

    bench.observe(&downloading(0));

    assert_ne!(bench.generation(), first);
    assert_eq!(bench.display_state(), Some(firmware_downloading(0)));
    assert_eq!(bench.state(), Some(SystemUpgradeState::UpgradeStarted));
}

#[test]
fn none_mid_run_clears_the_display_and_releases_restart() {
    let mut bench = bench();
    bench.observe(&running(
        ExecutionId::new(),
        WirePhase::Packages(PackagePhase::Building),
    ));

    bench.observe(&UpgradeState::None);

    // Boser restarted mid-run: the outcome is unknown, so nothing is shown
    // and the restart block is released.
    assert_eq!(bench.display(), None);
    assert_eq!(bench.state(), None);
}

#[test]
fn a_legacy_download_ending_in_none_clears_without_a_failure() {
    let mut bench = bench();
    bench.observe(&downloading(100));

    bench.observe(&UpgradeState::None);

    // The legacy BOS+ download reports success as a bare `None`;
    // a failure presentation here would lie.
    assert_eq!(bench.display(), None);
    assert_eq!(bench.state(), None);
}

#[test]
fn the_legacy_sequence_without_an_observed_none_ends_as_an_apply_presentation() {
    let mut bench = bench();
    bench.observe(&downloading(100));
    let download = bench.generation();

    bench.observe(&UpgradeState::Running {
        id: ExecutionId::new(),
        kind: UpgradeKind::Firmware,
        phase: WirePhase::Firmware(FirmwarePhase::Flashing),
        download: None,
    });

    // Fast transitions coalesce on Boser's watch;
    // the apply is a new execution and must not extend the download's presentation.
    assert_ne!(bench.generation(), download);
    assert_eq!(
        bench.display_state(),
        Some(UpgradeDisplayState::Running {
            kind: UpgradeKind::Firmware,
            phase: Some(UpgradePhase::FirmwareApplying),
            progress: None,
        })
    );
}

#[test]
fn a_running_for_another_execution_replaces_the_presentation() {
    let mut bench = bench();
    bench.observe(&running(
        ExecutionId::new(),
        WirePhase::Packages(PackagePhase::Building),
    ));
    let first = bench.generation();

    bench.observe(&running(
        ExecutionId::new(),
        WirePhase::Packages(PackagePhase::Activating),
    ));

    assert_ne!(bench.generation(), first);
    assert_eq!(
        bench.display_state(),
        Some(package_running(UpgradePhase::PackageActivating))
    );
    assert_eq!(bench.state(), Some(SystemUpgradeState::UpgradeStarted));
}

#[test]
fn an_idle_none_is_ignored() {
    let mut bench = bench();
    bench.observe(&UpgradeState::None);
    assert_eq!(bench.display(), None);

    let id = ExecutionId::new();
    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Building)));
    bench.observe(&completed(id));
    let shown = bench.display();
    bench.observe(&UpgradeState::None);

    // A terminal already handed to the compositor stays until its deadline;
    // Boser's idle state after it is not an ending.
    assert_eq!(bench.display(), shown);
    assert_eq!(bench.state(), Some(SystemUpgradeState::Finished));
}

#[test]
fn every_wire_phase_maps_to_its_display_phase() {
    let expectations = [
        (UpgradeKind::Packages, WirePhase::Preparing, None),
        (
            UpgradeKind::Firmware,
            WirePhase::Firmware(FirmwarePhase::Downloading),
            Some(UpgradePhase::FirmwareDownloading),
        ),
        (
            UpgradeKind::Firmware,
            WirePhase::Firmware(FirmwarePhase::Verifying),
            Some(UpgradePhase::FirmwareVerifying),
        ),
        (
            UpgradeKind::Firmware,
            WirePhase::Firmware(FirmwarePhase::Flashing),
            Some(UpgradePhase::FirmwareApplying),
        ),
        (
            UpgradeKind::Packages,
            WirePhase::Packages(PackagePhase::Realizing),
            Some(UpgradePhase::PackageRealizing),
        ),
        (
            UpgradeKind::Packages,
            WirePhase::Packages(PackagePhase::Verifying),
            Some(UpgradePhase::PackageVerifying),
        ),
        (
            UpgradeKind::Packages,
            WirePhase::Packages(PackagePhase::Building),
            Some(UpgradePhase::PackageBuilding),
        ),
        (
            UpgradeKind::Packages,
            WirePhase::Packages(PackagePhase::Activating),
            Some(UpgradePhase::PackageActivating),
        ),
        (
            UpgradeKind::Packages,
            WirePhase::Packages(PackagePhase::Cleaning),
            None,
        ),
        (
            UpgradeKind::Packages,
            WirePhase::Packages(PackagePhase::FindingGarbageRoots),
            None,
        ),
        (
            UpgradeKind::Packages,
            WirePhase::Packages(PackagePhase::DeterminingGarbageLiveness),
            None,
        ),
        (UpgradeKind::Firmware, WirePhase::Unknown, None),
        (
            UpgradeKind::Firmware,
            WirePhase::Firmware(FirmwarePhase::Unknown),
            None,
        ),
        (
            UpgradeKind::Packages,
            WirePhase::Packages(PackagePhase::Unknown),
            None,
        ),
    ];

    for (kind, wire, expected) in expectations {
        let mut bench = bench();
        bench.observe(&running_with_kind(ExecutionId::new(), kind, wire));
        let Some(UpgradeDisplayState::Running { phase, .. }) = bench.display_state() else {
            panic!("{wire:?} must present as running");
        };
        assert_eq!(phase, expected, "{wire:?}");
        assert!(
            bench.state().is_some_and(|state| state.blocks_restart()),
            "{wire:?} must keep restart blocked"
        );
    }
}

#[test]
fn a_reboot_presents_the_execution_kind_as_applying() {
    let mut bench = bench();

    bench.observe(&UpgradeState::Rebooting {
        id: ExecutionId::new(),
        kind: UpgradeKind::FirmwareAndPackages,
    });

    // The kind passes through unchanged;
    // how a combined upgrade is drawn is the compositor's call.
    assert_eq!(
        bench.display_state(),
        Some(UpgradeDisplayState::Running {
            kind: UpgradeKind::FirmwareAndPackages,
            phase: Some(UpgradePhase::FirmwareApplying),
            progress: None,
        })
    );
}

#[tokio::test(start_paused = true)]
async fn stream_loss_holds_restart_and_display_until_the_grace_expires() {
    let mut bench = bench();
    bench.observe(&running(
        ExecutionId::new(),
        WirePhase::Packages(PackagePhase::Building),
    ));
    let shown = bench.display();

    bench.projection.stream_lost();

    assert_eq!(bench.state(), Some(SystemUpgradeState::UpgradeStarted));
    assert_eq!(bench.display(), shown.clone());

    tokio::time::advance(
        UPGRADE_OUTAGE_GRACE
            .checked_sub(Duration::from_nanos(1))
            .expect("BUG: the outage grace is longer than one nanosecond"),
    )
    .await;
    bench.projection.stream_lost();
    assert_eq!(bench.state(), Some(SystemUpgradeState::UpgradeStarted));
    assert_eq!(bench.display(), shown);

    tokio::time::advance(Duration::from_nanos(1)).await;
    bench.projection.stream_lost();
    assert_eq!(bench.state(), None);
    assert_eq!(bench.display(), None);
}

#[tokio::test(start_paused = true)]
async fn the_same_execution_replayed_during_the_grace_keeps_its_generation() {
    let mut bench = bench();
    let id = ExecutionId::new();
    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Building)));
    let generation = bench.generation();
    bench.projection.stream_lost();
    tokio::time::advance(UPGRADE_OUTAGE_GRACE / 2).await;

    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Building)));

    assert_eq!(bench.state(), Some(SystemUpgradeState::UpgradeStarted));
    assert_eq!(bench.generation(), generation);
}

#[tokio::test(start_paused = true)]
async fn a_recovered_execution_gets_a_fresh_grace_for_its_next_outage() {
    let mut bench = bench();
    let id = ExecutionId::new();
    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Building)));
    bench.projection.stream_lost();
    tokio::time::advance(
        UPGRADE_OUTAGE_GRACE
            .checked_sub(Duration::from_secs(1))
            .expect("BUG: the outage grace is longer than one second"),
    )
    .await;

    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Activating)));
    bench.projection.stream_lost();
    tokio::time::advance(Duration::from_secs(1)).await;
    bench.projection.stream_lost();

    assert_eq!(bench.state(), Some(SystemUpgradeState::UpgradeStarted));
    assert_eq!(
        bench.display_state(),
        Some(package_running(UpgradePhase::PackageActivating))
    );
}

#[tokio::test(start_paused = true)]
async fn an_expired_execution_ignores_its_terminal_but_can_restart_running() {
    let mut bench = bench();
    let id = ExecutionId::new();
    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Building)));
    let generation = bench.generation();
    bench.projection.stream_lost();
    tokio::time::advance(UPGRADE_OUTAGE_GRACE).await;
    bench.projection.stream_lost();

    bench.observe(&completed(id));
    assert_eq!(bench.display(), None);
    assert_eq!(bench.state(), None);

    bench.observe(&running(id, WirePhase::Packages(PackagePhase::Building)));
    assert_ne!(bench.generation(), generation);
    assert_eq!(bench.state(), Some(SystemUpgradeState::UpgradeStarted));
}

#[tokio::test(start_paused = true)]
async fn losses_without_an_execution_do_not_shorten_a_later_grace() {
    let mut bench = bench();
    bench.projection.stream_lost();
    tokio::time::advance(UPGRADE_OUTAGE_GRACE).await;

    bench.observe(&running(
        ExecutionId::new(),
        WirePhase::Packages(PackagePhase::Building),
    ));
    let shown = bench.display();
    bench.projection.stream_lost();

    assert_eq!(bench.display(), shown);
    assert_eq!(bench.state(), Some(SystemUpgradeState::UpgradeStarted));
}

#[test]
fn a_contract_mismatch_ends_the_execution() {
    let mut bench = bench();
    bench.observe(&running(
        ExecutionId::new(),
        WirePhase::Packages(PackagePhase::Building),
    ));

    bench.projection.contract_mismatch();

    assert_eq!(bench.display(), None);
    assert_eq!(bench.state(), None);
}

#[tokio::test(start_paused = true)]
async fn a_contract_mismatch_drops_the_old_outage_deadline() {
    let mut bench = bench();
    bench.observe(&running(
        ExecutionId::new(),
        WirePhase::Packages(PackagePhase::Building),
    ));
    bench.projection.stream_lost();
    tokio::time::advance(UPGRADE_OUTAGE_GRACE).await;
    bench.projection.contract_mismatch();

    bench.observe(&running(
        ExecutionId::new(),
        WirePhase::Packages(PackagePhase::Building),
    ));
    let shown = bench.display();
    bench.projection.stream_lost();

    assert_eq!(bench.display(), shown);
    assert_eq!(bench.state(), Some(SystemUpgradeState::UpgradeStarted));
}

/// Serves one state on the observed path, then holds the connection open.
async fn serve(body: String) -> SocketAddr {
    let router = Router::new().route(
        Projection::PATH,
        get(move || {
            let body = body.clone();
            async move {
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    Body::from_stream(
                        stream::iter([body])
                            .chain(stream::pending())
                            .map(Ok::<_, Infallible>),
                    ),
                )
                    .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("BUG: a loopback listener binds");
    let address = listener
        .local_addr()
        .expect("BUG: a bound listener has an address");
    tokio::spawn(axum::serve(listener, router).into_future());
    address
}

/// The transport and the projection meet only here;
/// what the stream itself does is pinned in `crate::boser`'s own tests.
#[tokio::test]
async fn a_state_on_the_stream_reaches_the_display() {
    let dir = tempfile::tempdir().expect("BUG: test tempdir creation must succeed");
    let token_path = dir.path().join("token");
    std::fs::write(&token_path, "token").expect("BUG: the token file writes");
    let state = running(
        ExecutionId::new(),
        WirePhase::Packages(PackagePhase::Building),
    );
    let data = serde_json::to_string(&state).expect("BUG: wire states serialize");
    let address = serve(format!("data: {data}\n\n")).await;
    let display = DisplayStateService::new();
    let mut display_watch = display.subscribe();

    let observer = spawn_observer(
        StreamConfig {
            address,
            token_path,
            timing: Timing::default(),
        },
        display,
        StateService::new(),
    );

    let snapshot = tokio::time::timeout(
        Duration::from_secs(10),
        display_watch.wait_for(Option::is_some),
    )
    .await
    .expect("the display state must arrive in time")
    .expect("BUG: the display sender outlives the test")
    .clone()
    .expect("BUG: the predicate saw a snapshot");
    assert_eq!(
        snapshot.state,
        package_running(UpgradePhase::PackageBuilding)
    );
    observer.abort();
}
