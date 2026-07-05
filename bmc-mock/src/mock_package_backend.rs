// Copyright (C) 2026  Braiins Systems s.r.o.

//! Scenario-driven [`PackageBackend`] serving static package-upgrade data
//! so the frontend can exercise package flows without nix or network.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bmc_nix::store::progress::DownloadSnapshot;
use bmc_nix::types::MergedIndex;
use bmc_nix::upgrade::{UpgradePhase, UpgradeProgress};
use bmc_upgrade::packages::{
    ApplyError, EstimateMode, PackageBackend, PackageProbe, PackagesPreview, SystemPackageChange,
};

use crate::scenario::{self, PackagesScenario, RunScenario};

const STEP_DELAY: Duration = Duration::from_millis(300);
const DOWNLOAD_TOTAL_BYTES: u64 = 4_000_000;

#[derive(Debug)]
pub struct MockPackageBackend {
    scenario_path: PathBuf,
}

impl MockPackageBackend {
    #[must_use]
    pub fn new(scenario_path: PathBuf) -> Self {
        Self { scenario_path }
    }
}

fn empty_merged_index() -> MergedIndex {
    MergedIndex {
        packages: Vec::new(),
        by_name: BTreeMap::new(),
    }
}

fn static_preview(estimate: EstimateMode) -> PackagesPreview {
    let changes = vec![
        SystemPackageChange {
            name: "core".to_owned(),
            version_from: Some("26.06".to_owned()),
            version_to: Some("26.07".to_owned()),
            category: Some("system".to_owned()),
            changelog: Some(
                "- improve upgrade progress reporting\n- fix alarm scheduling around DST"
                    .to_owned(),
            ),
        },
        SystemPackageChange {
            name: "widget-weather".to_owned(),
            version_from: Some("1.2.0".to_owned()),
            version_to: Some("1.3.0".to_owned()),
            category: Some("widget".to_owned()),
            changelog: Some("- add wind speed display".to_owned()),
        },
        SystemPackageChange {
            name: "widget-mining-clock".to_owned(),
            version_from: None,
            version_to: Some("0.9.0".to_owned()),
            category: Some("widget".to_owned()),
            changelog: None,
        },
        SystemPackageChange {
            name: "widget-legacy-ticker".to_owned(),
            version_from: Some("0.4.2".to_owned()),
            version_to: None,
            category: None,
            changelog: None,
        },
    ];

    PackagesPreview {
        changes,
        download_size_bytes: match estimate {
            EstimateMode::Estimate => Some(DOWNLOAD_TOTAL_BYTES),
            EstimateMode::Skip => None,
        },
        bmc_version: Some("26.07".to_owned()),
        bmc_changelog: Some(
            "- improve upgrade progress reporting\n- fix alarm scheduling around DST".to_owned(),
        ),
    }
}

#[async_trait::async_trait]
impl PackageBackend for MockPackageBackend {
    async fn probe(&self, estimate: EstimateMode) -> PackageProbe {
        match scenario::read(&self.scenario_path).packages {
            PackagesScenario::Available => {
                PackageProbe::Available(empty_merged_index(), static_preview(estimate))
            }
            PackagesScenario::Unavailable => PackageProbe::Unavailable,
            PackagesScenario::FetchFailed => {
                PackageProbe::FetchFailed("mock: index fetch failed".to_owned())
            }
        }
    }

    async fn apply(
        &self,
        _merged: MergedIndex,
        progress: Arc<dyn UpgradeProgress>,
    ) -> Result<(), ApplyError> {
        let run = scenario::read(&self.scenario_path).run;

        progress.on_phase(UpgradePhase::Realizing);
        for downloaded_bytes in [1_000_000, 2_500_000, DOWNLOAD_TOTAL_BYTES] {
            progress.on_download_status(&DownloadSnapshot {
                active: Vec::new(),
                downloaded_bytes,
                total_bytes: Some(DOWNLOAD_TOTAL_BYTES),
                remaining_bytes: Some(DOWNLOAD_TOTAL_BYTES - downloaded_bytes),
            });
            tokio::time::sleep(STEP_DELAY).await;
        }

        if run == RunScenario::ApplyFail {
            return Err(ApplyError("mock: package apply failed".to_owned()));
        }

        for phase in [
            UpgradePhase::Verifying,
            UpgradePhase::Building,
            UpgradePhase::Activating,
        ] {
            progress.on_phase(phase);
            tokio::time::sleep(STEP_DELAY).await;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_nix::store::progress::DownloadSnapshot;
    use bmc_nix::upgrade::UpgradePhase;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct RecordingProgress {
        phases: Mutex<Vec<UpgradePhase>>,
        downloads: Mutex<usize>,
    }

    impl bmc_nix::upgrade::UpgradeProgress for RecordingProgress {
        fn on_phase(&self, phase: UpgradePhase) {
            self.phases.lock().expect("BUG: phases mutex").push(phase);
        }
        fn on_realization_started(&self, _total_paths: usize) {}
        fn on_realization_finished(&self) {}
        fn on_download_status(&self, _snapshot: &DownloadSnapshot) {
            *self.downloads.lock().expect("BUG: downloads mutex") += 1;
        }
        fn on_gc_deleted(&self, _deleted_paths: usize) {}
        fn on_gc_finished(&self, _deleted_paths: usize, _freed_bytes: Option<u64>) {}
    }

    fn write_scenario(dir: &std::path::Path, contents: &str) -> std::path::PathBuf {
        let path = dir.join("upgrade-scenario.json");
        std::fs::write(&path, contents).expect("BUG: write scenario");
        path
    }

    #[tokio::test]
    async fn probe_maps_scenarios() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");

        let path = write_scenario(dir.path(), r#"{"packages": "available"}"#);
        let backend = MockPackageBackend::new(path);
        let PackageProbe::Available(_, preview) = backend.probe(EstimateMode::Estimate).await
        else {
            panic!("BUG: expected Available");
        };
        assert!(!preview.changes.is_empty());
        assert!(preview.bmc_version.is_some());
        assert!(preview.download_size_bytes.is_some());

        let path = write_scenario(dir.path(), r#"{"packages": "unavailable"}"#);
        let backend = MockPackageBackend::new(path);
        assert!(matches!(
            backend.probe(EstimateMode::Estimate).await,
            PackageProbe::Unavailable
        ));

        let path = write_scenario(dir.path(), r#"{"packages": "fetch-failed"}"#);
        let backend = MockPackageBackend::new(path);
        assert!(matches!(
            backend.probe(EstimateMode::Estimate).await,
            PackageProbe::FetchFailed(_)
        ));
    }

    #[tokio::test]
    async fn probe_skips_size_estimate_when_asked() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), r#"{"packages": "available"}"#);
        let backend = MockPackageBackend::new(path);
        let PackageProbe::Available(_, preview) = backend.probe(EstimateMode::Skip).await else {
            panic!("BUG: expected Available");
        };
        assert!(preview.download_size_bytes.is_none());
    }

    #[tokio::test]
    async fn apply_walks_all_phases_and_finishes() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), r#"{"run": "success"}"#);
        let backend = MockPackageBackend::new(path);
        let progress = std::sync::Arc::new(RecordingProgress::default());
        backend
            .apply(empty_merged_index(), progress.clone())
            .await
            .expect("BUG: apply should succeed");
        let phases = progress.phases.lock().expect("BUG: phases mutex").clone();
        assert_eq!(
            phases,
            vec![
                UpgradePhase::Realizing,
                UpgradePhase::Verifying,
                UpgradePhase::Building,
                UpgradePhase::Activating,
            ]
        );
        assert!(*progress.downloads.lock().expect("BUG: downloads mutex") > 0);
    }

    #[tokio::test]
    async fn apply_fails_after_realizing_on_apply_fail() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), r#"{"run": "apply-fail"}"#);
        let backend = MockPackageBackend::new(path);
        let progress = std::sync::Arc::new(RecordingProgress::default());
        let result = backend.apply(empty_merged_index(), progress.clone()).await;
        assert!(result.is_err());
        let phases = progress.phases.lock().expect("BUG: phases mutex").clone();
        assert_eq!(phases, vec![UpgradePhase::Realizing]);
    }
}
