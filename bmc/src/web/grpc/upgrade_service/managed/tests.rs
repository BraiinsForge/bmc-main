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
use tonic::Code;

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
            firmware: Some(FirmwareUpgrade {
                hash: "abc".to_owned(),
                version: "26.10".to_owned(),
                // 2026-10-01T00:00:00Z
                release_date: Some(Timestamp {
                    seconds: 1_790_812_800,
                    nanos: 0,
                }),
                description: "notes".to_owned(),
                file_size_bytes: 42,
                previous_releases: vec![bmc_grpc::web::ReleaseInfo {
                    version: "26.09".to_owned(),
                    description: "older".to_owned(),
                }],
            }),
            packages: Some(bmc_grpc::web::PackageUpgradePlan {
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
            }),
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
    assert_eq!(translated.disruption(), UpgradeDisruption::Unspecified);
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
    assert_eq!(firmware.release_date, None);
    assert_eq!(firmware.version, "26.10");
}

/// The codes are the ones Deck answers for the same condition, so one frontend handles both,
/// except that Boser folds every package source failure into `INTERNAL`.
#[test]
fn client_errors_map_to_the_codes_the_local_path_uses() {
    let rejected = |rejection| ClientError::Rejected {
        rejection,
        message: "from boser".to_owned(),
    };
    let cases = [
        (ClientError::Unreachable, Code::Unavailable),
        (ClientError::NoAnswer, Code::Unavailable),
        (ClientError::StartUnconfirmed, Code::Unavailable),
        (ClientError::Unauthenticated, Code::Unauthenticated),
        (rejected(Rejection::Busy), Code::Unavailable),
        (rejected(Rejection::Expired), Code::FailedPrecondition),
        (
            rejected(Rejection::NotEnoughSpace),
            Code::FailedPrecondition,
        ),
        (
            rejected(Rejection::PackagesUnavailable),
            Code::FailedPrecondition,
        ),
        (rejected(Rejection::InvalidArgument), Code::InvalidArgument),
        (rejected(Rejection::Internal), Code::Internal),
        (
            ClientError::Unexpected(reqwest::StatusCode::BAD_GATEWAY),
            Code::Internal,
        ),
    ];
    for (error, code) in cases {
        let described = format!("{error:?}");
        assert_eq!(Status::from(error).code(), code, "{described}");
    }
}

/// Boser's message is the only explanation the user gets.
#[test]
fn a_rejection_keeps_bosers_message() {
    let status = Status::from(ClientError::Rejected {
        rejection: Rejection::Busy,
        message: "an upgrade is already running".to_owned(),
    });

    assert_eq!(status.message(), "an upgrade is already running");
}

/// The browser is told that Boser is unreachable, not where the BMC application looked for it.
#[test]
fn an_unreachable_boser_is_reported_without_its_address() {
    let status = Status::from(ClientError::Unreachable);

    assert_eq!(status.message(), "Boser is unreachable");
}

/// An unexpected answer names the HTTP status, for the log reader.
#[test]
fn an_unexpected_answer_names_the_http_status() {
    let status = Status::from(ClientError::Unexpected(reqwest::StatusCode::BAD_GATEWAY));

    assert!(status.message().contains("502"), "{}", status.message());
}

/// A failed run is a server-side fault; a lost one clears by retrying.
#[test]
fn follow_errors_map_to_internal_and_unavailable() {
    let failed = Status::from(FollowError::Failed("no space left".to_owned()));
    assert_eq!(failed.code(), Code::Internal);
    assert_eq!(failed.message(), "no space left");

    let lost = Status::from(FollowError::Lost);
    assert_eq!(lost.code(), Code::Unavailable);
    assert!(lost.message().contains("may still be running"));
}
