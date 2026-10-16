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
use std::error::Error as _;
use std::path::Path;

mod gc;

#[test]
fn unsubstitutable_estimate_is_unrealizable() {
    let err = bmc_nix::store::StorePathError::UnsubstitutablePaths {
        paths: vec!["/nix/store/aaa".to_owned(), "/nix/store/bbb".to_owned()],
    };
    assert_eq!(
        unrealizable_estimate(&err),
        Some("/nix/store/aaa, /nix/store/bbb".to_owned())
    );
}

#[test]
fn transient_estimate_errors_only_omit_the_size() {
    let unparsed = bmc_nix::store::StorePathError::EstimateSummaryUnparsed {
        line: "garbage".to_owned(),
    };
    assert_eq!(unrealizable_estimate(&unparsed), None);

    let start_failed = bmc_nix::store::StorePathError::EstimateFailed(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "nix-store missing",
    ));
    assert_eq!(unrealizable_estimate(&start_failed), None);
}

fn test_nix_config(root: &Path, servers: &Path) -> NixUpgradeConfig {
    NixUpgradeConfig {
        servers_config_path: servers.to_path_buf(),
        gc_config_path: root.join("gc.json"),
        profile_dir: root.join("profile"),
        staging_dir: root.join("upload"),
        hooks_dir: "hooks".to_owned(),
        hooks_override_path: None,
    }
}

use std::collections::BTreeMap;

use bmc_nix::types::{InstalledBy, Manifest, ManifestPackage};

/// A servers.json holding only a disabled factory and no `servers`,
/// so the probe short-circuits on "no enabled servers".
const FACTORY_ONLY: &str = r#"{"factory":{"id":"forge","base_url":"http://x","known_public_key":"k","priority":0,"enabled":false},"servers":[]}"#;

/// Spawn a throwaway HTTP server that answers every request with the
/// given JSON body, and return its base URL.
async fn spawn_index_server(body: String) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("BUG: failed to bind mock index server");
    let addr = listener.local_addr().expect("BUG: no local addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

/// Bind then immediately drop a listener to obtain a port that refuses
/// connections — a deterministic "dead URL".
async fn dead_base_url() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("BUG: failed to bind for dead url");
    let addr = listener.local_addr().expect("BUG: no local addr");
    drop(listener);
    format!("http://{addr}")
}

/// Serialize a package index whose `packages` list holds the given
/// `(name, version, store_path)` entries.
fn index_json(entries: &[(&str, &str, &str)]) -> String {
    let packages: Vec<String> = entries
        .iter()
        .map(|(name, version, store_path)| {
            format!(r#"{{"name":"{name}","version":"{version}","store_path":"{store_path}"}}"#)
        })
        .collect();
    format!(
        r#"{{"version":{},"provenance":null,"indexes":[],"caches":[],"packages":[{}]}}"#,
        bmc_nix::index::PACKAGE_INDEX_VERSION,
        packages.join(",")
    )
}

/// Write a servers.json with one enabled required server pointed at
/// `base_url`.
fn write_enabled_server(path: &Path, base_url: &str) {
    let json = format!(
        r#"{{"factory":{{"id":"forge","base_url":"{base_url}","known_public_key":"k","priority":0,"enabled":false}},"servers":[{{"id":"srv","index_url":"{base_url}","known_public_key":"k","priority":10,"enabled":true,"required":true}}]}}"#
    );
    std::fs::write(path, json).expect("BUG: write servers.json");
}

/// Write a base manifest holding the given `InstalledBy::System`
/// packages into generation `1-link` under `profile_dir`.
fn write_base_manifest(profile_dir: &Path, packages: &[(&str, &str, &str)]) {
    let mut map = BTreeMap::new();
    for (name, version, store_path) in packages {
        map.insert(
            (*name).to_owned(),
            ManifestPackage {
                version: (*version).to_owned(),
                store_path: (*store_path).to_owned(),
                category: None,
                description: None,
                upgrade_strategy: None,
                install_strategy: None,
                installed_by: InstalledBy::System,
                installed_from: "srv".to_owned(),
                pinned: None,
            },
        );
    }
    let generation_dir = profile_dir.join("1-link");
    std::fs::create_dir_all(&generation_dir).expect("BUG: create generation dir");
    bmc_nix::manifest::write_manifest(&generation_dir, &Manifest { packages: map })
        .expect("BUG: write base manifest");
}

fn merged_with(entries: &[(&str, &str, Option<serde_json::Value>)]) -> bmc_nix::types::MergedIndex {
    let packages: Vec<String> = entries
        .iter()
        .map(|(name, category, metadata)| {
            let meta = metadata
                .clone()
                .map_or_else(|| "{}".to_owned(), |m| m.to_string());
            format!(
                r#"{{"name":"{name}","version":"1.0.0","store_path":"/nix/store/{name}","category":"{category}","metadata":{meta}}}"#
            )
        })
        .collect();
    let json = format!(
        r#"{{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[{}]}}"#,
        packages.join(",")
    );
    let raw: bmc_nix::types::PackageIndex = serde_json::from_str(&json).expect("BUG: parse index");
    bmc_nix::index::merge_indexes(vec![bmc_nix::types::FetchedIndex {
        server_id: "srv".to_owned(),
        server_priority: 10,
        index: raw,
    }])
}

#[test]
fn resolve_installs_maps_names_to_resolved_packages() {
    let merged = merged_with(&[("widget-weather", "widget", None)]);
    let resolved =
        resolve_installs(&merged, &["widget-weather".to_owned()]).expect("BUG: resolve failed");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].name, "widget-weather");
    assert_eq!(resolved[0].installed_by, bmc_nix::types::InstalledBy::User);
}

#[test]
fn installable_packages_preserves_arbitrary_metadata_and_non_widgets() {
    let metadata = serde_json::json!({"future": {"nested": [true, 7, null]}, "service": "metrics"});
    let merged = merged_with(&[
        ("metrics", "service", Some(metadata.clone())),
        ("installed", "widget", None),
    ]);
    let packages = installable_packages_from(
        &merged,
        &std::collections::BTreeSet::from(["installed".to_owned()]),
    );
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "metrics");
    assert_eq!(packages[0].category.as_deref(), Some("service"));
    assert_eq!(serde_json::json!(packages[0].metadata), metadata);
}

#[test]
fn resolve_installs_reports_unknown_target() {
    let merged = merged_with(&[("widget-weather", "widget", None)]);
    let err = resolve_installs(&merged, &["widget-nope".to_owned()])
        .expect_err("BUG: unknown install target must fail");
    assert!(
        matches!(err, PackageProbeError::InstallTargetUnavailable(_)),
        "got {err:?}"
    );
}

async fn target_feed_upgrader() -> (tempfile::TempDir, PackageUpgrader) {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    let current_index = spawn_index_server(index_json(&[])).await;
    let mut target: serde_json::Value = serde_json::from_str(&index_json(&[
        ("nix", "2.0.0", "/nix/store/target-nix"),
        ("widget-target", "1.0.0", "/nix/store/widget-target"),
    ]))
    .expect("BUG: valid index");
    target["packages"][1]["category"] = serde_json::json!("widget");
    target["packages"][1]["metadata"] = serde_json::json!({"widget": {"uid": "target-only"}});
    let target_index = spawn_index_server(target.to_string()).await;
    let feed = serde_json::json!({
        "version": bmc_nix::feed::PACKAGE_FEED_VERSION,
        "entries": [
            {"bos_version": "current", "download_url": "http://unused/current",
             "profile_path": "/nix/store/current", "index_url": current_index},
            {"bos_version": "target", "download_url": "http://unused/target",
             "profile_path": "/nix/store/target", "index_url": target_index}
        ]
    });
    let feed_url = spawn_index_server(feed.to_string()).await;
    let servers = serde_json::json!({
        "factory": {"id": "forge", "base_url": "http://unused", "known_public_key": "k", "priority": 0, "enabled": false},
        "servers": [{
        "id": "release", "feed_url": feed_url, "known_public_key": "k", "priority": 10,
        "enabled": true, "required": true
    }]});
    std::fs::write(&path, servers.to_string()).expect("BUG: write servers");
    write_base_manifest(
        &dir.path().join("profile"),
        &[("nix", "1.0.0", "/nix/store/old-nix")],
    );
    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    (dir, upgrader)
}

#[tokio::test]
async fn probe_selects_target_firmware_packages() {
    let (_dir, upgrader) = target_feed_upgrader().await;
    let PackageProbe::Available(offer) = upgrader
        .probe(
            Some("target"),
            EstimateMode::Skip,
            &["widget-target".to_owned()],
        )
        .await
    else {
        panic!("the target feed must offer the incoming packages");
    };
    let package = offer
        .index
        .packages
        .iter()
        .find(|package| package.name == "nix")
        .expect("BUG: the target must contain nix");
    assert_eq!(package.version.to_string(), "2.0.0");
    assert_eq!(package.store_path, "/nix/store/target-nix");
}

#[tokio::test]
async fn missing_target_feed_fails_the_probe() {
    let (_dir, upgrader) = target_feed_upgrader().await;
    let failure = upgrader
        .probe(Some("missing"), EstimateMode::Skip, &[])
        .await;
    assert!(
        matches!(failure, PackageProbe::Failed(PackageProbeError::IndexUnusable(ref message)) if message.contains("no package feed entry for BOS version 'missing'")),
        "a missing target must fail instead of selecting another firmware: {failure:?}"
    );
}

#[tokio::test]
async fn installable_packages_come_from_target_firmware() {
    let (_dir, upgrader) = target_feed_upgrader().await;
    let widgets = upgrader
        .list_installable_packages(Some("target"))
        .await
        .expect("BUG: target feed must resolve");
    assert_eq!(
        widgets
            .iter()
            .map(|widget| widget.name.as_str())
            .collect::<Vec<_>>(),
        vec!["widget-target"]
    );
    assert!(
        upgrader
            .list_installable_packages(Some("current"))
            .await
            .expect("BUG: current feed must resolve")
            .is_empty()
    );
}

#[tokio::test]
async fn missing_target_feed_fails_package_discovery() {
    let (_dir, upgrader) = target_feed_upgrader().await;
    assert!(
        matches!(upgrader.list_installable_packages(Some("missing")).await, Err(PackageProbeError::IndexUnusable(message)) if message.contains("no package feed entry for BOS version 'missing'"))
    );
}
#[tokio::test]
async fn probe_reports_no_enabled_servers() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    std::fs::write(&path, FACTORY_ONLY).expect("BUG: write servers.json");

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    assert!(matches!(
        upgrader.probe(None, EstimateMode::Skip, &[]).await,
        PackageProbe::Failed(PackageProbeError::NoEnabledServers)
    ));
}

#[tokio::test]
async fn probe_reports_config_unavailable_when_absent() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    let probe = upgrader.probe(None, EstimateMode::Skip, &[]).await;
    assert!(
        matches!(
            probe,
            PackageProbe::Failed(PackageProbeError::ServersConfigUnavailable(_))
        ),
        "got {probe:?}"
    );
    assert!(
        !dir.path().join("servers.json.bcp").exists(),
        "an absent config must not be backed up"
    );
}

#[tokio::test]
async fn probe_recovers_missing_config_from_default() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    std::fs::write(dir.path().join("servers.json.default"), FACTORY_ONLY)
        .expect("BUG: write default");

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    assert!(matches!(
        upgrader.probe(None, EstimateMode::Skip, &[]).await,
        PackageProbe::Failed(PackageProbeError::NoEnabledServers)
    ));
    assert!(
        !dir.path().join("servers.json.bcp").exists(),
        "recovering a missing (not corrupt) config must not back anything up"
    );
    assert!(
        !path.exists(),
        "the default is served in memory and never persisted"
    );
}

#[tokio::test]
async fn probe_recovers_corrupt_config_from_default() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    std::fs::write(&path, "{ not json").expect("BUG: write corrupt");
    std::fs::write(dir.path().join("servers.json.default"), FACTORY_ONLY)
        .expect("BUG: write default");

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    assert!(matches!(
        upgrader.probe(None, EstimateMode::Skip, &[]).await,
        PackageProbe::Failed(PackageProbeError::NoEnabledServers)
    ));
    assert!(
        dir.path().join("servers.json.bcp").exists(),
        "a corrupt config must be backed up to .bcp"
    );
}

#[tokio::test]
async fn probe_reports_config_unavailable_on_corrupt_without_default() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    std::fs::write(&path, "{ not json").expect("BUG: write corrupt");

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    let probe = upgrader.probe(None, EstimateMode::Skip, &[]).await;
    assert!(
        matches!(
            probe,
            PackageProbe::Failed(PackageProbeError::ServersConfigUnavailable(_))
        ),
        "got {probe:?}"
    );
    assert!(
        dir.path().join("servers.json.bcp").exists(),
        "a corrupt config must still be backed up to .bcp"
    );
}

#[tokio::test]
async fn probe_reports_missing_system_package() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    let base_url = spawn_index_server(index_json(&[])).await;
    write_enabled_server(&path, &base_url);
    write_base_manifest(
        &dir.path().join("profile"),
        &[("nix", "1.0.0", "/nix/store/nix")],
    );

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    let PackageProbe::Failed(PackageProbeError::PlanFailed(
        PackagePlanFailure::MissingSystemPackages { names },
    )) = upgrader.probe(None, EstimateMode::Skip, &[]).await
    else {
        panic!("expected a missing-system-package failure");
    };
    assert_eq!(names, vec!["nix".to_owned()]);
    assert_eq!(
        PackagePlanFailure::MissingSystemPackages { names }.to_string(),
        "package source is incomplete; required system package \"nix\" is missing"
    );
}

#[tokio::test]
async fn probe_reports_all_missing_system_packages() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    let base_url = spawn_index_server(index_json(&[])).await;
    write_enabled_server(&path, &base_url);
    write_base_manifest(
        &dir.path().join("profile"),
        &[
            ("nix", "1.0.0", "/nix/store/nix"),
            ("core", "1.0.0", "/nix/store/core"),
        ],
    );

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    let PackageProbe::Failed(PackageProbeError::PlanFailed(
        PackagePlanFailure::MissingSystemPackages { names },
    )) = upgrader.probe(None, EstimateMode::Skip, &[]).await
    else {
        panic!("expected a missing-system-package failure");
    };
    assert_eq!(names, vec!["core".to_owned(), "nix".to_owned()]);
    assert_eq!(
        PackagePlanFailure::MissingSystemPackages { names }.to_string(),
        "package source is incomplete; required system packages \"core\" and \"nix\" are missing"
    );
}

#[tokio::test]
async fn probe_reports_index_fetch_failed_on_dead_url() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    write_enabled_server(&path, &dead_base_url().await);
    write_base_manifest(
        &dir.path().join("profile"),
        &[("nix", "1.0.0", "/nix/store/nix")],
    );

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    let probe = upgrader.probe(None, EstimateMode::Skip, &[]).await;
    assert!(
        matches!(
            probe,
            PackageProbe::Failed(PackageProbeError::IndexFetchFailed(_))
        ),
        "got {probe:?}"
    );
}

#[tokio::test]
async fn probe_reports_up_to_date_when_index_matches() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    let base_url = spawn_index_server(index_json(&[("nix", "1.0.0", "/nix/store/nix")])).await;
    write_enabled_server(&path, &base_url);
    write_base_manifest(
        &dir.path().join("profile"),
        &[("nix", "1.0.0", "/nix/store/nix")],
    );

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    assert!(matches!(
        upgrader.probe(None, EstimateMode::Skip, &[]).await,
        PackageProbe::UpToDate
    ));
}

/// Which typed estimate outcome the stubbed store returns, so a probe test
/// can drive each routing branch without nix or its `internal-json` format.
#[derive(Debug)]
enum StubEstimate {
    Downloads(u64),
    Unrealizable(Vec<String>),
    Transient,
}

/// A [`StoreOperations`](bmc_nix::store::StoreOperations) that returns a
/// fixed dry-run estimate outcome.
#[derive(Debug)]
struct StubStore(StubEstimate);

impl bmc_nix::store::StoreOperations for StubStore {
    fn estimate_realization(
        &self,
        _packages: &[bmc_nix::types::ResolvedPackage],
    ) -> impl std::future::Future<
        Output = Result<bmc_nix::store::RealizeEstimate, bmc_nix::store::StorePathError>,
    > + Send {
        use std::os::unix::process::ExitStatusExt as _;
        let result = match &self.0 {
            StubEstimate::Downloads(bytes) => Ok(bmc_nix::store::RealizeEstimate {
                fetch_paths: 1,
                download_bytes: *bytes,
                unpacked_bytes: *bytes,
            }),
            StubEstimate::Unrealizable(paths) => {
                Err(bmc_nix::store::StorePathError::UnsubstitutablePaths {
                    paths: paths.clone(),
                })
            }
            // A non-zero dry-run exit — dead substituter, inconclusive not
            // doomed — is the transient class that keeps offering the upgrade.
            StubEstimate::Transient => Err(bmc_nix::store::StorePathError::EstimateExited {
                status: std::process::ExitStatus::from_raw(1 << 8),
                messages: vec!["error: cannot reach substituter".to_owned()],
            }),
        };
        async move { result }
    }

    fn store_free_bytes(&self, _profile_dir: &std::path::Path) -> std::io::Result<u64> {
        unreachable!("BUG: StubStore serves probe estimates only")
    }

    async fn restore_nar(
        &self,
        _nar: bmc_nix::store::InputStream,
        _target: &Path,
    ) -> Result<(), bmc_nix::store::StoreCommandError> {
        unreachable!("BUG: StubStore serves probe estimates only")
    }

    async fn delete_store_path(
        &self,
        _store_path: &str,
    ) -> Result<(), bmc_nix::store::StoreCommandError> {
        unreachable!("BUG: StubStore serves probe estimates only")
    }

    async fn realize_store_paths(
        &self,
        _packages: &[bmc_nix::types::ResolvedPackage],
        _progress: Option<&dyn bmc_nix::store::RealizeProgress>,
    ) -> Result<(), bmc_nix::store::StorePathError> {
        unreachable!("BUG: StubStore serves probe estimates only")
    }

    async fn verify_store_paths(
        &self,
        _packages: &[bmc_nix::types::ResolvedPackage],
    ) -> Result<(), bmc_nix::store::StorePathError> {
        unreachable!("BUG: StubStore serves probe estimates only")
    }

    async fn collect_garbage(
        &self,
        _progress: Option<&dyn bmc_nix::gc::CollectGarbageProgress>,
    ) -> Result<(), bmc_nix::gc::CollectGarbageError> {
        unreachable!("BUG: StubStore serves probe estimates only")
    }

    async fn add_to_store(&self, _path: &Path) -> Result<String, bmc_nix::store::AddToStoreError> {
        unreachable!("BUG: StubStore serves probe estimates only")
    }
}

#[derive(Debug)]
struct ApplyStore {
    outcome: ApplyStoreOutcome,
}

#[derive(Debug)]
enum ApplyStoreOutcome {
    SpaceRefusal,
    RealizationFailure,
}

impl bmc_nix::store::StoreOperations for ApplyStore {
    async fn estimate_realization(
        &self,
        _packages: &[bmc_nix::types::ResolvedPackage],
    ) -> Result<bmc_nix::store::RealizeEstimate, bmc_nix::store::StorePathError> {
        Ok(bmc_nix::store::RealizeEstimate {
            fetch_paths: 1,
            download_bytes: 13,
            unpacked_bytes: 17,
        })
    }

    fn store_free_bytes(&self, _profile_dir: &Path) -> std::io::Result<u64> {
        Ok(match self.outcome {
            ApplyStoreOutcome::SpaceRefusal => 11,
            ApplyStoreOutcome::RealizationFailure => u64::MAX,
        })
    }

    async fn realize_store_paths(
        &self,
        _packages: &[bmc_nix::types::ResolvedPackage],
        _progress: Option<&dyn bmc_nix::store::RealizeProgress>,
    ) -> Result<(), bmc_nix::store::StorePathError> {
        match self.outcome {
            ApplyStoreOutcome::RealizationFailure => {
                Err(bmc_nix::store::StorePathError::RealiseFailed(
                    std::io::Error::other("scripted realization failure"),
                ))
            }
            ApplyStoreOutcome::SpaceRefusal => {
                unreachable!("BUG: a space refusal must precede realization")
            }
        }
    }

    async fn verify_store_paths(
        &self,
        _packages: &[bmc_nix::types::ResolvedPackage],
    ) -> Result<(), bmc_nix::store::StorePathError> {
        unreachable!("BUG: apply failure must precede verification")
    }

    async fn collect_garbage(
        &self,
        _progress: Option<&dyn bmc_nix::gc::CollectGarbageProgress>,
    ) -> Result<(), bmc_nix::gc::CollectGarbageError> {
        unreachable!("BUG: package apply disables garbage collection")
    }

    async fn add_to_store(&self, _path: &Path) -> Result<String, bmc_nix::store::AddToStoreError> {
        unreachable!("BUG: ApplyStore serves indexed installs only")
    }

    async fn restore_nar(
        &self,
        _nar: bmc_nix::store::InputStream,
        _target: &Path,
    ) -> Result<(), bmc_nix::store::StoreCommandError> {
        unreachable!("BUG: ApplyStore serves indexed installs only")
    }

    async fn delete_store_path(
        &self,
        _store_path: &str,
    ) -> Result<(), bmc_nix::store::StoreCommandError> {
        unreachable!("BUG: ApplyStore serves indexed installs only")
    }
}

#[derive(Debug)]
struct SilentProgress;

impl bmc_nix::upgrade::UpgradeProgress for SilentProgress {
    fn on_phase(&self, _phase: bmc_nix::upgrade::UpgradePhase) {}
    fn on_realization_started(&self, _total_paths: usize) {}
    fn on_realization_finished(&self) {}
    fn on_download_status(&self, _snapshot: &bmc_nix::store::progress::DownloadSnapshot) {}
    fn on_gc_deleted(&self, _deleted_paths: usize) {}
    fn on_gc_finished(&self, _deleted_paths: usize, _freed_bytes: Option<u64>) {}
}

async fn apply_one_widget(store: ApplyStore) -> ApplyError {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    write_base_manifest(&dir.path().join("profile"), &[]);
    let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &servers), store);

    upgrader
        .apply(
            merged_with(&[("widget-weather", "widget", None)]),
            vec!["widget-weather".to_owned()],
            Arc::new(SilentProgress),
        )
        .await
        .expect_err("scripted store must fail package apply")
}

#[tokio::test]
async fn apply_preserves_not_enough_space_fields_and_source() {
    let error = apply_one_widget(ApplyStore {
        outcome: ApplyStoreOutcome::SpaceRefusal,
    })
    .await;

    let ApplyError::Install(
        source @ bmc_nix::upgrade::InstallError::NotEnoughSpace {
            free_bytes,
            required_bytes,
            unpacked_bytes,
        },
    ) = &error
    else {
        panic!("expected a typed not-enough-space install failure, got {error:?}");
    };
    assert_eq!(*free_bytes, 11);
    assert_eq!(*required_bytes, bmc_nix::store::required_with_headroom(17));
    assert_eq!(*unpacked_bytes, 17);
    let preserved = error
        .source()
        .and_then(|source| source.downcast_ref::<bmc_nix::upgrade::InstallError>())
        .expect("BUG: the apply error must expose the original install error as its source");
    assert!(std::ptr::eq(preserved, source));
}

#[tokio::test]
async fn apply_preserves_non_space_install_source() {
    let error = apply_one_widget(ApplyStore {
        outcome: ApplyStoreOutcome::RealizationFailure,
    })
    .await;

    let ApplyError::Install(source @ bmc_nix::upgrade::InstallError::StorePaths(_)) = &error else {
        panic!("expected a typed store-path install failure, got {error:?}");
    };
    let preserved = error
        .source()
        .and_then(|source| source.downcast_ref::<bmc_nix::upgrade::InstallError>())
        .expect("BUG: the apply error must expose the original install error as its source");
    assert!(std::ptr::eq(preserved, source));
}

#[tokio::test]
async fn apply_preserves_resolve_failure_source() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let servers = dir.path().join("servers.json");
    let upgrader = PackageUpgrader::with_store(
        test_nix_config(dir.path(), &servers),
        ApplyStore {
            outcome: ApplyStoreOutcome::SpaceRefusal,
        },
    );

    let error = upgrader
        .apply(
            merged_with(&[]),
            vec!["widget-nope".to_owned()],
            Arc::new(SilentProgress),
        )
        .await
        .expect_err("an unavailable install target must fail resolution");
    let ApplyError::Resolve(source) = &error else {
        panic!("expected a typed resolve failure, got {error:?}");
    };
    assert!(matches!(
        source,
        PackageProbeError::InstallTargetUnavailable(_)
    ));
    let preserved = error
        .source()
        .and_then(|source| source.downcast_ref::<PackageProbeError>())
        .expect("BUG: the apply error must expose the original probe error as its source");
    assert!(std::ptr::eq(preserved, source));
}

/// Base `nix@1.0.0` with an index offering `nix@1.1.0` yields a non-empty
/// changed plan, so `EstimateMode::Estimate` actually runs the estimate.
async fn write_pending_upgrade(dir: &Path, path: &Path) {
    write_base_manifest(&dir.join("profile"), &[("nix", "1.0.0", "/nix/store/nix")]);
    let base_url = spawn_index_server(index_json(&[("nix", "1.1.0", "/nix/store/nix-new")])).await;
    write_enabled_server(path, &base_url);
}

#[tokio::test]
async fn probe_fails_unrealizable_when_estimate_needs_unsubstitutable_paths() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    write_pending_upgrade(dir.path(), &path).await;

    let store = StubStore(StubEstimate::Unrealizable(vec![
        "/nix/store/nix-new".to_owned(),
    ]));
    let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &path), store);

    let probe = upgrader.probe(None, EstimateMode::Estimate, &[]).await;
    let PackageProbe::Failed(PackageProbeError::Unrealizable(paths)) = probe else {
        panic!("an unsubstitutable estimate must fail the probe, got {probe:?}");
    };
    assert_eq!(paths, "/nix/store/nix-new");
}

#[tokio::test]
async fn probe_offers_upgrade_without_size_on_transient_estimate_error() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    write_pending_upgrade(dir.path(), &path).await;

    let store = StubStore(StubEstimate::Transient);
    let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &path), store);

    let probe = upgrader.probe(None, EstimateMode::Estimate, &[]).await;
    let PackageProbe::Available(offer) = probe else {
        panic!("a transient estimate error must still offer the upgrade, got {probe:?}");
    };
    assert!(
        offer.preview.download_size_bytes.is_none(),
        "a transient estimate error must omit the download size"
    );
    assert!(
        !offer.preview.changes.is_empty(),
        "the offered upgrade must still carry the changed package"
    );
}

#[tokio::test]
async fn probe_reports_download_size_from_a_successful_estimate() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    write_pending_upgrade(dir.path(), &path).await;

    let store = StubStore(StubEstimate::Downloads(4096));
    let upgrader = PackageUpgrader::with_store(test_nix_config(dir.path(), &path), store);

    let probe = upgrader.probe(None, EstimateMode::Estimate, &[]).await;
    let PackageProbe::Available(offer) = probe else {
        panic!("a successful estimate must offer the upgrade, got {probe:?}");
    };
    assert_eq!(offer.preview.download_size_bytes, Some(4096));
}
#[tokio::test]
async fn probe_reports_install_target_unavailable() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    let base_url = spawn_index_server(index_json(&[("nix", "1.0.0", "/nix/store/nix")])).await;
    write_enabled_server(&path, &base_url);
    write_base_manifest(
        &dir.path().join("profile"),
        &[("nix", "1.0.0", "/nix/store/nix")],
    );

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    // The index carries only "nix"; a requested install the index does not
    // list fails the whole probe at the resolve stage.
    let probe = upgrader
        .probe(None, EstimateMode::Skip, &["widget-nope".to_owned()])
        .await;
    assert!(
        matches!(
            probe,
            PackageProbe::Failed(PackageProbeError::InstallTargetUnavailable(_))
        ),
        "got {probe:?}"
    );
}

#[tokio::test]
async fn probe_reports_index_unusable_on_garbage_json() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    // The fetch succeeds (HTTP 200) but the body is not a valid index: a
    // parse failure is an unusable index, not a transient fetch failure.
    let base_url = spawn_index_server("{ not an index".to_owned()).await;
    write_enabled_server(&path, &base_url);
    write_base_manifest(
        &dir.path().join("profile"),
        &[("nix", "1.0.0", "/nix/store/nix")],
    );

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    let probe = upgrader.probe(None, EstimateMode::Skip, &[]).await;
    assert!(
        matches!(
            probe,
            PackageProbe::Failed(PackageProbeError::IndexUnusable(_))
        ),
        "got {probe:?}"
    );
}

#[tokio::test]
async fn probe_reports_manifest_read_failed_when_manifest_malformed() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    let base_url = spawn_index_server(index_json(&[("nix", "1.0.0", "/nix/store/nix")])).await;
    write_enabled_server(&path, &base_url);
    // A present but unparseable current manifest is a read failure. An
    // absent profile is not: read_latest_manifest falls back to an empty
    // manifest, so the probe would report UpToDate instead.
    let current = dir.path().join("profile/current");
    std::fs::create_dir_all(&current).expect("BUG: create current dir");
    std::fs::write(current.join("manifest"), "{ not a manifest").expect("BUG: write manifest");

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    let probe = upgrader.probe(None, EstimateMode::Skip, &[]).await;
    assert!(
        matches!(
            probe,
            PackageProbe::Failed(PackageProbeError::ManifestReadFailed(_))
        ),
        "got {probe:?}"
    );
}

#[tokio::test]
async fn list_installable_packages_reports_no_enabled_servers() {
    let dir = tempfile::tempdir().expect("BUG: tempdir");
    let path = dir.path().join("servers.json");
    std::fs::write(&path, FACTORY_ONLY).expect("BUG: write servers.json");

    let upgrader = PackageUpgrader::new(test_nix_config(dir.path(), &path));

    assert!(matches!(
        upgrader.list_installable_packages(None).await,
        Err(PackageProbeError::NoEnabledServers)
    ));
}
