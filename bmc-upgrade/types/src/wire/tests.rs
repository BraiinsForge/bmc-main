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

use serde_json::error::Category;
use serde_json::json;

use super::{
    AutoUpgradeStatus, AutoUpgradeUpdate, CheckUpgradeRequest, CheckUpgradeResponse, ErrorBody,
    InstallablePackages, PackageCapability, StartUpgradeRequest,
};
use crate::ExecutionId;
use crate::tests::round_trip;

const OFFER_ID: &str = "6a5fb1f4-1b6a-4a3f-9c2e-7d0d2f8a4b11";

fn offer_id() -> ExecutionId {
    OFFER_ID.parse().expect("BUG: fixture ID is a UUID")
}

#[test]
fn check_request_defaults_to_an_empty_package_list() {
    let request: CheckUpgradeRequest = serde_json::from_value(json!({ "future": true }))
        .expect("BUG: fixture follows the REST contract");

    assert!(request.packages.is_empty());
    assert_eq!(
        serde_json::to_value(request).expect("BUG: wire envelope serializes"),
        json!({ "packages": [] })
    );
}

#[test]
fn check_response_preserves_null_optional_fields() {
    let json = json!({
        "offer": null,
        "firmware": null,
        "packages": null,
        "package_capability": { "status": "ABSENT" }
    });

    round_trip::<CheckUpgradeResponse>(&json);
}

#[test]
fn check_response_preserves_firmware_and_package_metadata() {
    let json = json!({
        "offer": {
            "id": OFFER_ID,
            "kind": "FIRMWARE_AND_PACKAGES",
            "disruption": "REBOOT"
        },
        "firmware": {
            "version": "25.04.1",
            "hash": "sha256:abc",
            "release_date": "2026-09-16",
            "description": "Release notes",
            "file_size_bytes": 4_294_967_296_u64,
            "previous_releases": [
                { "version": "25.04.0", "description": "Earlier notes" }
            ]
        },
        "packages": {
            "changes": [{
                "name": "bmc",
                "version_from": null,
                "version_to": "2",
                "category": "system",
                "changelog": null
            }],
            "download_size_bytes": null,
            "unpacked_size_bytes": 12,
            "bmc_version": null,
            "bmc_changelog": "Changed"
        },
        "package_capability": { "status": "UNHEALTHY", "reason": "store unavailable" }
    });

    let response = round_trip::<CheckUpgradeResponse>(&json);

    let offer = response.offer.expect("BUG: fixture carries an offer");
    assert_eq!(offer.id, offer_id());
}

#[test]
fn every_package_capability_uses_the_tagged_rest_shape() {
    let fixtures = [
        (json!({ "status": "READY" }), PackageCapability::Ready),
        (
            json!({ "status": "UNSUPPORTED" }),
            PackageCapability::Unsupported,
        ),
        (json!({ "status": "ABSENT" }), PackageCapability::Absent),
        (
            json!({ "status": "UNHEALTHY", "reason": "broken" }),
            PackageCapability::Unhealthy {
                reason: String::from("broken"),
            },
        ),
    ];

    for (fixture, expected) in fixtures {
        assert_eq!(round_trip::<PackageCapability>(&fixture), expected);
    }
}

#[test]
fn start_request_and_error_body_ignore_unknown_fields() {
    let start: StartUpgradeRequest = serde_json::from_value(json!({
        "offer_id": OFFER_ID,
        "future": "ignored"
    }))
    .expect("BUG: fixture follows the REST contract");
    assert_eq!(start.offer_id, offer_id());

    let error: ErrorBody = serde_json::from_value(json!({
        "error": "BUSY",
        "message": "another upgrade is running",
        "future": "ignored"
    }))
    .expect("BUG: fixture follows the REST contract");
    assert_eq!(
        serde_json::to_value(error).expect("BUG: wire envelope serializes"),
        json!({ "error": "BUSY", "message": "another upgrade is running" })
    );
}

#[test]
fn installable_catalog_preserves_recursive_metadata_and_null_fields() {
    let json = json!({
        "packages": [{
            "name": "clock-widget",
            "version": "1.2.3",
            "category": null,
            "description": null,
            "metadata": {
                "enabled": true,
                "manifest": { "sizes": [1, 2], "theme": null }
            }
        }]
    });

    round_trip::<InstallablePackages>(&json);
}

#[test]
fn a_body_outside_the_contract_is_a_data_error_not_corruption() {
    let capability = serde_json::from_str::<PackageCapability>(r#"{"status":"SOMETHING_NEW"}"#)
        .expect_err("BUG: an unknown capability status is outside the REST contract");
    assert_eq!(capability.classify(), Category::Data);

    let start = serde_json::from_str::<StartUpgradeRequest>(r#"{"offer_id":"not-an-id"}"#)
        .expect_err("BUG: a non-UUID offer ID is outside the REST contract");
    assert_eq!(start.classify(), Category::Data);
}

#[test]
fn auto_upgrade_subsets_require_enabled_and_ignore_other_fields() {
    let status: AutoUpgradeStatus = serde_json::from_value(json!({
        "enabled": true,
        "schedule": { "schedule_type": null },
        "next_execution": null
    }))
    .expect("BUG: fixture follows the REST contract");
    assert_eq!(status, AutoUpgradeStatus { enabled: true });

    let update = AutoUpgradeUpdate { enabled: false };
    assert_eq!(
        serde_json::to_value(update).expect("BUG: wire envelope serializes"),
        json!({ "enabled": false })
    );
    assert!(serde_json::from_value::<AutoUpgradeUpdate>(json!({})).is_err());
}
