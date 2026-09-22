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

use std::time::Duration;

use bmc_upgrade_types::{
    ExecutionId, FirmwarePhase, PackagePhase, UpgradePhase as WirePhase, UpgradeState,
};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::{DisplayStateService, StateService, SystemUpgradeState};
use crate::boser::{StateSink, StreamConfig};
use crate::compositor::{
    UpgradeDisplaySnapshot, UpgradeDisplayState, UpgradeGeneration, UpgradeKind, UpgradePhase,
};

const UPGRADE_OUTAGE_GRACE: Duration = Duration::from_secs(30);

pub(crate) fn spawn_observer(
    config: StreamConfig,
    display: DisplayStateService,
    state: StateService,
) -> JoinHandle<()> {
    crate::boser::spawn(config, Projection::new(display, state))
}

/// Which Boser flow a snapshot belongs to: an execution with Boser's id,
/// or the id-less legacy download.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecutionKey {
    Boser(ExecutionId),
    Download,
}

struct Projection {
    display: DisplayStateService,
    state: StateService,
    current: Option<(ExecutionKey, UpgradeGeneration)>,
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
                self.present(generation, state.clone());
                return;
            }
            // `None` or another flow while something is on display: that execution
            // ended and its outcome was not observed.
            self.end_current();
        }
        // A terminal presents only when it continues the execution on display,
        // which rules out both the terminal Boser retains at boot and its replay
        // after a reconnect.
        if let Some((key, state @ UpgradeDisplayState::Running { .. })) = projected {
            let generation = self.display.next_generation();
            self.current = Some((key, generation));
            self.present(generation, state);
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
    fn new(display: DisplayStateService, state: StateService) -> Self {
        Self {
            display,
            state,
            current: None,
            outage_since: None,
        }
    }

    fn present(&mut self, generation: UpgradeGeneration, state: UpgradeDisplayState) {
        let outcome = match &state {
            UpgradeDisplayState::Running { .. } => None,
            UpgradeDisplayState::Succeeded { .. } => Some(SystemUpgradeState::Finished),
            UpgradeDisplayState::Failed { .. } => Some(SystemUpgradeState::Failed),
        };
        self.display
            .publish(UpgradeDisplaySnapshot { generation, state });
        match outcome {
            None => self.state.notify(SystemUpgradeState::UpgradeStarted),
            Some(outcome) => {
                self.state.notify(outcome);
                self.current = None;
                self.outage_since = None;
            }
        }
    }

    /// Only a live execution ends here: a terminal already handed
    /// to the compositor keeps its deadline.
    fn end_current(&mut self) {
        self.outage_since = None;
        if self.current.take().is_some() {
            self.display.clear();
            self.state.clear();
        }
    }
}

fn project(response: &UpgradeState) -> Option<(ExecutionKey, UpgradeDisplayState)> {
    let projected = match response {
        UpgradeState::None => return None,
        UpgradeState::DownloadingImage { download } => (
            ExecutionKey::Download,
            UpgradeDisplayState::Running {
                kind: UpgradeKind::Firmware,
                phase: Some(UpgradePhase::FirmwareDownloading),
                progress: Some(*download),
            },
        ),
        UpgradeState::DownloadFailed { .. } => (
            ExecutionKey::Download,
            UpgradeDisplayState::Failed {
                kind: UpgradeKind::Firmware,
            },
        ),
        UpgradeState::Running {
            id,
            kind,
            phase,
            download,
        } => (
            ExecutionKey::Boser(*id),
            UpgradeDisplayState::Running {
                kind: *kind,
                phase: display_phase(*phase),
                progress: *download,
            },
        ),
        UpgradeState::Rebooting { id, kind } => (
            ExecutionKey::Boser(*id),
            UpgradeDisplayState::Running {
                kind: *kind,
                phase: Some(UpgradePhase::FirmwareApplying),
                progress: None,
            },
        ),
        UpgradeState::Completed { id, kind } => (
            ExecutionKey::Boser(*id),
            UpgradeDisplayState::Succeeded { kind: *kind },
        ),
        UpgradeState::Failed { id, kind, .. } => (
            ExecutionKey::Boser(*id),
            UpgradeDisplayState::Failed { kind: *kind },
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
