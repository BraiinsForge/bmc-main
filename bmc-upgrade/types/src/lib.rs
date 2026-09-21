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

//! Upgrade metadata, execution state and wire types independent of Nix.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod wire;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UpgradeKind {
    Firmware,
    FirmwareAndPackages,
    Packages,
}

impl UpgradeKind {
    #[must_use]
    pub fn disruption(self) -> Disruption {
        match self {
            Self::Firmware | Self::FirmwareAndPackages => Disruption::Reboot,
            Self::Packages => Disruption::AppRestart,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Disruption {
    Reboot,
    AppRestart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FirmwarePhase {
    Downloading,
    Verifying,
    Flashing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PackagePhase {
    Realizing,
    Verifying,
    Building,
    Activating,
    Cleaning,
    FindingGarbageRoots,
    DeterminingGarbageLiveness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", content = "step", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UpgradePhase {
    Preparing,
    Firmware(FirmwarePhase),
    Packages(PackagePhase),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionId(Uuid);

impl ExecutionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ExecutionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ExecutionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ExecutionId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UpgradeState {
    None,
    DownloadingImage {
        download: DownloadProgress,
    },
    DownloadFailed {
        reason: String,
    },
    Running {
        id: ExecutionId,
        kind: UpgradeKind,
        phase: UpgradePhase,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        download: Option<DownloadProgress>,
    },
    Rebooting {
        id: ExecutionId,
        kind: UpgradeKind,
    },
    Completed {
        id: ExecutionId,
        kind: UpgradeKind,
    },
    Failed {
        id: ExecutionId,
        kind: UpgradeKind,
        phase: UpgradePhase,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackagesPreview {
    pub changes: Vec<PackageChange>,
    pub download_size_bytes: Option<u64>,
    /// Unpacked (NAR) size the realization would add to the store.
    pub unpacked_size_bytes: Option<u64>,
    pub bmc_version: Option<String>,
    pub bmc_changelog: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageChange {
    pub name: String,
    pub version_from: Option<String>,
    pub version_to: Option<String>,
    pub category: Option<String>,
    pub changelog: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallablePackage {
    pub name: String,
    pub version: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests;
