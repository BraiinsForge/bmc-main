// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::path::Path;

use crate::index::{self, ResolvePackageError};
use crate::types::{
    Manifest, ManifestPackage, MergedIndex, PackageChange, PackageVersion, ResolvedPackage,
    UpgradePlan,
};

/// Error type for manifest write operations.
#[derive(Debug, thiserror::Error)]
pub enum WriteManifestError {
    #[error("failed to serialize manifest: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write manifest to {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Error type for manifest read operations.
#[derive(Debug, thiserror::Error)]
pub enum ReadManifestError {
    #[error("manifest read failed: {0}")]
    Read(#[source] std::io::Error),
    #[error("manifest parse failed: {0}")]
    Parse(#[source] serde_json::Error),
}

/// Error type for upgrade plan computation.
#[derive(Debug, thiserror::Error)]
pub enum ComputeUpgradePlanError {
    #[error("failed to resolve package '{name}': {source}")]
    ResolveError {
        name: String,
        #[source]
        source: ResolvePackageError,
    },
}

/// Build a [`Manifest`] from a slice of resolved packages.
///
/// Each [`ResolvedPackage`] is converted into a [`ManifestPackage`] and keyed
/// by its name. The `cache` field is taken from `ResolvedPackage::cache_name`.
#[must_use]
pub fn build_manifest(packages: &[ResolvedPackage]) -> Manifest {
    let packages = packages
        .iter()
        .map(|pkg| {
            let entry = ManifestPackage {
                version: pkg.version.clone(),
                cache: pkg.cache_name.clone(),
                store_path: pkg.store_path.clone(),
                category: pkg.category.clone(),
                description: pkg.description.clone(),
                upgrade_strategy: pkg.upgrade_strategy.clone(),
                install_strategy: pkg.install_strategy.clone(),
                installed_by: pkg.installed_by.clone(),
                installed_from: pkg.installed_from.clone(),
                pinned: pkg.pinned.clone(),
            };
            (pkg.name.clone(), entry)
        })
        .collect::<BTreeMap<_, _>>();

    Manifest { packages }
}

/// Read a manifest from a profile generation directory.
///
/// Reads `<profile_path>/manifest` and deserializes it.
pub fn read_manifest(profile_path: &Path) -> Result<Manifest, ReadManifestError> {
    let manifest_path = profile_path.join("manifest");
    let contents = std::fs::read_to_string(&manifest_path).map_err(ReadManifestError::Read)?;
    serde_json::from_str(&contents).map_err(ReadManifestError::Parse)
}

/// Convert a manifest package back to a resolved package.
///
/// This is lossy — `ManifestPackage` does not store `cache_url`, only
/// `cache` (name). The `cache_url` is set to empty string. Only use
/// the result for profile building (which needs store paths), NOT for
/// `copy_store_paths` (which needs cache URLs).
fn manifest_package_to_resolved(name: &str, mp: &ManifestPackage) -> ResolvedPackage {
    ResolvedPackage {
        name: name.to_owned(),
        version: mp.version.clone(),
        store_path: mp.store_path.clone(),
        cache_url: String::new(),
        cache_name: mp.cache.clone(),
        category: mp.category.clone(),
        description: mp.description.clone(),
        upgrade_strategy: mp.upgrade_strategy.clone(),
        install_strategy: mp.install_strategy.clone(),
        installed_by: mp.installed_by.clone(),
        installed_from: mp.installed_from.clone(),
        pinned: mp.pinned.clone(),
    }
}

/// Compute an upgrade plan by diffing the current manifest against a
/// merged index.
///
/// - Packages in the manifest missing from the index are kept at current
///   version and reported as stale.
/// - `add_packages` are new packages to install.
/// - `remove_packages` are package names to remove from the profile.
pub fn compute_upgrade_plan(
    current: &Manifest,
    merged: Option<&MergedIndex>,
    add_packages: &[ResolvedPackage],
    remove_packages: &[String],
) -> Result<UpgradePlan, ComputeUpgradePlanError> {
    let mut packages = Vec::new();
    let mut stale = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();

    // Track which names are being added, so we can detect add-replaces-existing
    let add_by_name: BTreeMap<&str, &ResolvedPackage> =
        add_packages.iter().map(|p| (p.name.as_str(), p)).collect();

    for (name, pkg) in &current.packages {
        // Check if being removed
        if remove_packages.iter().any(|r| r == name) {
            removed.push(PackageVersion {
                name: name.clone(),
                version: pkg.version.clone(),
            });
            continue;
        }

        // Check if being replaced by an explicit add
        if let Some(&new_pkg) = add_by_name.get(name.as_str()) {
            if new_pkg.version != pkg.version {
                changed.push(PackageChange {
                    name: name.clone(),
                    from_version: pkg.version.clone(),
                    to_version: new_pkg.version.clone(),
                });
            }
            packages.push(new_pkg.clone());
            continue;
        }

        // Try to resolve from merged index
        if let Some(merged) = merged {
            match index::resolve_installed_package(merged, name, pkg) {
                Ok(resolved) => {
                    if resolved.version != pkg.version {
                        changed.push(PackageChange {
                            name: name.clone(),
                            from_version: pkg.version.clone(),
                            to_version: resolved.version.clone(),
                        });
                    }
                    packages.push(resolved);
                }
                Err(
                    ResolvePackageError::PackageNotFound(_)
                    | ResolvePackageError::VersionNotFound { .. },
                ) => {
                    // Package not in index or no matching version — stale
                    stale.push(PackageVersion {
                        name: name.clone(),
                        version: pkg.version.clone(),
                    });
                    packages.push(manifest_package_to_resolved(name, pkg));
                }
                Err(e) => {
                    return Err(ComputeUpgradePlanError::ResolveError {
                        name: name.clone(),
                        source: e,
                    });
                }
            }
        } else {
            // Offline mode — keep current version
            packages.push(manifest_package_to_resolved(name, pkg));
        }
    }

    // Add new packages that aren't replacing existing ones
    for pkg in add_packages {
        if !current.packages.contains_key(&pkg.name) {
            added.push(PackageVersion {
                name: pkg.name.clone(),
                version: pkg.version.clone(),
            });
            packages.push(pkg.clone());
        }
    }

    Ok(UpgradePlan {
        packages,
        stale,
        added,
        removed,
        changed,
    })
}

/// Write a manifest as pretty-printed JSON to `<profile_path>/manifest`.
///
/// Note: `fs::write` itself is not atomic (it truncates and writes in place).
/// Atomicity is provided by the caller (`build_profile`) which writes into a
/// temporary directory and renames it to the final generation path.
pub fn write_manifest(profile_path: &Path, manifest: &Manifest) -> Result<(), WriteManifestError> {
    let json = serde_json::to_string_pretty(manifest).map_err(WriteManifestError::Serialize)?;
    let manifest_path = profile_path.join("manifest");
    std::fs::write(&manifest_path, json).map_err(|source| WriteManifestError::Write {
        path: manifest_path.display().to_string(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::index::merge_indexes;
    use crate::types::{
        CacheEntry, InstalledBy, ManifestPackage, MergedIndex, PackageEntry, PackageIndex,
        PinStrategy,
    };

    fn test_package(name: &str, store_path: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            store_path: store_path.into(),
            cache_url: "https://cache.example.com".into(),
            cache_name: "local".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        }
    }

    fn test_resolved_package(name: &str, store_path: &str) -> ResolvedPackage {
        test_package(name, store_path)
    }

    fn test_manifest_package(
        version: &str,
        installed_from: &str,
        pinned: PinStrategy,
    ) -> ManifestPackage {
        ManifestPackage {
            version: version.into(),
            cache: "default".into(),
            store_path: format!("/nix/store/hash-pkg-{version}"),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: installed_from.into(),
            pinned,
        }
    }

    fn build_test_merged_index(entries: &[(&str, &str, &str)]) -> MergedIndex {
        let mut by_server: BTreeMap<String, Vec<(&str, &str)>> = BTreeMap::new();
        for &(name, version, server_id) in entries {
            by_server
                .entry(server_id.to_owned())
                .or_default()
                .push((name, version));
        }

        let mut all_fetched = Vec::new();
        let mut priorities = BTreeMap::new();
        for (server_id, pkgs) in &by_server {
            priorities.insert(server_id.clone(), 1_u32);
            let cache_name = format!("cache-{server_id}");
            let packages: Vec<PackageEntry> = pkgs
                .iter()
                .map(|(name, version)| PackageEntry {
                    name: (*name).into(),
                    version: (*version).into(),
                    cache: Some(cache_name.clone()),
                    store_path: format!("/nix/store/hash-{name}-{version}"),
                    category: None,
                    description: None,
                    upgrade_strategy: None,
                    install_strategy: None,
                    server_id: String::new(),
                })
                .collect();
            let index = PackageIndex {
                version: 1,
                provenance: None,
                indexes: vec![],
                caches: vec![CacheEntry {
                    name: cache_name.clone(),
                    cache_url: format!("https://{cache_name}.example.com"),
                    cache_key: format!("{cache_name}:KEY"),
                }],
                packages,
            };
            all_fetched.push((server_id.clone(), index));
        }

        merge_indexes(all_fetched, &priorities)
    }

    // ---- build_manifest tests ----

    #[test]
    fn build_manifest_round_trips() {
        let packages = vec![
            test_package("pkg-a", "/nix/store/aaa-pkg-a-1.0.0"),
            test_package("pkg-b", "/nix/store/bbb-pkg-b-1.0.0"),
        ];

        let manifest = build_manifest(&packages);
        let json =
            serde_json::to_string_pretty(&manifest).expect("BUG: serialization should succeed");
        let parsed: Manifest =
            serde_json::from_str(&json).expect("BUG: deserialization should succeed");

        assert_eq!(parsed.packages.len(), 2);
        assert!(parsed.packages.contains_key("pkg-a"));
        assert!(parsed.packages.contains_key("pkg-b"));
        assert_eq!(
            parsed
                .packages
                .get("pkg-a")
                .expect("BUG: pkg-a should exist")
                .store_path,
            "/nix/store/aaa-pkg-a-1.0.0"
        );
    }

    #[test]
    fn build_manifest_uses_cache_name_from_resolved() {
        let mut pkg = test_package("some-pkg", "/nix/store/xyz-some-pkg-1.0.0");
        pkg.cache_name = "my-cache".into();

        let manifest = build_manifest(&[pkg]);
        let entry = manifest
            .packages
            .get("some-pkg")
            .expect("BUG: some-pkg should be present");

        assert_eq!(entry.cache, "my-cache");
    }

    #[test]
    fn build_manifest_defaults_to_local_cache() {
        let packages = vec![test_package("some-pkg", "/nix/store/xyz-some-pkg-1.0.0")];
        let manifest = build_manifest(&packages);
        let pkg = manifest
            .packages
            .get("some-pkg")
            .expect("BUG: some-pkg should be present in manifest");
        assert_eq!(pkg.cache, "local");
    }

    #[test]
    fn write_manifest_creates_file() {
        let dir = tempfile::tempdir().expect("BUG: should create temp dir");
        let packages = vec![test_package("test-pkg", "/nix/store/abc-test-pkg-1.0.0")];
        let manifest = build_manifest(&packages);

        write_manifest(dir.path(), &manifest).expect("BUG: write_manifest should succeed");

        let manifest_path = dir.path().join("manifest");
        let contents =
            std::fs::read_to_string(&manifest_path).expect("BUG: should read manifest file");
        let parsed: Manifest =
            serde_json::from_str(&contents).expect("BUG: manifest file should contain valid JSON");

        assert_eq!(parsed.packages.len(), 1);
        assert!(parsed.packages.contains_key("test-pkg"));
    }

    // ---- read_manifest tests ----

    #[test]
    fn read_manifest_round_trips() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");

        let packages = vec![test_package("pkg-a", "/nix/store/aaa")];
        let manifest = build_manifest(&packages);
        write_manifest(dir.path(), &manifest).expect("BUG: write failed");

        let read_back = read_manifest(dir.path()).expect("BUG: read failed");
        assert_eq!(read_back.packages.len(), 1);
        assert!(read_back.packages.contains_key("pkg-a"));
    }

    #[test]
    fn read_manifest_missing_file_returns_error() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let result = read_manifest(dir.path());
        assert!(result.is_err());
    }

    // ---- compute_upgrade_plan tests ----

    #[test]
    fn compute_upgrade_plan_detects_version_change() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", PinStrategy::None),
            )]),
        };
        let merged = build_test_merged_index(&[("widget", "2.0.0", "server_a")]);
        let plan =
            compute_upgrade_plan(&current, Some(&merged), &[], &[]).expect("BUG: plan failed");
        assert_eq!(plan.changed.len(), 1);
        assert_eq!(plan.changed[0].from_version, "1.0.0");
        assert_eq!(plan.changed[0].to_version, "2.0.0");
    }

    #[test]
    fn compute_upgrade_plan_detects_new_package() {
        let current = Manifest {
            packages: BTreeMap::new(),
        };
        let new_pkg = test_resolved_package("new-widget", "/nix/store/new");
        let plan = compute_upgrade_plan(&current, None, &[new_pkg], &[]).expect("BUG: plan failed");
        assert_eq!(plan.added.len(), 1);
        assert_eq!(plan.added[0].name, "new-widget");
    }

    #[test]
    fn compute_upgrade_plan_reports_stale_packages() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "old-widget".into(),
                test_manifest_package("1.0.0", "server_a", PinStrategy::None),
            )]),
        };
        let merged = MergedIndex {
            caches: vec![],
            packages: vec![],
            by_name: BTreeMap::new(),
        };
        let plan =
            compute_upgrade_plan(&current, Some(&merged), &[], &[]).expect("BUG: plan failed");
        assert_eq!(plan.stale.len(), 1);
    }

    #[test]
    fn upgrade_plan_respects_pin_major() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", PinStrategy::Major),
            )]),
        };
        let merged = build_test_merged_index(&[
            ("widget", "1.1.0", "server_a"),
            ("widget", "2.0.0", "server_a"),
        ]);
        let plan =
            compute_upgrade_plan(&current, Some(&merged), &[], &[]).expect("BUG: plan failed");
        assert_eq!(plan.changed.len(), 1);
        assert_eq!(plan.changed[0].to_version, "1.1.0");
    }

    #[test]
    fn upgrade_plan_with_add_and_remove() {
        let current = Manifest {
            packages: BTreeMap::from([
                (
                    "keep-pkg".into(),
                    test_manifest_package("1.0.0", "server_a", PinStrategy::None),
                ),
                (
                    "remove-pkg".into(),
                    test_manifest_package("1.0.0", "server_a", PinStrategy::None),
                ),
            ]),
        };
        let new_pkg = test_resolved_package("add-pkg", "/nix/store/add");
        let plan = compute_upgrade_plan(&current, None, &[new_pkg], &["remove-pkg".into()])
            .expect("BUG: plan failed");
        assert_eq!(plan.added.len(), 1);
        assert_eq!(plan.removed.len(), 1);
        // keep-pkg + add-pkg
        assert_eq!(plan.packages.len(), 2);
    }

    #[test]
    fn upgrade_plan_no_changes() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", PinStrategy::None),
            )]),
        };
        let merged = build_test_merged_index(&[("widget", "1.0.0", "server_a")]);
        let plan =
            compute_upgrade_plan(&current, Some(&merged), &[], &[]).expect("BUG: plan failed");
        assert!(plan.changed.is_empty());
        assert!(plan.added.is_empty());
        assert!(plan.removed.is_empty());
        assert!(plan.stale.is_empty());
        assert_eq!(plan.packages.len(), 1);
    }

    #[test]
    fn upgrade_plan_add_replaces_existing() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", PinStrategy::None),
            )]),
        };
        let mut new_pkg = test_resolved_package("widget", "/nix/store/widget-2");
        new_pkg.version = "2.0.0".into();
        let plan = compute_upgrade_plan(&current, None, &[new_pkg], &[]).expect("BUG: plan failed");
        // Should count as a change, not an add
        assert_eq!(plan.changed.len(), 1);
        assert!(plan.added.is_empty());
        assert_eq!(plan.packages.len(), 1);
    }

    #[test]
    fn upgrade_plan_offline_mode_keeps_all_current() {
        let current = Manifest {
            packages: BTreeMap::from([
                (
                    "widget".into(),
                    test_manifest_package("1.0.0", "server_a", PinStrategy::None),
                ),
                (
                    "gadget".into(),
                    test_manifest_package("3.0.0", "server_a", PinStrategy::None),
                ),
            ]),
        };
        let plan = compute_upgrade_plan(&current, None, &[], &[]).expect("BUG: plan failed");
        assert!(plan.changed.is_empty());
        assert!(plan.stale.is_empty());
        assert!(plan.added.is_empty());
        assert!(plan.removed.is_empty());
        assert_eq!(plan.packages.len(), 2);
    }

    #[test]
    fn upgrade_plan_mixed_operations() {
        let current = Manifest {
            packages: BTreeMap::from([
                (
                    "widget".into(),
                    test_manifest_package("1.0.0", "server_a", PinStrategy::None),
                ),
                (
                    "old-thing".into(),
                    test_manifest_package("1.0.0", "server_a", PinStrategy::None),
                ),
                (
                    "to-remove".into(),
                    test_manifest_package("2.0.0", "server_a", PinStrategy::None),
                ),
            ]),
        };
        let merged = build_test_merged_index(&[("widget", "2.0.0", "server_a")]);
        let new_pkg = test_resolved_package("fresh-app", "/nix/store/fresh");
        let plan = compute_upgrade_plan(&current, Some(&merged), &[new_pkg], &["to-remove".into()])
            .expect("BUG: plan failed");
        assert_eq!(plan.changed.len(), 1, "widget upgraded");
        assert_eq!(plan.stale.len(), 1, "old-thing is stale");
        assert_eq!(plan.added.len(), 1, "fresh-app added");
        assert_eq!(plan.removed.len(), 1, "to-remove removed");
        assert_eq!(plan.packages.len(), 3);
    }

    #[test]
    fn upgrade_plan_empty_manifest_with_adds() {
        let current = Manifest {
            packages: BTreeMap::new(),
        };
        let pkgs = vec![
            test_resolved_package("app-a", "/nix/store/a"),
            test_resolved_package("app-b", "/nix/store/b"),
        ];
        let plan = compute_upgrade_plan(&current, None, &pkgs, &[]).expect("BUG: plan failed");
        assert_eq!(plan.added.len(), 2);
        assert_eq!(plan.packages.len(), 2);
        assert!(plan.changed.is_empty());
        assert!(plan.removed.is_empty());
    }

    #[test]
    fn upgrade_plan_remove_nonexistent_is_noop() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", PinStrategy::None),
            )]),
        };
        let plan = compute_upgrade_plan(&current, None, &[], &["nonexistent".into()])
            .expect("BUG: plan failed");
        assert!(plan.removed.is_empty());
        assert_eq!(plan.packages.len(), 1);
    }
}
