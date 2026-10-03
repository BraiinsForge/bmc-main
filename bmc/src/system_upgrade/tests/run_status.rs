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

fn running(generation: usize) -> UpgradeRunSnapshot {
    UpgradeRunSnapshot {
        generation: UpgradeGeneration::new(generation),
        id: None,
        state: UpgradeRunStatus::Running {
            kind: UpgradeKind::Firmware,
            phase: Some(UpgradePhase::FirmwareDownloading),
            progress: None,
        },
    }
}

fn received(events: &mut RunStatusEvents) -> Vec<Option<UpgradeRunSnapshot>> {
    std::iter::from_fn(|| events.try_recv().ok()).collect()
}

#[test]
fn every_display_change_reaches_the_event_stream_once_and_in_order() {
    let display = RunStatusService::new();
    let mut events = display
        .take_events()
        .expect("BUG: a new run status service still holds its events");
    let failed = UpgradeRunSnapshot {
        generation: UpgradeGeneration::new(0),
        id: None,
        state: UpgradeRunStatus::Failed {
            kind: UpgradeKind::Firmware,
            reason: String::new(),
        },
    };

    display.publish(running(0));
    display.publish(running(0));
    display.clear();
    display.clear();
    display.publish(failed.clone());

    assert_eq!(
        received(&mut events),
        [Some(running(0)), None, Some(failed)],
        "the pause listener must see each transition exactly once, and nothing that did not change"
    );
}

#[test]
fn the_event_stream_is_handed_out_once() {
    let display = RunStatusService::new();

    assert!(display.take_events().is_some());
    assert!(
        display.take_events().is_none(),
        "a second reader would split the transitions between two listeners"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_publishers_leave_the_stream_ending_on_the_settled_watch() {
    const PUBLISHERS: usize = 4;
    const PUBLISHES: usize = 250;
    let display = RunStatusService::new();
    let mut events = display
        .take_events()
        .expect("BUG: a new run status service still holds its events");

    let publishers: Vec<_> = (0..PUBLISHERS)
        .map(|publisher| {
            let display = display.clone();
            tokio::spawn(async move {
                for publish in 0..PUBLISHES {
                    display.publish(running(publisher * PUBLISHES + publish));
                }
            })
        })
        .collect();
    for publisher in publishers {
        publisher.await.expect("BUG: a publisher panicked");
    }

    let received = received(&mut events);
    assert_eq!(received.len(), PUBLISHERS * PUBLISHES);
    assert_eq!(
        received.last(),
        Some(&*display.subscribe().borrow()),
        "the last event must be the value the watch settled on, or the listener acts on a stale state"
    );
}

#[test]
fn the_projector_carries_the_offer_id_and_the_failure_text() {
    let id = ExecutionId::new();
    let generation = UpgradeGeneration::new(0);
    let mut projector = UpgradeRunProjector::new(generation, Some(id), UpgradeKind::Packages);

    assert_eq!(
        projector.initial_snapshot().id,
        Some(id),
        "the run is identifiable from its first snapshot on"
    );
    assert_eq!(
        projector
            .project(&UpgradeRunState::Phase(UpgradePhase::PackageRealizing))
            .id,
        Some(id),
        "a later snapshot of the run keeps its id"
    );
    assert_eq!(
        projector.project(&UpgradeRunState::Failed(
            SystemUpgradeError::PackageUpgradeFailed("boom".to_owned())
        )),
        UpgradeRunSnapshot {
            generation,
            id: Some(id),
            state: UpgradeRunStatus::Failed {
                kind: UpgradeKind::Packages,
                reason: "Package upgrade failed: boom".to_owned(),
            },
        },
        "the display carries why the run failed, not only that it did"
    );
}
