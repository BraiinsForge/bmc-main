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

//! The upgrade RPCs of a Boser-managed product: Boser's wire types and errors in gRPC terms.

use std::time::Duration;

use bmc_grpc::web::{CheckForUpgradeResponse, FirmwareUpgrade, UpgradeDisruption, UpgradeProgress};
use bmc_upgrade_types::{Disruption, ExecutionId, wire};
use chrono::{NaiveDate, NaiveTime};
use futures::stream::{self, BoxStream, StreamExt};
use prost_types::Timestamp;
use tokio::time::Instant;
use tonic::Status;
use tracing::warn;

use super::{map_package_upgrade_plan, run_state_to_progress};
use crate::system_upgrade::boser::client::{BoserUpgrade, ClientError, Rejection};
use crate::system_upgrade::boser::progress::{FollowError, Followed, follow};
use crate::system_upgrade::{SystemUpgradeError, UpgradeRunState};

/// How long after a start Boser accepted, or did not answer, its execution has to show up
/// on the display. Boser publishes the execution before it answers, so for an accepted
/// start this covers only the observer's lag.
const START_SEEN_WITHIN: Duration = Duration::from_secs(30);

pub(super) fn check_response(response: wire::CheckUpgradeResponse) -> CheckForUpgradeResponse {
    let (upgrade_id, disruption) = match response.offer {
        Some(offer) => (
            Some(offer.id.to_string()),
            match offer.disruption {
                Disruption::Reboot => UpgradeDisruption::Reboot,
                Disruption::AppRestart => UpgradeDisruption::AppRestart,
            },
        ),
        None => (None, UpgradeDisruption::Unspecified),
    };
    CheckForUpgradeResponse {
        upgrade_id,
        firmware: response.firmware.map(firmware_upgrade),
        packages: response.packages.map(map_package_upgrade_plan),
        disruption: disruption.into(),
    }
}

fn firmware_upgrade(firmware: wire::FirmwareUpgrade) -> FirmwareUpgrade {
    let release_date = match NaiveDate::parse_from_str(&firmware.release_date, "%Y-%m-%d") {
        Ok(date) => Some(Timestamp {
            seconds: date.and_time(NaiveTime::MIN).and_utc().timestamp(),
            nanos: 0,
        }),
        Err(error) => {
            warn!(
                release_date = firmware.release_date,
                %error,
                "Boser's firmware release date is not YYYY-MM-DD"
            );
            None
        }
    };
    FirmwareUpgrade {
        hash: firmware.hash,
        version: firmware.version,
        release_date,
        description: firmware.description,
        file_size_bytes: firmware.file_size_bytes,
        previous_releases: firmware
            .previous_releases
            .into_iter()
            .map(|release| bmc_grpc::web::ReleaseInfo {
                version: release.version,
                description: release.description,
            })
            .collect(),
    }
}

/// Starts offer `upgrade_id` in Boser and follows it.
/// A start that failed for any other reason is the stream's only item, as on the local path.
/// A start Boser did not answer may still have been admitted, so the run
/// is followed like an accepted one and is lost only if it never appears.
pub(super) async fn start(
    boser: &BoserUpgrade,
    session_id: &str,
    upgrade_id: &str,
) -> BoxStream<'static, Result<UpgradeProgress, Status>> {
    let Ok(offer) = upgrade_id.parse::<ExecutionId>() else {
        return only(SystemUpgradeError::UpgradeExpired.into());
    };
    // Subscribed before the request: Boser publishes the execution before it answers.
    let updates = boser.subscribe();
    match boser.start(session_id, offer).await {
        Ok(()) | Err(ClientError::StartUnconfirmed) => {}
        Err(error) => return only(error.into()),
    }
    follow(offer, updates, Instant::now() + START_SEEN_WITHIN)
        .map(progress)
        .boxed()
}

pub(super) fn only(status: Status) -> BoxStream<'static, Result<UpgradeProgress, Status>> {
    stream::once(std::future::ready(Err(status))).boxed()
}

pub(super) fn boser_unavailable() -> Status {
    Status::unavailable("Boser manages upgrades on this platform and its address is not configured")
}

fn progress(item: Result<Followed, FollowError>) -> Result<UpgradeProgress, Status> {
    let state = match item? {
        Followed::Phase(phase) => UpgradeRunState::Phase(phase),
        Followed::Download(download) => UpgradeRunState::Progress {
            downloaded_bytes: download.downloaded_bytes,
            total_bytes: download.total_bytes,
        },
        Followed::Finished => UpgradeRunState::Finished,
    };
    run_state_to_progress(state)
}

impl From<ClientError> for Status {
    fn from(error: ClientError) -> Self {
        let message = error.to_string();
        match error {
            ClientError::Unreachable | ClientError::NoAnswer | ClientError::StartUnconfirmed => {
                Status::unavailable(message)
            }
            ClientError::Unauthenticated => Status::unauthenticated(message),
            ClientError::Rejected { rejection, .. } => match rejection {
                Rejection::Busy => Status::unavailable(message),
                Rejection::Expired | Rejection::NotEnoughSpace | Rejection::PackagesUnavailable => {
                    Status::failed_precondition(message)
                }
                Rejection::InvalidArgument => Status::invalid_argument(message),
                Rejection::Internal => Status::internal(message),
            },
            ClientError::Unexpected(_) => Status::internal(message),
        }
    }
}

impl From<FollowError> for Status {
    fn from(error: FollowError) -> Self {
        let message = error.to_string();
        match error {
            FollowError::Failed(_) => Status::internal(message),
            FollowError::Lost => Status::unavailable(message),
        }
    }
}

#[cfg(test)]
mod tests;
