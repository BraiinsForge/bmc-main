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

use std::sync::Arc;
use std::time::Duration;

use bmc_grpc::web::{CheckForUpgradeResponse, FirmwareUpgrade, UpgradeDisruption, UpgradeProgress};
use bmc_intl::{Intl, format_message};
use bmc_upgrade_types::{Disruption, ExecutionId, wire};
use buffa_types::google::protobuf::Timestamp;
use chrono::{NaiveDate, NaiveTime};
use connectrpc::{ConnectError, ServiceStream};
use futures::stream::{self, StreamExt};
use tokio::time::Instant;
use tracing::warn;

use super::super::shared::{StatusExt as _, ToStatus, internal_error};
use super::{
    map_package_upgrade_plan, not_enough_space, run_state_to_progress, upgrade_expired,
    upgrade_failed, upgrade_in_progress,
};
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
        firmware: response.firmware.map(firmware_upgrade).into(),
        packages: response.packages.map(map_package_upgrade_plan).into(),
        disruption: disruption.into(),
    }
}

fn firmware_upgrade(firmware: wire::FirmwareUpgrade) -> FirmwareUpgrade {
    let release_date = match NaiveDate::parse_from_str(&firmware.release_date, "%Y-%m-%d") {
        Ok(date) => Some(Timestamp {
            seconds: date.and_time(NaiveTime::MIN).and_utc().timestamp(),
            nanos: 0,
            ..Default::default()
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
        release_date: release_date.into(),
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
/// The stream outlives the call, so it holds the caller's `intl` for what it words later.
pub(super) async fn start(
    intl: Arc<Intl>,
    boser: &BoserUpgrade,
    session_id: &str,
    upgrade_id: &str,
) -> ServiceStream<UpgradeProgress> {
    let Ok(offer) = upgrade_id.parse::<ExecutionId>() else {
        warn!(upgrade_id, "Upgrade id is not one Boser issued");
        return only(SystemUpgradeError::UpgradeExpired.to_status(&intl));
    };
    // Subscribed before the request: Boser publishes the execution before it answers.
    let updates = boser.subscribe();
    match boser.start(session_id, offer).await {
        Ok(()) | Err(ClientError::StartUnconfirmed) => {}
        Err(error) => return only(error.to_status(&intl)),
    }
    follow(offer, updates, Instant::now() + START_SEEN_WITHIN)
        .map(move |item| progress(&intl, item))
        .boxed()
}

pub(super) fn only(error: ConnectError) -> ServiceStream<UpgradeProgress> {
    stream::once(std::future::ready(Err(error))).boxed()
}

pub(super) fn boser_unavailable(intl: &Intl) -> ConnectError {
    ConnectError::unavailable(format_message!(
        intl,
        default_message: "Boser manages upgrades on this platform and its address is not configured.",
        description: "Error when the device cannot reach Boser for an upgrade because it was never told where Boser is; Boser is a product name",
    ))
}

fn progress(
    intl: &Intl,
    item: Result<Followed, FollowError>,
) -> Result<UpgradeProgress, ConnectError> {
    let followed = item.or_status(intl)?;
    let state = match followed {
        Followed::Phase(phase) => UpgradeRunState::Phase(phase),
        Followed::Download(download) => UpgradeRunState::Progress {
            downloaded_bytes: download.downloaded_bytes,
            total_bytes: download.total_bytes,
        },
        Followed::Finished => UpgradeRunState::Finished,
    };
    run_state_to_progress(intl, state)
}

impl ToStatus for ClientError {
    fn to_status(&self, intl: &Intl) -> ConnectError {
        match self {
            ClientError::Unreachable => ConnectError::unavailable(format_message!(
                intl,
                default_message: "Boser is unreachable.",
                description: "Error when the device cannot connect to Boser for an upgrade; Boser is a product name",
            )),
            ClientError::NoAnswer => ConnectError::unavailable(format_message!(
                intl,
                default_message: "Boser did not answer.",
                description: "Error when Boser stayed silent on an upgrade request; Boser is a product name",
            )),
            ClientError::StartUnconfirmed => internal_error(
                intl,
                "a start Boser did not answer is followed, not refused",
            ),
            ClientError::Unauthenticated => ConnectError::unauthenticated(format_message!(
                intl,
                default_message: "Boser rejected the session.",
                description: "Error when Boser does not accept the signed-in user's session for an upgrade; Boser is a product name",
            )),
            ClientError::Rejected(rejection) => match rejection {
                Rejection::Busy => ConnectError::unavailable(upgrade_in_progress(intl)),
                Rejection::Expired => ConnectError::failed_precondition(upgrade_expired(intl)),
                Rejection::NotEnoughSpace => {
                    ConnectError::failed_precondition(not_enough_space(intl))
                }
                Rejection::PackagesUnavailable => {
                    ConnectError::failed_precondition(format_message!(
                        intl,
                        default_message: "Packages cannot be upgraded on this device.",
                        description: "Error when an upgrade needs software packages and the device cannot change them right now",
                    ))
                }
                Rejection::InvalidArgument => ConnectError::invalid_argument(format_message!(
                    intl,
                    default_message: "Boser refused the upgrade request as invalid.",
                    description: "Error when Boser does not accept what the device asked of it for an upgrade; Boser is a product name",
                )),
                Rejection::Internal => ConnectError::internal(format_message!(
                    intl,
                    default_message: "Boser failed to handle the upgrade request.",
                    description: "Error when Boser could not complete an upgrade request; Boser is a product name",
                )),
            },
            ClientError::Unexpected(_) => ConnectError::internal(format_message!(
                intl,
                default_message: "Unexpected Boser response!",
                description: "Error when Boser's answer to an upgrade request cannot be understood; Boser is a product name",
            )),
        }
    }
}

impl ToStatus for FollowError {
    fn to_status(&self, intl: &Intl) -> ConnectError {
        match self {
            FollowError::Failed(_) => ConnectError::internal(upgrade_failed(intl)),
            FollowError::Lost => {
                warn!("Following the Boser upgrade failed: {self}");
                ConnectError::unavailable(format_message!(
                    intl,
                    default_message: "Lost track of the upgrade; it may still be running.",
                    description: "Error when the device stopped seeing the progress of an upgrade it started",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests;
