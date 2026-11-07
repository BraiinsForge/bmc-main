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
use bmc_grpc::web::UpgradeDisruption;
use bmc_upgrade_types::{Disruption, PackageChange, PackagesPreview, UpgradeKind as WireKind};
use connectrpc::ErrorCode;

use crate::compositor::{UpgradeGeneration, UpgradeRunSnapshot, UpgradeRunStatus};
use crate::system_upgrade::RunStatusService;
use crate::system_upgrade::boser::client::tests::MockBoser;
use crate::web::grpc::tests::{assert_worded, pseudo_locale, status_text};

fn english() -> Arc<Intl> {
    bmc_intl::shared_formatter(bmc_intl::Language::EN)
}

fn message(error: &ConnectError) -> &str {
    error.message.as_deref().unwrap_or_default()
}

fn firmware(release_date: &str) -> wire::FirmwareUpgrade {
    wire::FirmwareUpgrade {
        version: "26.10".to_owned(),
        hash: "abc".to_owned(),
        release_date: release_date.to_owned(),
        description: "notes".to_owned(),
        file_size_bytes: 42,
        previous_releases: vec![wire::PreviousRelease {
            version: "26.09".to_owned(),
            description: "older".to_owned(),
        }],
    }
}

/// The frontend must get the same shape from a managed check as from a Deck one.
#[test]
fn a_full_offer_translates_field_by_field() {
    let id = ExecutionId::new();
    let response = wire::CheckUpgradeResponse {
        offer: Some(wire::Offer {
            id,
            kind: WireKind::FirmwareAndPackages,
            disruption: Disruption::Reboot,
        }),
        firmware: Some(firmware("2026-10-01")),
        packages: Some(PackagesPreview {
            changes: vec![PackageChange {
                name: "widget-clock".to_owned(),
                version_from: None,
                version_to: Some("1.2.0".to_owned()),
                category: Some("widget".to_owned()),
                changelog: None,
            }],
            download_size_bytes: Some(7),
            unpacked_size_bytes: Some(9),
            bmc_version: Some("3.0".to_owned()),
            bmc_changelog: None,
        }),
        package_capability: wire::PackageCapability::Unhealthy {
            reason: "dropped on purpose".to_owned(),
        },
    };

    let translated = check_response(response);

    assert_eq!(
        translated,
        CheckForUpgradeResponse {
            upgrade_id: Some(id.to_string()),
            firmware: FirmwareUpgrade {
                hash: "abc".to_owned(),
                version: "26.10".to_owned(),
                // 2026-10-01T00:00:00Z
                release_date: Timestamp {
                    seconds: 1_790_812_800,
                    nanos: 0,
                    ..Default::default()
                }
                .into(),
                description: "notes".to_owned(),
                file_size_bytes: 42,
                previous_releases: vec![bmc_grpc::web::ReleaseInfo {
                    version: "26.09".to_owned(),
                    description: "older".to_owned(),
                }],
            }
            .into(),
            packages: bmc_grpc::web::PackageUpgradePlan {
                changes: vec![bmc_grpc::web::PackageChange {
                    name: "widget-clock".to_owned(),
                    version_from: None,
                    version_to: Some("1.2.0".to_owned()),
                    category: Some("widget".to_owned()),
                    changelog: None,
                }],
                download_size_bytes: Some(7),
                bmc_version: Some("3.0".to_owned()),
                bmc_changelog: None,
            }
            .into(),
            disruption: UpgradeDisruption::Reboot.into(),
        }
    );
}

/// "Nothing to do" has to look exactly like the local up-to-date answer.
#[test]
fn no_offer_translates_to_an_empty_response() {
    let translated = check_response(wire::CheckUpgradeResponse {
        offer: None,
        firmware: None,
        packages: None,
        package_capability: wire::PackageCapability::Ready,
    });

    assert_eq!(translated, CheckForUpgradeResponse::default());
    assert_eq!(translated.disruption, UpgradeDisruption::Unspecified);
}

/// A date Boser formats differently must not fail the whole check.
#[test]
fn an_unparseable_release_date_is_left_unset() {
    let translated = check_response(wire::CheckUpgradeResponse {
        offer: None,
        firmware: Some(firmware("October 1st")),
        packages: None,
        package_capability: wire::PackageCapability::Ready,
    });

    let firmware = translated
        .firmware
        .expect("BUG: the firmware is still offered");
    assert!(firmware.release_date.is_unset());
    assert_eq!(firmware.version, "26.10");
}

/// The codes are the ones Deck answers for the same condition, so one frontend handles both,
/// except that Boser folds every package source failure into `INTERNAL`.
#[test]
fn client_errors_map_to_the_codes_the_local_path_uses() {
    let rejected = ClientError::Rejected;
    let cases = [
        (ClientError::Unreachable, ErrorCode::Unavailable),
        (ClientError::NoAnswer, ErrorCode::Unavailable),
        (ClientError::StartUnconfirmed, ErrorCode::Internal),
        (ClientError::Unauthenticated, ErrorCode::Unauthenticated),
        (rejected(Rejection::Busy), ErrorCode::Unavailable),
        (rejected(Rejection::Expired), ErrorCode::FailedPrecondition),
        (
            rejected(Rejection::NotEnoughSpace),
            ErrorCode::FailedPrecondition,
        ),
        (
            rejected(Rejection::PackagesUnavailable),
            ErrorCode::FailedPrecondition,
        ),
        (
            rejected(Rejection::InvalidArgument),
            ErrorCode::InvalidArgument,
        ),
        (rejected(Rejection::Internal), ErrorCode::Internal),
        (
            ClientError::Unexpected(reqwest::StatusCode::BAD_GATEWAY),
            ErrorCode::Internal,
        ),
    ];
    for (error, code) in cases {
        assert_eq!(error.to_status(&english()).code, code, "{error:?}");
    }
}

/// One frontend reads both paths, so a refusal Deck also knows reads as it does on Deck.
#[test]
fn a_rejection_the_local_path_shares_reads_the_same() {
    for (rejection, local) in [
        (Rejection::Busy, SystemUpgradeError::UpgradeInProgress),
        (Rejection::Expired, SystemUpgradeError::UpgradeExpired),
        (
            Rejection::NotEnoughSpace,
            SystemUpgradeError::NotEnoughSpace,
        ),
    ] {
        let managed = ClientError::Rejected(rejection).to_status(&english());
        let local = local.to_status(&english());
        assert_eq!(message(&managed), message(&local), "{rejection:?}");
    }
}

/// The browser is told that Boser is unreachable,
/// not where the BMC application looked for it.
#[test]
fn an_unreachable_boser_is_reported_without_its_address() {
    let status = ClientError::Unreachable.to_status(&english());

    assert_eq!(message(&status), "Boser is unreachable.");
}

/// A failed run is a server-side fault; a lost one clears by retrying.
#[test]
fn follow_errors_map_to_internal_and_unavailable() {
    let failed = FollowError::Failed("no space left".to_owned()).to_status(&english());
    assert_eq!(failed.code, ErrorCode::Internal);
    assert_eq!(message(&failed), "Upgrade failed!");

    let lost = FollowError::Lost.to_status(&english());
    assert_eq!(lost.code, ErrorCode::Unavailable);
    assert!(message(&lost).contains("may still be running"));
}

/// The run fails long after the call that started it,
/// so only the stream still knows the language of whoever asked.
#[tokio::test]
async fn a_run_failing_later_speaks_the_language_of_whoever_started_it() {
    let boser = MockBoser::default();
    let address = boser.serve(reqwest::StatusCode::NO_CONTENT, "").await;
    let display = RunStatusService::new();
    let upgrade = BoserUpgrade::new(address, display.clone());
    let offer = ExecutionId::new();

    let mut stream = start(
        Arc::new(pseudo_locale()),
        &upgrade,
        "session",
        &offer.to_string(),
    )
    .await;
    display.publish(UpgradeRunSnapshot {
        generation: UpgradeGeneration::new(0),
        id: Some(offer),
        state: UpgradeRunStatus::Failed {
            kind: WireKind::Packages,
            reason: "1".to_owned(),
        },
    });
    let error = stream
        .next()
        .await
        .expect("BUG: the stream reports how the run ended")
        .expect_err("BUG: the run failed");

    assert_eq!(message(&error), bmc_intl_pseudo::respell("Upgrade failed!"));
}

#[test]
fn every_boser_refusal_is_worded() {
    assert_worded(|intl| {
        let mut refusals: Vec<String> = [
            ClientError::Unreachable,
            ClientError::NoAnswer,
            ClientError::StartUnconfirmed,
            ClientError::Unauthenticated,
            ClientError::Rejected(Rejection::Busy),
            ClientError::Rejected(Rejection::Expired),
            ClientError::Rejected(Rejection::NotEnoughSpace),
            ClientError::Rejected(Rejection::PackagesUnavailable),
            ClientError::Rejected(Rejection::InvalidArgument),
            ClientError::Rejected(Rejection::Internal),
            ClientError::Unexpected(reqwest::StatusCode::BAD_GATEWAY),
        ]
        .iter()
        .map(|error| status_text(&error.to_status(intl)))
        .collect();
        refusals.push(status_text(&boser_unavailable(intl)));
        refusals.push(status_text(&FollowError::Lost.to_status(intl)));
        refusals.push(status_text(
            &FollowError::Failed("1".to_owned()).to_status(intl),
        ));
        refusals
    });
}
