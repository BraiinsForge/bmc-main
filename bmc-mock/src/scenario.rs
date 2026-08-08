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

//! Upgrade scenario selection for the mock, read from
//! `<mockfs>/etc/upgrade-scenario.json` on every check so states can be
//! flipped at runtime without restarting.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FirmwareScenario {
    #[default]
    Available,
    UpToDate,
    CheckError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackagesScenario {
    #[default]
    Available,
    Unavailable,
    /// Transient probe failure (index fetch) — surfaces as `Internal`.
    FetchFailed,
    /// Non-transient probe failure (no enabled servers) — surfaces as
    /// `FailedPrecondition`.
    PreconditionFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunScenario {
    #[default]
    Success,
    DownloadFail,
    HashMismatch,
    ApplyFail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackageUpgradeAction {
    #[default]
    Nothing,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpgradeScenario {
    pub firmware: FirmwareScenario,
    pub packages: PackagesScenario,
    pub run: RunScenario,
    pub package_action: PackageUpgradeAction,
    pub shadowed_packages: Vec<String>,
    /// Free store bytes the mock reports. `None` leaves the store
    /// unconstrained: the mock filesystem has no capacity to measure,
    /// so only an explicit value can drive the daemon's space preflight.
    pub store_free_bytes: Option<u64>,
}

#[must_use]
pub fn read(path: &Path) -> UpgradeScenario {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return UpgradeScenario::default();
    };
    match serde_json::from_str(&contents) {
        Ok(scenario) => scenario,
        Err(err) => {
            warn!(error = %err, path = %path.display(), "Unparseable upgrade scenario, using defaults");
            UpgradeScenario::default()
        }
    }
}

/// Rewrite the scenario file with `installed` removed from the shadow list,
/// modelling a completed install so discovery no longer offers them.
pub fn unshadow(path: &Path, installed: &[String]) -> Result<(), String> {
    let mut scenario = read(path);
    scenario
        .shadowed_packages
        .retain(|pkg| !installed.contains(pkg));
    let json = serde_json::to_string_pretty(&scenario)
        .map_err(|err| format!("serialize scenario: {err}"))?;
    std::fs::write(path, json)
        .map_err(|err| format!("persist unshadow to {}: {err}", path.display()))
}

/// Consume a pending-install handoff written by bmc: unshadow the named
/// packages in the scenario (modelling their install) and remove the file.
/// Called on a successful firmware upgrade before the mock exits to reboot.
pub fn consume_pending_install(handoff_path: &Path, scenario_path: &Path) {
    let pending = match bmc_nix::pending_install::read_pending_install(handoff_path) {
        Ok(pending) => pending,
        // No handoff pending — the normal case for a firmware-only upgrade.
        Err(bmc_nix::pending_install::PendingInstallError::Io(err))
            if err.kind() == std::io::ErrorKind::NotFound =>
        {
            return;
        }
        // A present-but-unreadable/corrupt handoff is a dropped install, not
        // the normal absent case: surface it and remove the bad file so a
        // later run cannot consume the same broken handoff.
        Err(err) => {
            warn!(error = %err, path = %handoff_path.display(),
                "Discarding unreadable pending-install handoff");
            if let Err(err) = std::fs::remove_file(handoff_path) {
                warn!(error = %err, path = %handoff_path.display(),
                    "Failed to remove unreadable handoff");
            }
            return;
        }
    };
    // Best-effort: this runs at exit-time before the simulated reboot, so
    // there is no run stream left to fail into — log and continue.
    if let Err(err) = unshadow(scenario_path, &pending.install) {
        warn!(error = %err, "Failed to unshadow consumed pending install");
    }
    if let Err(err) = std::fs::remove_file(handoff_path) {
        warn!(error = %err, path = %handoff_path.display(), "Failed to remove consumed handoff");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_file_missing() {
        let scenario = read(std::path::Path::new("/nonexistent/upgrade-scenario.json"));
        assert_eq!(scenario.firmware, FirmwareScenario::Available);
        assert_eq!(scenario.packages, PackagesScenario::Available);
        assert_eq!(scenario.run, RunScenario::Success);
        assert_eq!(scenario.package_action, PackageUpgradeAction::Nothing);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("upgrade-scenario.json");
        std::fs::write(&path, r#"{"packages": "fetch-failed"}"#).expect("BUG: write scenario");
        let scenario = read(&path);
        assert_eq!(scenario.firmware, FirmwareScenario::Available);
        assert_eq!(scenario.packages, PackagesScenario::FetchFailed);
        assert_eq!(scenario.run, RunScenario::Success);
        assert_eq!(scenario.package_action, PackageUpgradeAction::Nothing);
    }

    #[test]
    fn garbage_falls_back_to_defaults() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("upgrade-scenario.json");
        std::fs::write(&path, "not json at all").expect("BUG: write scenario");
        assert_eq!(read(&path), UpgradeScenario::default());
    }

    #[test]
    fn parses_shadowed_packages_and_defaults_empty() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("upgrade-scenario.json");
        std::fs::write(&path, r#"{"shadowed_packages": ["widget-flip-clock"]}"#)
            .expect("BUG: write scenario");
        assert_eq!(
            read(&path).shadowed_packages,
            vec!["widget-flip-clock".to_owned()]
        );
        std::fs::write(&path, r#"{"packages": "available"}"#).expect("BUG: write scenario");
        assert!(read(&path).shadowed_packages.is_empty());
    }

    #[test]
    fn unshadow_removes_only_named_packages() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("upgrade-scenario.json");
        std::fs::write(
            &path,
            r#"{"shadowed_packages": ["widget-flip-clock", "widget-weather"]}"#,
        )
        .expect("BUG: write scenario");
        unshadow(&path, &["widget-flip-clock".to_owned()]).expect("BUG: unshadow failed");
        assert_eq!(
            read(&path).shadowed_packages,
            vec!["widget-weather".to_owned()]
        );
    }

    #[test]
    fn consume_pending_install_unshadows_and_clears() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let scenario_path = dir.path().join("upgrade-scenario.json");
        std::fs::write(
            &scenario_path,
            r#"{"shadowed_packages": ["widget-flip-clock", "widget-weather"]}"#,
        )
        .expect("BUG: write scenario");
        let handoff = dir.path().join("pending.json");
        std::fs::write(&handoff, r#"{"install": ["widget-flip-clock"]}"#)
            .expect("BUG: write handoff");

        consume_pending_install(&handoff, &scenario_path);

        assert_eq!(
            read(&scenario_path).shadowed_packages,
            vec!["widget-weather".to_owned()]
        );
        assert!(!handoff.exists(), "handoff should be deleted after consume");
    }

    #[test]
    fn full_file_parses_all_fields() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("upgrade-scenario.json");
        std::fs::write(
            &path,
            r#"{"firmware": "check-error", "packages": "unavailable", "run": "hash-mismatch"}"#,
        )
        .expect("BUG: write scenario");
        let scenario = read(&path);
        assert_eq!(scenario.firmware, FirmwareScenario::CheckError);
        assert_eq!(scenario.packages, PackagesScenario::Unavailable);
        assert_eq!(scenario.run, RunScenario::HashMismatch);
        assert_eq!(scenario.package_action, PackageUpgradeAction::Nothing);
    }

    #[test]
    fn package_action_values_parse() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("upgrade-scenario.json");

        std::fs::write(&path, r#"{"package_action":"restart"}"#)
            .expect("BUG: write restart scenario");
        assert_eq!(read(&path).package_action, PackageUpgradeAction::Restart);

        std::fs::write(&path, r#"{"package_action":"nothing"}"#)
            .expect("BUG: write nothing scenario");
        assert_eq!(read(&path).package_action, PackageUpgradeAction::Nothing);
    }
}
