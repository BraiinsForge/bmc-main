// Copyright (C) 2026  Braiins Systems s.r.o.

//! Scenario-driven [`PackageBackend`] serving static package-upgrade data
//! so the frontend can exercise package flows without nix or network.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use bmc_nix::index::merge_indexes;
use bmc_nix::store::progress::DownloadSnapshot;
use bmc_nix::types::{FetchedIndex, MergedIndex, PackageIndex};
use bmc_nix::upgrade::{UpgradePhase, UpgradeProgress};
use bmc_upgrade::packages::{
    ApplyError, EstimateMode, InstallableCategory, InstallableWidget, PackageBackend, PackageProbe,
    PackageProbeError, PackagesPreview, SystemPackageChange, installable_widgets_from,
};
use bmc_widget_manifest::Manifest;
use walkdir::WalkDir;

use crate::pacing::UpgradePacing;
use crate::scenario::{self, PackagesScenario, RunScenario};

const DOWNLOAD_TOTAL_BYTES: u64 = 4_000_000;

/// Cap on a package-index file the mock will read, mirroring bmc-nix's
/// index fetch. A package index is small; this bounds memory on a stray or
/// oversized path.
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub struct MockPackageBackend {
    scenario_path: PathBuf,
    pacing: UpgradePacing,
    index_path: Option<PathBuf>,
    widgets_path: Option<PathBuf>,
}

impl MockPackageBackend {
    #[must_use]
    pub fn new(scenario_path: PathBuf, pacing: UpgradePacing) -> Self {
        Self {
            scenario_path,
            pacing,
            index_path: None,
            widgets_path: None,
        }
    }

    #[must_use]
    pub fn with_package_index(mut self, index_path: Option<PathBuf>) -> Self {
        self.index_path = index_path;
        self
    }

    /// Point the fallback catalog at the widget tree the mock renders from
    /// (its `--widgets-path`), so the installable set mirrors the widgets
    /// actually present instead of a fabricated list.
    #[must_use]
    pub fn with_widgets_path(mut self, widgets_path: Option<PathBuf>) -> Self {
        self.widgets_path = widgets_path;
        self
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

/// Derive the fallback installable catalog from the widget tree the mock
/// renders from (its `--widgets-path`). Each `<name>/manifest.json` becomes
/// an [`InstallableWidget`] named `widget-<name>`, mirroring the nix package
/// convention; the shadow gate then decides which are offered. Manifests
/// carry no previews, so those stay empty.
fn installable_widgets_from_dir(root: &Path) -> Vec<InstallableWidget> {
    WalkDir::new(root)
        .max_depth(3)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == "manifest.json")
        .filter_map(|entry| widget_from_manifest(entry.path()))
        .collect()
}

/// Parse one widget `manifest.json` into an [`InstallableWidget`], resolving
/// the manifest-relative icon and inlining it. Returns `None` on a manifest
/// that will not parse rather than failing the whole listing.
fn widget_from_manifest(manifest_path: &Path) -> Option<InstallableWidget> {
    let widget_dir = manifest_path.parent()?;
    let package_name = format!("widget-{}", widget_dir.file_name()?.to_str()?);
    let manifest: Manifest = std::fs::read_to_string(manifest_path).ok()?.parse().ok()?;
    let icon = manifest
        .icon
        .map(|rel| widget_dir.join(rel).to_string_lossy().into_owned());
    Some(inline_widget_icon(InstallableWidget {
        package_name,
        uid: manifest.uid.to_string(),
        version: manifest.version.to_string(),
        display_name: manifest.name,
        subname: manifest.subname,
        category: InstallableCategory::Known(manifest.category),
        description: Some(manifest.description),
        icon,
        previews: Vec::new(),
    }))
}

/// `installable_widgets_from` carries the icon as a raw store-path string,
/// which the frontend cannot render; inline it as a `data:` URI, dropping to
/// `None` when the file is absent or unreadable rather than failing discovery.
fn inline_widget_icon(mut widget: InstallableWidget) -> InstallableWidget {
    widget.icon = widget.icon.and_then(|path| {
        let meta = std::fs::metadata(&path).ok()?;
        if !meta.is_file() || meta.len() > MAX_INDEX_BYTES {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        let mime = match std::path::Path::new(&path)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
        {
            Some("svg") => "image/svg+xml",
            Some("png") => "image/png",
            _ => "application/octet-stream",
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Some(format!("data:{mime};base64,{encoded}"))
    });
    widget
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
        }
    }

    async fn apply(
        &self,
        _merged: MergedIndex,
        install: Vec<String>,
        progress: Arc<dyn UpgradeProgress>,
    ) -> Result<(), ApplyError> {
        let run = scenario::read(&self.scenario_path).run;
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

        if run == RunScenario::ApplyFail {
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
        Ok(())
    }

    async fn list_installable_widgets(&self) -> Result<Vec<InstallableWidget>, PackageProbeError> {
        let scenario = scenario::read(&self.scenario_path);
        if scenario.packages == PackagesScenario::FetchFailed {
            return Err(PackageProbeError::IndexFetchFailed(
                "mock: index fetch failed".to_owned(),
            ));
        }
        let widgets: Vec<InstallableWidget> = match &self.index_path {
            None => self
                .widgets_path
                .as_deref()
                .map(installable_widgets_from_dir)
                .unwrap_or_default()
                .into_iter()
                .filter(|w| scenario.shadowed_packages.contains(&w.package_name))
                .collect(),
            Some(path) => {
                let meta = std::fs::metadata(path).map_err(|err| {
                    PackageProbeError::IndexFetchFailed(format!(
                        "mock: cannot read package index {}: {err}",
                        path.display()
                    ))
                })?;
                if !meta.is_file() {
                    return Err(PackageProbeError::IndexFetchFailed(format!(
                        "mock: package index {} is not a regular file",
                        path.display()
                    )));
                }
                if meta.len() > MAX_INDEX_BYTES {
                    return Err(PackageProbeError::IndexFetchFailed(format!(
                        "mock: package index {} is too large: {} bytes exceeds the \
                         {MAX_INDEX_BYTES}-byte cap",
                        path.display(),
                        meta.len()
                    )));
                }
                let bytes = std::fs::read(path).map_err(|err| {
                    PackageProbeError::IndexFetchFailed(format!(
                        "mock: cannot read package index {}: {err}",
                        path.display()
                    ))
                })?;
                let index: PackageIndex = serde_json::from_slice(&bytes).map_err(|err| {
                    PackageProbeError::IndexFetchFailed(format!(
                        "mock: invalid package index {}: {err}",
                        path.display()
                    ))
                })?;
                if index.version != bmc_nix::index::PACKAGE_INDEX_VERSION {
                    return Err(PackageProbeError::IndexFetchFailed(format!(
                        "mock: unsupported package index version {} (expected {})",
                        index.version,
                        bmc_nix::index::PACKAGE_INDEX_VERSION
                    )));
                }
                let merged = merge_indexes(vec![FetchedIndex {
                    server_id: "mock".to_owned(),
                    server_priority: 0,
                    index,
                }]);
                installable_widgets_from(&merged, &BTreeSet::new())
                    .into_iter()
                    .filter(|w| scenario.shadowed_packages.contains(&w.package_name))
                    .map(inline_widget_icon)
                    .collect()
            }
        };
        Ok(widgets)
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

    fn write_index(dir: &std::path::Path, contents: &str) -> std::path::PathBuf {
        let path = dir.join("nix-package-index.v1.json");
        std::fs::write(&path, contents).expect("BUG: write index");
        path
    }

    /// Write a `<name>/manifest.json` widget tree under `dir` and return its
    /// root, to stand in for the mock's `--widgets-path`.
    fn write_widget_tree(dir: &std::path::Path, names: &[&str]) -> std::path::PathBuf {
        let root = dir.join("widgets");
        for name in names {
            let widget_dir = root.join(name);
            std::fs::create_dir_all(&widget_dir).expect("BUG: create widget dir");
            std::fs::write(
                widget_dir.join("manifest.json"),
                format!(
                    r#"{{"uid":"7cb584a8-1f26-42a0-867e-955aadd2391c","version":"1.0.0",
                        "name":"{name}","description":"A {name} widget.","binary":"bin/{name}",
                        "category":"clock","supported_viewports":[{{"type":"rectangular",
                        "min_width":100,"max_width":200,"min_height":100,"max_height":200}}]}}"#
                ),
            )
            .expect("BUG: write manifest");
        }
        root
    }

    #[tokio::test]
    async fn probe_maps_scenarios() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");

        let path = write_scenario(dir.path(), r#"{"packages": "available"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant);
        let PackageProbe::Available(_, preview) = backend.probe(EstimateMode::Estimate, &[]).await
        else {
            panic!("BUG: expected Available");
        };
        assert!(!preview.changes.is_empty());
        assert!(preview.bmc_version.is_some());
        assert!(preview.download_size_bytes.is_some());

        let path = write_scenario(dir.path(), r#"{"packages": "unavailable"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant);
        assert!(matches!(
            backend.probe(EstimateMode::Estimate, &[]).await,
            PackageProbe::UpToDate
        ));

        let path = write_scenario(dir.path(), r#"{"packages": "fetch-failed"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant);
        assert!(matches!(
            backend.probe(EstimateMode::Estimate, &[]).await,
            PackageProbe::Failed(PackageProbeError::IndexFetchFailed(_))
        ));
    }

    #[tokio::test]
    async fn probe_skips_size_estimate_when_asked() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), r#"{"packages": "available"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant);
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
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant);
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
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant);
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
        let widgets = write_widget_tree(dir.path(), &["flip-clock", "weather"]);
        let path = write_scenario(
            dir.path(),
            r#"{"packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#,
        );
        let backend =
            MockPackageBackend::new(path, UpgradePacing::Instant).with_widgets_path(Some(widgets));
        let widgets = backend
            .list_installable_widgets()
            .await
            .expect("BUG: list failed");
        // Only the shadowed widget is offered, derived from its manifest in
        // the widget tree — not from a fabricated list.
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].package_name, "widget-flip-clock");
        assert!(!widgets[0].uid.is_empty());
    }

    #[tokio::test]
    async fn lists_nothing_when_no_packages_shadowed() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let widgets = write_widget_tree(dir.path(), &["flip-clock"]);
        let path = write_scenario(dir.path(), r#"{"packages": "available"}"#);
        let backend =
            MockPackageBackend::new(path, UpgradePacing::Instant).with_widgets_path(Some(widgets));
        // The widget is present in the tree but not shadowed: the shadow gate,
        // not an empty tree, produces the empty list.
        assert!(
            backend
                .list_installable_widgets()
                .await
                .expect("BUG: list failed")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn real_index_lists_real_widgets_filtered_by_shadow_set() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let index = write_index(
            dir.path(),
            r#"{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[
              {"name":"widget-flip-clock","version":"1.0.0","store_path":"/nix/store/f",
               "category":"widget",
               "metadata":{"widget":{"uid":"flip-clock","display_name":"Flip Clock","category":"clock"}}},
              {"name":"widget-weather","version":"1.3.0","store_path":"/nix/store/w",
               "category":"widget",
               "metadata":{"widget":{"uid":"weather","display_name":"Weather","category":"info"}}}
            ]}"#,
        );
        let scenario = write_scenario(
            dir.path(),
            r#"{"packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#,
        );
        let backend = MockPackageBackend::new(scenario, UpgradePacing::Instant)
            .with_package_index(Some(index));
        let widgets = backend
            .list_installable_widgets()
            .await
            .expect("BUG: list failed");
        // The real mapping and the shadow gate both apply: only the shadowed
        // widget is offered, and its uid is the real one carried by the index.
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].package_name, "widget-flip-clock");
        assert_eq!(widgets[0].uid, "flip-clock");
        assert!(!widgets.iter().any(|w| w.package_name == "widget-weather"));
    }

    #[tokio::test]
    async fn inlines_icon_from_a_real_file_as_data_uri() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let icon = dir.path().join("icon.svg");
        std::fs::write(&icon, "<svg/>").expect("BUG: write icon");
        let index = write_index(
            dir.path(),
            &format!(
                r#"{{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[
                  {{"name":"widget-flip-clock","version":"1.0.0","store_path":"/nix/store/f",
                   "category":"widget",
                   "metadata":{{"widget":{{"uid":"flip-clock","display_name":"Flip Clock"}},
                               "assets":{{"icon":"{}"}}}}}}
                ]}}"#,
                icon.display()
            ),
        );
        let scenario = write_scenario(
            dir.path(),
            r#"{"packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#,
        );
        let backend = MockPackageBackend::new(scenario, UpgradePacing::Instant)
            .with_package_index(Some(index));
        let widgets = backend
            .list_installable_widgets()
            .await
            .expect("BUG: list failed");
        assert_eq!(widgets.len(), 1);
        // A real on-disk asset becomes a renderable data: URI, not a store path.
        let icon = widgets[0].icon.as_deref().expect("BUG: icon inlined");
        assert!(icon.starts_with("data:image/svg+xml;base64,"), "got {icon}");
    }

    #[tokio::test]
    async fn drops_unreadable_icon_without_failing_listing() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let index = write_index(
            dir.path(),
            r#"{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[
              {"name":"widget-flip-clock","version":"1.0.0","store_path":"/nix/store/f",
               "category":"widget",
               "metadata":{"widget":{"uid":"flip-clock","display_name":"Flip Clock"},
                           "assets":{"icon":"/nonexistent/icon.svg"}}}
            ]}"#,
        );
        let scenario = write_scenario(
            dir.path(),
            r#"{"packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#,
        );
        let backend = MockPackageBackend::new(scenario, UpgradePacing::Instant)
            .with_package_index(Some(index));
        // A bad icon path must not break discovery: the widget still lists,
        // just without an icon.
        let widgets = backend
            .list_installable_widgets()
            .await
            .expect("BUG: list should succeed despite unreadable icon");
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].icon, None);
    }

    #[tokio::test]
    async fn unsupported_index_version_is_rejected() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let index = write_index(
            dir.path(),
            r#"{"version":2,"provenance":null,"indexes":[],"caches":[],"packages":[
              {"name":"widget-flip-clock","version":"1.0.0","store_path":"/nix/store/f",
               "category":"widget",
               "metadata":{"widget":{"uid":"flip-clock","display_name":"Flip Clock"}}}
            ]}"#,
        );
        let scenario = write_scenario(
            dir.path(),
            r#"{"packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#,
        );
        let backend = MockPackageBackend::new(scenario, UpgradePacing::Instant)
            .with_package_index(Some(index));
        // An index format the mock doesn't understand must diverge the same way
        // the real backend does, not be served silently.
        assert!(matches!(
            backend.list_installable_widgets().await,
            Err(PackageProbeError::IndexFetchFailed(_))
        ));
    }

    #[tokio::test]
    async fn non_file_index_path_is_rejected() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let scenario = write_scenario(
            dir.path(),
            r#"{"packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#,
        );
        // A directory is not a regular file: the mock reports it, never reads it.
        let backend = MockPackageBackend::new(scenario, UpgradePacing::Instant)
            .with_package_index(Some(dir.path().to_path_buf()));
        assert!(matches!(
            backend.list_installable_widgets().await,
            Err(PackageProbeError::IndexFetchFailed(_))
        ));
    }

    #[tokio::test]
    async fn icon_pointing_at_a_directory_drops_to_none() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let icon_dir = dir.path().join("icon-dir");
        std::fs::create_dir(&icon_dir).expect("BUG: create icon dir");
        let index = write_index(
            dir.path(),
            &format!(
                r#"{{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[
                  {{"name":"widget-flip-clock","version":"1.0.0","store_path":"/nix/store/f",
                   "category":"widget",
                   "metadata":{{"widget":{{"uid":"flip-clock","display_name":"Flip Clock"}},
                               "assets":{{"icon":"{}"}}}}}}
                ]}}"#,
                icon_dir.display()
            ),
        );
        let scenario = write_scenario(
            dir.path(),
            r#"{"packages": "available", "shadowed_packages": ["widget-flip-clock"]}"#,
        );
        let backend = MockPackageBackend::new(scenario, UpgradePacing::Instant)
            .with_package_index(Some(index));
        // A non-file icon path can never break discovery: the widget still lists,
        // and the is_file guard drops the icon to None.
        let widgets = backend
            .list_installable_widgets()
            .await
            .expect("BUG: list should succeed despite directory icon");
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].package_name, "widget-flip-clock");
        assert_eq!(widgets[0].icon, None);
    }

    #[tokio::test]
    async fn apply_fails_after_realizing_on_apply_fail() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), r#"{"run": "apply-fail"}"#);
        let backend = MockPackageBackend::new(path, UpgradePacing::Instant);
        let progress = std::sync::Arc::new(RecordingProgress::default());
        let result = backend
            .apply(empty_merged_index(), Vec::new(), progress.clone())
            .await;
        assert!(result.is_err());
        let phases = progress.phases.lock().expect("BUG: phases mutex").clone();
        assert_eq!(phases, vec![UpgradePhase::Realizing]);
    }
}
