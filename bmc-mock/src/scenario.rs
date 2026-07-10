// Copyright (C) 2026  Braiins Systems s.r.o.

//! Upgrade scenario selection for the mock, read from
//! `<mockfs>/etc/upgrade-scenario.json` on every check so states can be
//! flipped at runtime without restarting.

use std::path::Path;

use serde::Deserialize;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FirmwareScenario {
    #[default]
    Available,
    UpToDate,
    CheckError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunScenario {
    #[default]
    Success,
    DownloadFail,
    HashMismatch,
    ApplyFail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackageUpgradeAction {
    #[default]
    Nothing,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct UpgradeScenario {
    pub firmware: FirmwareScenario,
    pub packages: PackagesScenario,
    pub run: RunScenario,
    pub package_action: PackageUpgradeAction,
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
