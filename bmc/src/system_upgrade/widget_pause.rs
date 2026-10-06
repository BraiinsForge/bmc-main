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

//! Stops every widget while a firmware image is downloaded and flashed,
//! since the image lands on tmpfs.
//! It follows the upgrade state all producers publish to
//! [`RunStatusService`](super::RunStatusService).

use super::{RunStatusEvents, WidgetLifecycle};
use crate::compositor::{
    UpgradeGeneration, UpgradeKind, UpgradePhase, UpgradeRunSnapshot, UpgradeRunStatus,
};
use bmc_upgrade_types::Disruption;
use futures::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;
use tracing::info;

/// How long widgets stay stopped after their run clears the display without
/// an outcome, in case another firmware run takes its place.
/// Boser's legacy download ends in a bare `None`,
/// and the frontend starts the flash as a new execution right after.
/// A Boser stream lost past its outage grace clears the display the same way,
/// so widgets return even if Boser is still downloading:
/// the BMC cannot tell a dead Boser from a silent one.
pub(crate) const REPLACEMENT_GRACE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Preparing,
    /// Never lapses: going quiet now normally means the board is going down for the flash,
    /// and a board left without widgets beats widgets started next to a flash.
    /// Only a reported failure or the reboot brings them back; a Boser that restarts
    /// or stays silent without rebooting leaves them stopped until the BMC application
    /// restarts, by design.
    Applying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pause {
    Idle,
    Paused {
        generation: UpgradeGeneration,
        stage: Stage,
        lapses_at: Option<Instant>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Running,
    Stopped,
}

impl Pause {
    fn log(self) {
        match self {
            Self::Idle => info!("widget pause released"),
            Self::Paused {
                generation,
                stage,
                lapses_at,
            } => info!(
                generation = generation.get(),
                ?stage,
                lapse_armed = lapses_at.is_some(),
                "widget pause changed"
            ),
        }
    }

    fn desired(self) -> Level {
        match self {
            Self::Idle => Level::Running,
            Self::Paused { .. } => Level::Stopped,
        }
    }

    fn lapses_at(self) -> Option<Instant> {
        match self {
            Self::Idle => None,
            Self::Paused { lapses_at, .. } => lapses_at,
        }
    }
}

fn carries_firmware(kind: UpgradeKind) -> bool {
    match kind.disruption() {
        Disruption::Reboot => true,
        Disruption::AppRestart => false,
    }
}

fn step(pause: Pause, snapshot: Option<&UpgradeRunSnapshot>, now: Instant) -> Pause {
    let Some(snapshot) = snapshot else {
        return match pause {
            Pause::Paused {
                generation,
                stage: Stage::Preparing,
                lapses_at: None,
            } => Pause::Paused {
                generation,
                stage: Stage::Preparing,
                lapses_at: Some(now + REPLACEMENT_GRACE),
            },
            Pause::Paused {
                stage: Stage::Applying,
                ..
            }
            | Pause::Paused {
                lapses_at: Some(_), ..
            }
            | Pause::Idle => pause,
        };
    };
    match snapshot.state {
        UpgradeRunStatus::Running { kind, phase, .. } if carries_firmware(kind) => {
            let applying_before = matches!(
                pause,
                Pause::Paused { generation, stage: Stage::Applying, .. }
                    if generation == snapshot.generation
            );
            let stage = if applying_before || phase == Some(UpgradePhase::FirmwareApplying) {
                Stage::Applying
            } else {
                Stage::Preparing
            };
            Pause::Paused {
                generation: snapshot.generation,
                stage,
                lapses_at: None,
            }
        }
        UpgradeRunStatus::Succeeded { kind } if carries_firmware(kind) => pause,
        UpgradeRunStatus::Running { .. }
        | UpgradeRunStatus::Succeeded { .. }
        | UpgradeRunStatus::Failed { .. } => Pause::Idle,
    }
}

fn expire(pause: Pause, now: Instant) -> Pause {
    match pause {
        Pause::Paused {
            lapses_at: Some(deadline),
            ..
        } if deadline <= now => Pause::Idle,
        Pause::Paused { .. } | Pause::Idle => pause,
    }
}

/// `Some(generation)` exactly while every widget is stopped for that run.
pub(crate) type Acknowledgement = watch::Receiver<Option<UpgradeGeneration>>;

/// Starts the listener.
/// A self-managed run waits on the returned acknowledgement before downloading;
/// its sender drops if the listener dies.
/// A dead listener leaves widgets as they are until the BMC application restarts,
/// since it cannot tell whether a flash is under way.
pub(crate) fn spawn(
    events: RunStatusEvents,
    lifecycle: Arc<dyn WidgetLifecycle>,
) -> Acknowledgement {
    let (acknowledgement, acknowledged) = watch::channel(None);
    tokio::spawn(drive(events, lifecycle, acknowledgement));
    acknowledged
}

struct Operation {
    target: Level,
    run: BoxFuture<'static, ()>,
}

impl Operation {
    fn start(target: Level, lifecycle: &Arc<dyn WidgetLifecycle>) -> Self {
        let lifecycle = Arc::clone(lifecycle);
        let run: BoxFuture<'static, ()> = match target {
            Level::Stopped => Box::pin(async move { lifecycle.stop_all_widgets().await }),
            Level::Running => Box::pin(async move { lifecycle.restart_widgets().await }),
        };
        Self { target, run }
    }
}

enum Wake {
    Snapshot(Option<UpgradeRunSnapshot>),
    OperationDone,
    Lapsed,
}

async fn drive(
    mut events: RunStatusEvents,
    lifecycle: Arc<dyn WidgetLifecycle>,
    acknowledgement: watch::Sender<Option<UpgradeGeneration>>,
) {
    let mut pause = Pause::Idle;
    let mut reached = Level::Running;
    let mut in_flight: Option<Operation> = None;
    loop {
        let lapses_at = pause.lapses_at();
        let wake = tokio::select! {
            // The operation in flight is polled on every wake, so a stream of
            // snapshots cannot starve a stop; a queued snapshot outranks the lapse,
            // so a replacement run never expires by the luck of the draw.
            biased;
            () = async {
                in_flight
                    .as_mut()
                    .expect("BUG: polled only while an operation is in flight")
                    .run
                    .as_mut()
                    .await;
            }, if in_flight.is_some() => Wake::OperationDone,
            event = events.recv() => match event {
                Some(snapshot) => Wake::Snapshot(snapshot),
                None => return,
            },
            () = tokio::time::sleep_until(lapses_at.unwrap_or_else(Instant::now)),
                if lapses_at.is_some() => Wake::Lapsed,
        };

        // Snapshots queued since the poll go first as well.
        let before = pause;
        let now = Instant::now();
        if let Wake::Snapshot(snapshot) = &wake {
            pause = step(pause, snapshot.as_ref(), now);
        }
        while let Ok(snapshot) = events.try_recv() {
            pause = step(pause, snapshot.as_ref(), now);
        }
        match wake {
            Wake::Snapshot(_) => {}
            Wake::OperationDone => {
                reached = in_flight
                    .take()
                    .expect("BUG: an operation just completed")
                    .target;
            }
            Wake::Lapsed => pause = expire(pause, now),
        }
        if pause != before {
            pause.log();
        }
        if in_flight.is_none() && pause.desired() != reached {
            info!(target = ?pause.desired(), "widget lifecycle operation started");
            in_flight = Some(Operation::start(pause.desired(), &lifecycle));
        }
        let acknowledged = match pause {
            Pause::Paused { generation, .. }
                if in_flight.is_none() && reached == Level::Stopped =>
            {
                Some(generation)
            }
            Pause::Paused { .. } | Pause::Idle => None,
        };
        let changed = acknowledgement.send_if_modified(|current| {
            let changed = *current != acknowledged;
            *current = acknowledged;
            changed
        });
        if changed && let Some(generation) = acknowledged {
            info!(
                generation = generation.get(),
                "widgets stopped for the firmware run"
            );
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod test_support;
