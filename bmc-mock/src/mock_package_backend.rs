// Copyright (C) 2026  Braiins Systems s.r.o.

//! Scenario-driven [`PackageBackend`] serving static package-upgrade data
//! so the frontend can exercise package flows without nix or network.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use bmc_nix::store::progress::DownloadSnapshot;
use bmc_nix::types::MergedIndex;
use bmc_nix::upgrade::{UpgradePhase, UpgradeProgress};
use bmc_upgrade::packages::{
    ApplyError, EstimateMode, InstallablePreview, InstallableWidget, PackageBackend, PackageProbe,
    PackageProbeError, PackagesPreview, SystemPackageChange,
};
use tokio::sync::Notify;

use crate::pacing::UpgradePacing;
use crate::scenario::{self, PackageUpgradeAction, PackagesScenario, RunScenario};

const DOWNLOAD_TOTAL_BYTES: u64 = 4_000_000;

#[derive(Debug)]
pub struct MockPackageBackend {
    scenario_path: PathBuf,
    pacing: UpgradePacing,
    stop: Arc<Notify>,
}

impl MockPackageBackend {
    #[must_use]
    pub fn new(scenario_path: PathBuf, pacing: UpgradePacing, stop: Arc<Notify>) -> Self {
        Self {
            scenario_path,
            pacing,
            stop,
        }
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

/// A tiny inline SVG served as a `data:` icon so the picker renders
/// something without an on-disk asset.
fn swatch_icon(fill: &str) -> String {
    format!(
        "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><rect width='24' height='24' rx='4' fill='{fill}'/></svg>"
    )
}

/// The built-in set of widgets the mock can offer. `widget-flip-clock`
/// ships in the init image; the rest exist only to populate the picker.
fn widget_catalog() -> Vec<InstallableWidget> {
    vec![
        InstallableWidget {
            package_name: "widget-flip-clock".to_owned(),
            uid: "flip-clock".to_owned(),
            version: "1.0.0".to_owned(),
            display_name: "Flip Clock".to_owned(),
            subname: None,
            category: Some("clock".to_owned()),
            description: Some("A retro split-flap clock face.".to_owned()),
            icon: Some(swatch_icon("%23f2a900")),
            previews: vec![InstallablePreview {
                image: swatch_icon("%23222222"),
            }],
        },
        InstallableWidget {
            package_name: "widget-weather".to_owned(),
            uid: "weather".to_owned(),
            version: "1.3.0".to_owned(),
            display_name: "Weather".to_owned(),
            subname: Some("Local forecast".to_owned()),
            category: Some("info".to_owned()),
            description: Some("Current conditions and a short forecast.".to_owned()),
            icon: Some(swatch_icon("%234a90d9")),
            previews: vec![
                InstallablePreview {
                    image: swatch_icon("%23a0d8ef"),
                },
                InstallablePreview {
                    image: swatch_icon("%23557799"),
                },
            ],
        },
        InstallableWidget {
            package_name: "widget-mining-clock".to_owned(),
            uid: "mining-clock".to_owned(),
            version: "0.9.0".to_owned(),
            display_name: "Mining Clock".to_owned(),
            subname: None,
            category: Some("mining".to_owned()),
            description: Some("Hashrate and power at a glance.".to_owned()),
            icon: Some(swatch_icon("%2300a86b")),
            previews: Vec::new(),
        },
    ]
}

#[async_trait::async_trait]
impl PackageBackend for MockPackageBackend {
    async fn probe(&self, estimate: EstimateMode, install: &[String]) -> PackageProbe {
        match scenario::read(&self.scenario_path).packages {
            PackagesScenario::Available => {
                let mut preview = static_preview(estimate);
                for name in install {
                    preview.changes.push(SystemPackageChange {
                        name: name.clone(),
                        version_from: None,
                        version_to: Some("1.0.0".to_owned()),
                        category: Some("widget".to_owned()),
                        changelog: None,
                    });
                }
                PackageProbe::Available(empty_merged_index(), preview)
            }
            PackagesScenario::Unavailable => PackageProbe::UpToDate,
            PackagesScenario::FetchFailed => PackageProbe::Failed(
                PackageProbeError::IndexFetchFailed("mock: index fetch failed".to_owned()),
            ),
            PackagesScenario::PreconditionFailed => {
                PackageProbe::Failed(PackageProbeError::NoEnabledServers)
            }
        }
    }

    async fn apply(
        &self,
        _merged: MergedIndex,
        install: Vec<String>,
        progress: Arc<dyn UpgradeProgress>,
    ) -> Result<(), ApplyError> {
        let scenario = scenario::read(&self.scenario_path);
        let step_delay = self.pacing.progress_step();

        progress.on_phase(UpgradePhase::Realizing);
        for downloaded_bytes in [1_000_000, 2_500_000, DOWNLOAD_TOTAL_BYTES] {
            progress.on_download_status(&DownloadSnapshot {
                active: Vec::new(),
                downloaded_bytes,
                total_bytes: Some(DOWNLOAD_TOTAL_BYTES),
                remaining_bytes: Some(DOWNLOAD_TOTAL_BYTES - downloaded_bytes),
            });
            tokio::time::sleep(step_delay).await;
        }

        if scenario.run == RunScenario::ApplyFail {
            return Err(ApplyError("mock: package apply failed".to_owned()));
        }

        for phase in [
            UpgradePhase::Verifying,
            UpgradePhase::Building,
            UpgradePhase::Activating,
        ] {
            progress.on_phase(phase);
            tokio::time::sleep(step_delay).await;
        }

        scenario::unshadow(&self.scenario_path, &install).map_err(ApplyError)?;
        if scenario.package_action == PackageUpgradeAction::Restart {
            // The notifier models bmc-openwrt receiving procd's SIGTERM from
            // the external service orchestrator once a packages-only
            // activation lands; the mock never signals itself.
            let stop = Arc::clone(&self.stop);
            let delay = self.pacing.shutdown_delay();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                stop.notify_one();
            });
        }

        Ok(())
    }

    async fn list_installable_widgets(&self) -> Result<Vec<InstallableWidget>, PackageProbeError> {
        let scenario = scenario::read(&self.scenario_path);
        if scenario.packages == PackagesScenario::FetchFailed {
            return Err(PackageProbeError::IndexFetchFailed(
                "mock: index fetch failed".to_owned(),
            ));
        }
        Ok(widget_catalog()
            .into_iter()
            .filter(|w| scenario.shadowed_packages.contains(&w.package_name))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_nix::store::progress::DownloadSnapshot;
    use bmc_nix::upgrade::UpgradePhase;
    use std::sync::Mutex;
    use std::time::Duration;

    fn notifier() -> Arc<Notify> {
        Arc::new(Notify::new())
    }

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
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier());
        let PackageProbe::Available(_, preview) = backend.probe(EstimateMode::Estimate, &[]).await
        else {
            panic!("BUG: expected Available");
        };
        assert!(!preview.changes.is_empty());
        assert!(preview.bmc_version.is_some());
        assert!(preview.download_size_bytes.is_some());

        let path = write_scenario(dir.path(), r#"{"packages": "unavailable"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier());
        assert!(matches!(
            backend.probe(EstimateMode::Estimate, &[]).await,
            PackageProbe::UpToDate
        ));

        let path = write_scenario(dir.path(), r#"{"packages": "fetch-failed"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier());
        assert!(matches!(
            backend.probe(EstimateMode::Estimate, &[]).await,
            PackageProbe::Failed(PackageProbeError::IndexFetchFailed(_))
        ));

        let path = write_scenario(dir.path(), r#"{"packages": "precondition-failed"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier());
        assert!(matches!(
            backend.probe(EstimateMode::Estimate, &[]).await,
            PackageProbe::Failed(PackageProbeError::NoEnabledServers)
        ));
    }

    #[tokio::test]
    async fn probe_skips_size_estimate_when_asked() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), r#"{"packages": "available"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier());
        let PackageProbe::Available(_, preview) = backend.probe(EstimateMode::Skip, &[]).await
        else {
            panic!("BUG: expected Available");
        };
        assert!(preview.download_size_bytes.is_none());
    }

    #[tokio::test]
    async fn probe_surfaces_requested_installs_as_added() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), r#"{"packages": "available"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier());
        let PackageProbe::Available(_, preview) = backend
            .probe(EstimateMode::Skip, &["widget-flip-clock".to_owned()])
            .await
        else {
            panic!("BUG: expected an available probe");
        };
        let added = preview
            .changes
            .iter()
            .find(|c| c.name == "widget-flip-clock")
            .expect("BUG: install not in preview");
        assert_eq!(added.version_from, None);
    }

    #[tokio::test]
    async fn apply_walks_all_phases_and_finishes() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), r#"{"run": "success"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier());
        let progress = std::sync::Arc::new(RecordingProgress::default());
        backend
            .apply(empty_merged_index(), Vec::new(), progress.clone())
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
    async fn lists_shadowed_widgets_as_installable() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(
            dir.path(),
            r#"{"packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#,
        );
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier());
        let widgets = backend
            .list_installable_widgets()
            .await
            .expect("BUG: list failed");
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].package_name, "widget-flip-clock");
        assert!(!widgets[0].uid.is_empty());
    }

    #[tokio::test]
    async fn lists_nothing_when_no_packages_shadowed() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), r#"{"packages": "available"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier());
        assert!(
            backend
                .list_installable_widgets()
                .await
                .expect("BUG: list failed")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn apply_fails_after_realizing_on_apply_fail() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), r#"{"run": "apply-fail"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, notifier());
        let progress = std::sync::Arc::new(RecordingProgress::default());
        let result = backend
            .apply(empty_merged_index(), Vec::new(), progress.clone())
            .await;
        assert!(result.is_err());
        let phases = progress.phases.lock().expect("BUG: phases mutex").clone();
        assert_eq!(phases, vec![UpgradePhase::Realizing]);
    }

    #[tokio::test]
    async fn successful_restart_action_notifies_application_stop() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(
            dir.path(),
            r#"{"run":"success","package_action":"restart"}"#,
        );
        let stop = Arc::new(Notify::new());
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, Arc::clone(&stop));

        backend
            .apply(
                empty_merged_index(),
                Vec::new(),
                Arc::new(RecordingProgress::default()),
            )
            .await
            .expect("BUG: package apply should succeed");

        tokio::time::timeout(Duration::from_secs(1), stop.notified())
            .await
            .expect("restart action did not notify application stop");
    }

    #[tokio::test]
    async fn apply_failure_does_not_notify_application_stop() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(
            dir.path(),
            r#"{"run":"apply-fail","package_action":"restart"}"#,
        );
        let stop = Arc::new(Notify::new());
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant, Arc::clone(&stop));

        let result = backend
            .apply(
                empty_merged_index(),
                Vec::new(),
                Arc::new(RecordingProgress::default()),
            )
            .await;
        assert!(result.is_err());

        tokio::time::timeout(Duration::from_millis(200), stop.notified())
            .await
            .expect_err("apply failure must not notify application stop");
    }
}
