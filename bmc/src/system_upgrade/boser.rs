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

//! Observation of Boser's upgrade state on managed products: one task
//! follows the state stream and presents each execution on the display
//! and the restart block, the way the local flow does on Deck.

pub(crate) mod client;
pub(crate) mod progress;

use std::time::Duration;

use bmc_upgrade_types::{
    ExecutionId, FirmwarePhase, PackagePhase, UpgradePhase as WirePhase, UpgradeState,
};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::{RunStatusService, StateService, SystemUpgradeState, UnseenOutcome};
use crate::boser::{StateSink, StreamConfig};
use crate::compositor::{
    UpgradeGeneration, UpgradeKind, UpgradePhase, UpgradeRunSnapshot, UpgradeRunStatus,
};

const UPGRADE_OUTAGE_GRACE: Duration = Duration::from_secs(30);

pub(crate) fn spawn_observer(
    config: StreamConfig,
    display: RunStatusService,
    state: StateService,
) -> JoinHandle<()> {
    crate::boser::spawn(config, Projection::new(display, state))
}

/// Where the upgrade RPCs of this product go.
#[derive(Debug)]
pub(crate) enum UpgradeRoute {
    Local,
    Boser(client::BoserUpgrade),
    /// Managed product without a Boser address: the RPCs fail, they never run locally.
    BoserUnavailable,
}

/// Which Boser flow a snapshot belongs to: an execution with Boser's id,
/// or the id-less legacy download.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecutionKey {
    Boser(ExecutionId),
    Download,
}

impl ExecutionKey {
    fn id(&self) -> Option<ExecutionId> {
        match self {
            Self::Boser(id) => Some(*id),
            Self::Download => None,
        }
    }
}

struct Projection {
    display: RunStatusService,
    state: StateService,
    current: Option<(ExecutionKey, UpgradeGeneration)>,
    /// The execution that ended without an outcome, until another one starts:
    /// an outcome Boser reports for it after the outage grace still counts.
    ended: Option<(ExecutionKey, UpgradeGeneration)>,
    /// The execution whose outcome went to the display, which Boser will replay.
    presented: Option<ExecutionId>,
    outage_since: Option<Instant>,
}

impl StateSink for Projection {
    type State = UpgradeState;

    const PATH: &'static str = "/api/v1/upgrade/state/events";

    fn observe(&mut self, response: &UpgradeState) {
        self.outage_since = None;
        let projected = project(response);
        if let Some((current_key, generation)) = self.current.clone() {
            if let Some((key, state)) = &projected
                && *key == current_key
            {
                self.present(generation, current_key.id(), state.clone());
                return;
            }
            // `None` or another flow while something is on display: that execution
            // ended and its outcome was not observed.
            self.end_current();
        }
        // Present an outcome only for the running execution or the one that ended
        // before its outcome arrived. That keeps off the display the outcome Boser
        // retains from before boot and a replay of one already presented.
        match projected {
            Some((
                key,
                state @ (UpgradeRunStatus::Running { .. } | UpgradeRunStatus::Rebooting { .. }),
            )) => {
                self.ended = None;
                let generation = self.display.next_generation();
                let id = key.id();
                self.current = Some((key, generation));
                self.present(generation, id, state);
            }
            Some((key, state)) => {
                if let Some((_, generation)) = self.ended.take_if(|(ended, _)| *ended == key) {
                    self.present(generation, key.id(), state);
                } else if key.id() != self.presented
                    && let Some(outcome) = unseen_outcome(&key, state)
                {
                    self.display.publish_unseen_outcome(outcome);
                }
            }
            None => {}
        }
    }

    /// A snapshot this build cannot read ends the execution
    /// without an outcome, the way `None` mid-run does.
    fn contract_mismatch(&mut self) {
        self.end_current();
    }

    fn stream_lost(&mut self) {
        if self.current.is_none() {
            self.outage_since = None;
            return;
        }
        let now = Instant::now();
        let since = self.outage_since.get_or_insert(now);
        if now.saturating_duration_since(*since) >= UPGRADE_OUTAGE_GRACE {
            self.end_current();
        }
    }
}

impl Projection {
    fn new(display: RunStatusService, state: StateService) -> Self {
        Self {
            display,
            state,
            current: None,
            ended: None,
            presented: None,
            outage_since: None,
        }
    }

    fn present(
        &mut self,
        generation: UpgradeGeneration,
        id: Option<ExecutionId>,
        state: UpgradeRunStatus,
    ) {
        let outcome = match &state {
            UpgradeRunStatus::Running { .. } | UpgradeRunStatus::Rebooting { .. } => None,
            UpgradeRunStatus::Succeeded { .. } => Some(SystemUpgradeState::Finished),
            UpgradeRunStatus::Failed { .. } => Some(SystemUpgradeState::Failed),
        };
        self.display.publish(UpgradeRunSnapshot {
            generation,
            id,
            state,
        });
        match outcome {
            None => self.state.notify(SystemUpgradeState::UpgradeStarted),
            Some(outcome) => {
                self.state.notify(outcome);
                if id.is_some() {
                    self.presented = id;
                }
                self.current = None;
                self.outage_since = None;
            }
        }
    }

    /// Only a live execution ends here: a terminal already handed
    /// to the compositor keeps its deadline.
    fn end_current(&mut self) {
        self.outage_since = None;
        if let Some(ended) = self.current.take() {
            self.ended = Some(ended);
            self.display.clear();
            self.state.clear();
        }
    }
}

fn unseen_outcome(key: &ExecutionKey, state: UpgradeRunStatus) -> Option<UnseenOutcome> {
    let result = match state {
        UpgradeRunStatus::Succeeded { .. } => Ok(()),
        UpgradeRunStatus::Failed { reason, .. } => Err(reason),
        UpgradeRunStatus::Running { .. } | UpgradeRunStatus::Rebooting { .. } => return None,
    };
    Some(UnseenOutcome {
        id: key.id()?,
        result,
    })
}

fn project(response: &UpgradeState) -> Option<(ExecutionKey, UpgradeRunStatus)> {
    let projected = match response {
        UpgradeState::None => return None,
        UpgradeState::DownloadingImage { download } => (
            ExecutionKey::Download,
            UpgradeRunStatus::Running {
                kind: UpgradeKind::Firmware,
                phase: Some(UpgradePhase::FirmwareDownloading),
                progress: Some(*download),
            },
        ),
        UpgradeState::DownloadFailed { reason } => (
            ExecutionKey::Download,
            UpgradeRunStatus::Failed {
                kind: UpgradeKind::Firmware,
                reason: reason.clone(),
            },
        ),
        UpgradeState::Running {
            id,
            kind,
            phase,
            download,
        } => (
            ExecutionKey::Boser(*id),
            UpgradeRunStatus::Running {
                kind: *kind,
                phase: display_phase(*phase),
                progress: *download,
            },
        ),
        UpgradeState::Rebooting { id, kind } => (
            ExecutionKey::Boser(*id),
            UpgradeRunStatus::Rebooting { kind: *kind },
        ),
        UpgradeState::Completed { id, kind } => (
            ExecutionKey::Boser(*id),
            UpgradeRunStatus::Succeeded { kind: *kind },
        ),
        UpgradeState::Failed {
            id, kind, reason, ..
        } => (
            ExecutionKey::Boser(*id),
            UpgradeRunStatus::Failed {
                kind: *kind,
                reason: reason.clone(),
            },
        ),
    };
    Some(projected)
}

fn display_phase(phase: WirePhase) -> Option<UpgradePhase> {
    match phase {
        WirePhase::Preparing
        | WirePhase::Unknown
        | WirePhase::Packages(
            PackagePhase::Cleaning
            | PackagePhase::FindingGarbageRoots
            | PackagePhase::DeterminingGarbageLiveness
            | PackagePhase::Unknown,
        )
        | WirePhase::Firmware(FirmwarePhase::Unknown) => None,
        WirePhase::Firmware(FirmwarePhase::Downloading) => Some(UpgradePhase::FirmwareDownloading),
        WirePhase::Firmware(FirmwarePhase::Verifying) => Some(UpgradePhase::FirmwareVerifying),
        WirePhase::Firmware(FirmwarePhase::Flashing) => Some(UpgradePhase::FirmwareApplying),
        WirePhase::Packages(PackagePhase::Realizing) => Some(UpgradePhase::PackageRealizing),
        WirePhase::Packages(PackagePhase::Verifying) => Some(UpgradePhase::PackageVerifying),
        WirePhase::Packages(PackagePhase::Building) => Some(UpgradePhase::PackageBuilding),
        WirePhase::Packages(PackagePhase::Activating) => Some(UpgradePhase::PackageActivating),
    }
}

#[cfg(test)]
mod tests;
