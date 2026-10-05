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

use super::{FollowError, Followed, follow};
use crate::compositor::{
    DownloadProgress, UpgradeGeneration, UpgradeKind, UpgradePhase, UpgradeRunSnapshot,
    UpgradeRunStatus,
};
use crate::system_upgrade::{RunUpdates, UnseenOutcome};
use bmc_upgrade_types::ExecutionId;
use futures::future::FutureExt;
use futures::stream::{BoxStream, Fuse, StreamExt};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;

const SEEN_WITHIN: Duration = Duration::from_secs(30);
const SECOND: Duration = Duration::from_secs(1);

type Item = Result<Followed, FollowError>;

struct Bench {
    offer: ExecutionId,
    display: watch::Sender<Option<UpgradeRunSnapshot>>,
    unseen_outcomes: watch::Sender<Option<UnseenOutcome>>,
    stream: Fuse<BoxStream<'static, Item>>,
}

fn bench() -> Bench {
    let offer = ExecutionId::new();
    let (display, displayed) = watch::channel(None);
    let (unseen_outcomes, outcomes) = watch::channel(None);
    let updates = RunUpdates {
        display: displayed,
        unseen_outcomes: outcomes,
    };
    let stream = follow(offer, updates, Instant::now() + SEEN_WITHIN)
        .boxed()
        .fuse();
    Bench {
        offer,
        display,
        unseen_outcomes,
        stream,
    }
}

impl Bench {
    fn publish(&mut self, snapshot: Option<UpgradeRunSnapshot>) -> Vec<Item> {
        self.display.send_replace(snapshot);
        self.ready()
    }

    fn ready(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while let Some(Some(item)) = self.stream.next().now_or_never() {
            items.push(item);
        }
        items
    }

    fn unseen(&mut self, id: ExecutionId, result: Result<(), String>) -> Vec<Item> {
        self.unseen_outcomes
            .send_replace(Some(UnseenOutcome { id, result }));
        self.ready()
    }

    fn ours(&mut self, state: UpgradeRunStatus) -> Vec<Item> {
        self.publish(Some(snapshot(Some(self.offer), state)))
    }

    fn phase(&mut self, phase: UpgradePhase) -> Vec<Item> {
        self.ours(running(Some(phase), None))
    }

    fn ended(&mut self) -> bool {
        matches!(self.stream.next().now_or_never(), Some(None))
    }
}

fn snapshot(id: Option<ExecutionId>, state: UpgradeRunStatus) -> UpgradeRunSnapshot {
    UpgradeRunSnapshot {
        generation: UpgradeGeneration::new(0),
        id,
        state,
    }
}

fn running(phase: Option<UpgradePhase>, progress: Option<DownloadProgress>) -> UpgradeRunStatus {
    UpgradeRunStatus::Running {
        kind: UpgradeKind::Packages,
        phase,
        progress,
    }
}

fn foreign() -> UpgradeRunSnapshot {
    snapshot(
        Some(ExecutionId::new()),
        running(Some(UpgradePhase::PackageBuilding), None),
    )
}

fn downloaded(downloaded_bytes: u64) -> DownloadProgress {
    DownloadProgress {
        downloaded_bytes,
        total_bytes: Some(100),
    }
}

const REBOOTING: UpgradeRunStatus = UpgradeRunStatus::Rebooting {
    kind: UpgradeKind::Firmware,
};

const SUCCEEDED: UpgradeRunStatus = UpgradeRunStatus::Succeeded {
    kind: UpgradeKind::Packages,
};

fn failed(reason: &str) -> UpgradeRunStatus {
    UpgradeRunStatus::Failed {
        kind: UpgradeKind::Packages,
        reason: reason.to_owned(),
    }
}

const LOST: Item = Err(FollowError::Lost);

#[tokio::test]
async fn a_phase_is_emitted_once_until_it_changes() {
    let mut bench = bench();
    let phase = UpgradePhase::PackageRealizing;

    let first = bench.ours(running(Some(phase), Some(downloaded(10))));
    let second = bench.ours(running(Some(phase), Some(downloaded(20))));

    assert_eq!(
        first,
        [
            Ok(Followed::Phase(phase)),
            Ok(Followed::Download(downloaded(10)))
        ]
    );
    assert_eq!(
        second,
        [Ok(Followed::Download(downloaded(20)))],
        "progress within a phase must not announce the phase again"
    );
}

#[tokio::test]
async fn a_phase_that_recurs_after_another_is_emitted_again() {
    let mut bench = bench();

    let mut items = bench.phase(UpgradePhase::PackageVerifying);
    items.extend(bench.phase(UpgradePhase::PackageRealizing));
    items.extend(bench.phase(UpgradePhase::PackageVerifying));

    assert_eq!(
        items,
        [
            Ok(Followed::Phase(UpgradePhase::PackageVerifying)),
            Ok(Followed::Phase(UpgradePhase::PackageRealizing)),
            Ok(Followed::Phase(UpgradePhase::PackageVerifying)),
        ],
        "only an unchanged phase is a repeat"
    );
}

#[tokio::test]
async fn a_repeated_download_is_emitted_once() {
    let mut bench = bench();
    bench.ours(running(
        Some(UpgradePhase::PackageRealizing),
        Some(downloaded(10)),
    ));

    assert_eq!(
        bench.ours(running(
            Some(UpgradePhase::PackageRealizing),
            Some(downloaded(10)),
        )),
        [],
        "the same byte count in the same phase is not new download progress"
    );
}

#[tokio::test]
async fn a_new_phase_reports_its_download_again() {
    let mut bench = bench();
    bench.ours(running(
        Some(UpgradePhase::PackageRealizing),
        Some(downloaded(10)),
    ));

    let items = bench.ours(running(
        Some(UpgradePhase::PackageVerifying),
        Some(downloaded(10)),
    ));

    assert_eq!(
        items,
        [
            Ok(Followed::Phase(UpgradePhase::PackageVerifying)),
            Ok(Followed::Download(downloaded(10)))
        ],
        "a client drops the byte count on a new phase, so an equal one must be sent again"
    );
}

#[tokio::test]
async fn a_run_without_a_display_phase_counts_as_seen() {
    let mut bench = bench();

    assert_eq!(bench.ours(running(None, None)), []);

    assert_eq!(
        bench.publish(Some(foreign())),
        [LOST],
        "our run was on display, so another run replacing it means it is gone"
    );
}

#[tokio::test]
async fn a_download_without_a_display_phase_is_still_reported() {
    let mut bench = bench();

    assert_eq!(
        bench.ours(running(None, Some(downloaded(10)))),
        [Ok(Followed::Download(downloaded(10)))],
        "bytes arriving in a phase the display does not name are still progress"
    );
}

#[tokio::test]
async fn success_finishes_the_stream() {
    let mut bench = bench();
    bench.phase(UpgradePhase::PackageActivating);

    assert_eq!(bench.ours(SUCCEEDED), [Ok(Followed::Finished)]);
    assert!(bench.ended());
}

#[tokio::test]
async fn failure_carries_the_reason() {
    let mut bench = bench();
    bench.phase(UpgradePhase::PackageBuilding);

    assert_eq!(
        bench.ours(failed("disk full")),
        [Err(FollowError::Failed("disk full".to_owned()))],
        "the caller is told why its upgrade failed"
    );
    assert!(bench.ended());
}

/// Boser reports flashing before it stages the packages, which can still fail.
#[tokio::test]
async fn flashing_is_not_applying_while_the_run_can_still_fail() {
    let mut bench = bench();

    assert_eq!(bench.phase(UpgradePhase::FirmwareApplying), []);
    assert_eq!(
        bench.phase(UpgradePhase::PackageRealizing),
        [Ok(Followed::Phase(UpgradePhase::PackageRealizing))]
    );
    assert_eq!(
        bench.ours(failed("not enough space")),
        [Err(FollowError::Failed("not enough space".to_owned()))],
        "a failure after flashing was reported still reaches the caller"
    );
}

#[tokio::test]
async fn the_handoff_to_the_reboot_is_applying_and_ends_the_stream_cleanly() {
    let mut bench = bench();
    bench.phase(UpgradePhase::FirmwareApplying);

    assert_eq!(
        bench.ours(REBOOTING),
        [Ok(Followed::Phase(UpgradePhase::FirmwareApplying))]
    );
    assert!(
        bench.ended(),
        "the device reboots from here, so the stream ends without a verdict"
    );
}

#[tokio::test]
async fn a_handoff_seen_first_is_not_lost() {
    let mut bench = bench();

    assert_eq!(
        bench.ours(REBOOTING),
        [Ok(Followed::Phase(UpgradePhase::FirmwareApplying))]
    );
    assert!(bench.ended());
}

#[tokio::test]
async fn a_success_seen_first_is_not_lost() {
    let mut bench = bench();

    assert_eq!(
        bench.ours(SUCCEEDED),
        [Ok(Followed::Finished)],
        "a run too quick to be seen running still reports its outcome"
    );
}

/// A package run can end before the observer saw it running; its caller still has to learn how.
#[tokio::test]
async fn a_success_never_displayed_finishes_the_stream() {
    let mut bench = bench();

    assert_eq!(bench.unseen(bench.offer, Ok(())), [Ok(Followed::Finished)]);
    assert!(bench.ended());
}

#[tokio::test]
async fn a_failure_never_displayed_carries_the_reason() {
    let mut bench = bench();

    assert_eq!(
        bench.unseen(bench.offer, Err("no space".to_owned())),
        [Err(FollowError::Failed("no space".to_owned()))]
    );
}

/// Boser retains the last outcome for hours, so the one told may belong to an older run.
#[tokio::test]
async fn another_runs_undisplayed_outcome_is_skipped() {
    let mut bench = bench();

    assert_eq!(bench.unseen(ExecutionId::new(), Ok(())), []);
    assert_eq!(
        bench.phase(UpgradePhase::PackageRealizing),
        [Ok(Followed::Phase(UpgradePhase::PackageRealizing))],
        "our run is still followed"
    );
}

#[tokio::test]
async fn another_run_is_skipped_before_ours_appears() {
    let mut bench = bench();

    assert_eq!(bench.publish(Some(foreign())), []);

    assert_eq!(
        bench.phase(UpgradePhase::PackageRealizing),
        [Ok(Followed::Phase(UpgradePhase::PackageRealizing))],
        "what preceded our start is neither ours nor a loss"
    );
}

#[tokio::test]
async fn a_cleared_display_is_skipped_before_ours_appears() {
    let mut bench = bench();
    bench.publish(Some(foreign()));

    assert_eq!(bench.publish(None), []);

    assert_eq!(
        bench.phase(UpgradePhase::PackageRealizing),
        [Ok(Followed::Phase(UpgradePhase::PackageRealizing))]
    );
}

#[tokio::test]
async fn a_run_without_an_id_is_never_ours() {
    let mut bench = bench();
    let anonymous = Some(snapshot(
        None,
        running(Some(UpgradePhase::FirmwareDownloading), None),
    ));

    assert_eq!(bench.publish(anonymous.clone()), []);
    bench.phase(UpgradePhase::PackageRealizing);

    assert_eq!(
        bench.publish(anonymous),
        [LOST],
        "an automatic or legacy run has no id and must not be reported as the caller's"
    );
}

#[tokio::test]
async fn another_run_after_ours_loses_track() {
    let mut bench = bench();
    bench.phase(UpgradePhase::PackageRealizing);

    assert_eq!(bench.publish(Some(foreign())), [LOST]);
    assert!(bench.ended());
}

#[tokio::test]
async fn a_cleared_display_after_ours_loses_track() {
    let mut bench = bench();
    bench.phase(UpgradePhase::PackageRealizing);

    assert_eq!(
        bench.publish(None),
        [LOST],
        "the display dropped our run without an outcome, so none may be invented"
    );
    assert!(bench.ended());
}

#[tokio::test]
async fn a_dropped_channel_loses_track() {
    let Bench {
        offer,
        display,
        unseen_outcomes: _unseen_outcomes,
        mut stream,
    } = bench();
    display.send_replace(Some(snapshot(
        Some(offer),
        running(Some(UpgradePhase::PackageRealizing), None),
    )));
    assert_eq!(
        stream.next().await,
        Some(Ok(Followed::Phase(UpgradePhase::PackageRealizing)))
    );

    drop(display);

    assert_eq!(stream.next().await, Some(LOST));
    assert_eq!(stream.next().await, None);
}

#[tokio::test(start_paused = true)]
async fn a_run_that_never_appears_loses_track_at_the_deadline() {
    let mut bench = bench();
    assert_eq!(bench.ready(), []);

    tokio::time::advance(SEEN_WITHIN.saturating_sub(SECOND)).await;
    assert_eq!(
        bench.publish(Some(foreign())),
        [],
        "the run may still appear, and another run on display does not extend the wait"
    );

    tokio::time::advance(SECOND).await;
    assert_eq!(
        bench.ready(),
        [LOST],
        "a start Boser accepted but never showed must not hang the caller"
    );
    assert!(bench.ended());
}

#[tokio::test(start_paused = true)]
async fn the_deadline_stops_applying_once_ours_was_seen() {
    let mut bench = bench();
    bench.phase(UpgradePhase::PackageRealizing);

    tokio::time::advance(SEEN_WITHIN * 2).await;

    assert_eq!(
        bench.ready(),
        [],
        "the deadline bounds the wait for the run to appear, not the run"
    );
    assert!(!bench.ended());
}
