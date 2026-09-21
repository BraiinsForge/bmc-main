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

use serde_json::json;

use super::{
    Disruption, DownloadProgress, ExecutionId, FirmwarePhase, PackagePhase, UpgradeKind,
    UpgradePhase, UpgradeState,
};

fn round_trip<T>(json: &serde_json::Value) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let value =
        serde_json::from_value(json.clone()).expect("BUG: fixture follows the wire contract");
    assert_eq!(
        &serde_json::to_value(&value).expect("BUG: shared type serializes"),
        json
    );
    value
}

#[test]
fn phase_serializes_as_a_stage_step_object() {
    assert_eq!(
        round_trip::<UpgradePhase>(&json!({ "stage": "PREPARING" })),
        UpgradePhase::Preparing
    );
    assert_eq!(
        round_trip::<UpgradePhase>(&json!({ "stage": "FIRMWARE", "step": "FLASHING" })),
        UpgradePhase::Firmware(FirmwarePhase::Flashing)
    );
    assert_eq!(
        round_trip::<UpgradePhase>(
            &json!({ "stage": "PACKAGES", "step": "DETERMINING_GARBAGE_LIVENESS" })
        ),
        UpgradePhase::Packages(PackagePhase::DeterminingGarbageLiveness)
    );
}

#[test]
fn kind_decides_disruption() {
    assert_eq!(UpgradeKind::Firmware.disruption(), Disruption::Reboot);
    assert_eq!(
        UpgradeKind::FirmwareAndPackages.disruption(),
        Disruption::Reboot
    );
    assert_eq!(UpgradeKind::Packages.disruption(), Disruption::AppRestart);
    assert_eq!(
        round_trip::<UpgradeKind>(&json!("FIRMWARE_AND_PACKAGES")),
        UpgradeKind::FirmwareAndPackages
    );
}

#[test]
fn execution_id_is_a_transparent_uuid_string() {
    let id = ExecutionId::new();
    let json = serde_json::to_value(id).expect("BUG: id serializes");
    assert_eq!(json, json!(id.to_string()));
    assert_eq!(json.as_str().map(str::len), Some(36));
    assert_eq!(
        id.to_string()
            .parse::<ExecutionId>()
            .expect("BUG: own display parses"),
        id
    );
}

#[test]
fn running_state_omits_absent_download_and_keeps_present_one() {
    let id = ExecutionId::new();
    let without = json!({ "state": "RUNNING", "id": id.to_string(), "kind": "PACKAGES",
                          "phase": { "stage": "PREPARING" } });
    assert_eq!(
        round_trip::<UpgradeState>(&without),
        UpgradeState::Running {
            id,
            kind: UpgradeKind::Packages,
            phase: UpgradePhase::Preparing,
            download: None
        }
    );
    let with = json!({ "state": "RUNNING", "id": id.to_string(), "kind": "PACKAGES",
                       "phase": { "stage": "PACKAGES", "step": "REALIZING" },
                       "download": { "downloaded_bytes": 10, "total_bytes": null } });
    assert_eq!(
        round_trip::<UpgradeState>(&with),
        UpgradeState::Running {
            id,
            kind: UpgradeKind::Packages,
            phase: UpgradePhase::Packages(PackagePhase::Realizing),
            download: Some(DownloadProgress {
                downloaded_bytes: 10,
                total_bytes: None
            }),
        }
    );
}

#[test]
fn terminal_states_carry_identity() {
    let id = ExecutionId::new();
    round_trip::<UpgradeState>(&json!({ "state": "NONE" }));
    round_trip::<UpgradeState>(
        &json!({ "state": "DOWNLOADING_IMAGE", "download": { "downloaded_bytes": 1, "total_bytes": 2 } }),
    );
    round_trip::<UpgradeState>(&json!({ "state": "DOWNLOAD_FAILED", "reason": "timeout" }));
    round_trip::<UpgradeState>(
        &json!({ "state": "REBOOTING", "id": id.to_string(), "kind": "FIRMWARE" }),
    );
    round_trip::<UpgradeState>(
        &json!({ "state": "COMPLETED", "id": id.to_string(), "kind": "PACKAGES" }),
    );
    round_trip::<UpgradeState>(
        &json!({ "state": "FAILED", "id": id.to_string(), "kind": "FIRMWARE",
                                        "phase": { "stage": "FIRMWARE", "step": "VERIFYING" }, "reason": "bad hash" }),
    );
}
