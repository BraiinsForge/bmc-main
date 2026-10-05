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

//! One upgrade run's progress, read from what the Boser observer publishes about it.
//! The stream ends as soon as it can no longer tell what the run is doing.

use std::collections::VecDeque;

use bmc_upgrade_types::ExecutionId;
use futures::Stream;
use thiserror::Error;
use tokio::time::Instant;

use crate::compositor::{DownloadProgress, UpgradePhase, UpgradeRunSnapshot, UpgradeRunStatus};
use crate::system_upgrade::{RunUpdates, UnseenOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Followed {
    Phase(UpgradePhase),
    Download(DownloadProgress),
    Finished,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum FollowError {
    /// The run's own reason for the failure.
    #[error("{0}")]
    Failed(String),
    #[error("lost track of the upgrade; it may still be running")]
    Lost,
}

/// Follows the run started from `offer` through `updates`,
/// whose current values the caller has already marked seen.
/// The run has to appear by `seen_by`.
pub(crate) fn follow(
    offer: ExecutionId,
    updates: RunUpdates,
    seen_by: Instant,
) -> impl Stream<Item = Result<Followed, FollowError>> + Send + 'static {
    let follower = Follower {
        offer,
        updates,
        seen_by,
        seen: false,
        last_phase: None,
        last_download: None,
        pending: VecDeque::new(),
        ended: false,
    };
    futures::stream::unfold(follower, |mut follower| async move {
        let item = follower.next().await?;
        Some((item, follower))
    })
}

enum Changed {
    Display,
    UnseenOutcome,
}

enum Update {
    Display(Option<UpgradeRunSnapshot>),
    UnseenOutcome(Option<UnseenOutcome>),
}

struct Follower {
    offer: ExecutionId,
    updates: RunUpdates,
    seen_by: Instant,
    /// Whether a snapshot of our run has been published yet.
    seen: bool,
    last_phase: Option<UpgradePhase>,
    last_download: Option<DownloadProgress>,
    pending: VecDeque<Result<Followed, FollowError>>,
    ended: bool,
}

impl Follower {
    async fn next(&mut self) -> Option<Result<Followed, FollowError>> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                return Some(item);
            }
            if self.ended {
                return None;
            }
            match self.changed().await {
                Ok(Update::Display(snapshot)) => self.judge(snapshot),
                Ok(Update::UnseenOutcome(outcome)) => self.conclude(outcome),
                Err(lost) => self.end(Err(lost)),
            }
        }
    }

    /// The next published value; lost when the channel is gone or our run did not appear in time.
    async fn changed(&mut self) -> Result<Update, FollowError> {
        let RunUpdates {
            display,
            unseen_outcomes,
        } = &mut self.updates;
        let next = async {
            tokio::select! {
                changed = display.changed() => changed.map(|()| Changed::Display),
                changed = unseen_outcomes.changed() => changed.map(|()| Changed::UnseenOutcome),
            }
        };
        let changed = if self.seen {
            next.await
        } else {
            tokio::time::timeout_at(self.seen_by, next)
                .await
                .map_err(|_deadline| FollowError::Lost)?
        };
        Ok(
            match changed.map_err(|_sender_dropped| FollowError::Lost)? {
                Changed::Display => Update::Display(display.borrow_and_update().clone()),
                Changed::UnseenOutcome => {
                    Update::UnseenOutcome(unseen_outcomes.borrow_and_update().clone())
                }
            },
        )
    }

    /// A run too quick to be seen running never reaches the display; only its outcome is told.
    fn conclude(&mut self, outcome: Option<UnseenOutcome>) {
        let Some(outcome) = outcome.filter(|outcome| outcome.id == self.offer) else {
            return;
        };
        match outcome.result {
            Ok(()) => self.end(Ok(Followed::Finished)),
            Err(reason) => self.end(Err(FollowError::Failed(reason))),
        }
    }

    fn judge(&mut self, snapshot: Option<UpgradeRunSnapshot>) {
        let Some(state) = snapshot
            .filter(|snapshot| snapshot.id == Some(self.offer))
            .map(|snapshot| snapshot.state)
        else {
            // Before ours appears this may still describe what preceded the start.
            if self.seen {
                self.end(Err(FollowError::Lost));
            }
            return;
        };
        self.seen = true;
        match state {
            UpgradeRunStatus::Running {
                phase, progress, ..
            } => {
                // Applying is reported once the run hands over to the reboot, as on a local run:
                // Boser flashes first and may still stage packages, and fail, before that.
                if let Some(phase) = phase.filter(|phase| *phase != UpgradePhase::FirmwareApplying)
                    && self.last_phase != Some(phase)
                {
                    self.last_phase = Some(phase);
                    self.last_download = None;
                    self.pending.push_back(Ok(Followed::Phase(phase)));
                }
                if let Some(progress) = progress
                    && self.last_download != Some(progress)
                {
                    self.last_download = Some(progress);
                    self.pending.push_back(Ok(Followed::Download(progress)));
                }
            }
            UpgradeRunStatus::Rebooting { .. } => {
                self.end(Ok(Followed::Phase(UpgradePhase::FirmwareApplying)));
            }
            UpgradeRunStatus::Succeeded { .. } => self.end(Ok(Followed::Finished)),
            UpgradeRunStatus::Failed { reason, .. } => {
                self.end(Err(FollowError::Failed(reason)));
            }
        }
    }

    fn end(&mut self, last: Result<Followed, FollowError>) {
        self.pending.push_back(last);
        self.ended = true;
    }
}
