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

//! Request and response envelopes of the Boser upgrade REST API.

use serde::{Deserialize, Serialize};

use crate::{Disruption, ExecutionId, InstallablePackage, PackagesPreview, UpgradeKind};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct CheckUpgradeRequest {
    /// Packages to install on top of the upgrade;
    /// empty requests firmware plus the packages already installed.
    #[serde(default)]
    pub packages: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct CheckUpgradeResponse {
    pub offer: Option<Offer>,
    pub firmware: Option<FirmwareUpgrade>,
    pub packages: Option<PackagesPreview>,
    pub package_capability: PackageCapability,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct Offer {
    pub id: ExecutionId,
    pub kind: UpgradeKind,
    pub disruption: Disruption,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct FirmwareUpgrade {
    pub version: String,
    pub hash: String,
    /// Release date as published by the firmware index, `YYYY-MM-DD`.
    pub release_date: String,
    pub description: String,
    pub file_size_bytes: u64,
    pub previous_releases: Vec<PreviousRelease>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct PreviousRelease {
    pub version: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PackageCapability {
    Ready,
    Unsupported,
    Absent,
    Unhealthy { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct StartUpgradeRequest {
    pub offer_id: ExecutionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct InstallablePackages {
    pub packages: Vec<InstallablePackage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct AutoUpgradeStatus {
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct AutoUpgradeUpdate {
    pub enabled: bool,
}

/// Error envelope every Boser upgrade REST endpoint returns.
/// Boser serialises it from its own `ErrorResponse` in `open/boser/boser/src/api/rest/utils.rs`,
/// so this mirror exists for bmc to parse the body and the two must be kept in step by hand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

#[cfg(test)]
mod tests;
