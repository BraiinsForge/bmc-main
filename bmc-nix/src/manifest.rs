// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::types::{
    Manifest, ManifestPackage, PackageChange, PackageVersion, ResolvedPackage, UpgradePlan,
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

/// Error returned when `compute_upgrade_plan` is given inputs that conflict
/// or are meaningless relative to the current manifest.
#[derive(Debug, thiserror::Error)]
pub enum PlanConflict {
    #[error("package `{0}` appears in both add and remove lists")]
    AddAndRemove(String),

    #[error(
        "package `{name}` added more than once with mismatched specs \
         (first: version {first_version} store-path {first_store_path}; \
         second: version {second_version} store-path {second_store_path})"
    )]
    DuplicateAddMismatch {
        name: String,
        first_version: String,
        first_store_path: String,
        second_version: String,
        second_store_path: String,
    },

    #[error("package `{0}` listed for removal more than once")]
    DuplicateRemove(String),

    #[error("package `{0}` requested for removal but not present in the current profile")]
    RemoveNotInstalled(String),
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

/// Read a manifest from a profile generation directory.
///
/// Reads `<profile_path>/manifest` and deserializes it.
pub fn read_manifest(profile_path: &Path) -> Result<Manifest, ReadManifestError> {
    let manifest_path = profile_path.join("manifest");
    let contents = std::fs::read_to_string(&manifest_path).map_err(ReadManifestError::Read)?;
    serde_json::from_str(&contents).map_err(ReadManifestError::Parse)
}

/// Read the manifest from the `current` symlink in `profile_dir`, or return
/// an empty manifest when no profile exists yet.
///
/// Returns an error if the manifest file exists but cannot be parsed.
pub fn read_current_manifest(profile_dir: &Path) -> Result<Manifest, ReadManifestError> {
    let current_link = profile_dir.join("current");
    if current_link.exists() {
        read_manifest(&current_link)
    } else {
        Ok(Manifest::default())
    }
}

/// Convert a manifest package back to a resolved package.
///
/// This is lossy — `ManifestPackage` does not store `cache_url`, only
/// `cache` (name). The `cache_url` is set to `None`. The store path
/// is expected to already be present locally.
#[must_use]
pub fn manifest_package_to_resolved(name: &str, mp: &ManifestPackage) -> ResolvedPackage {
    ResolvedPackage {
        name: name.to_owned(),
        version: mp.version.clone(),
        store_path: mp.store_path.clone(),
        cache_url: None,
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

/// Compute an upgrade plan by diffing the current manifest against
/// add/remove requests.
///
/// - Existing packages not being removed or replaced are kept at their
///   current version.
/// - `add_packages` are new packages to install (or replace existing ones).
/// - `remove_packages` are package names to remove from the profile.
///
/// Returns [`PlanConflict`] when the request is self-contradictory or
/// cannot be honoured against the current manifest (e.g. removing a
/// package that is not installed). Conflicts are checked in a deterministic
/// order: add/remove overlap, duplicate add, duplicate remove, remove of a
/// not-installed package.
pub fn compute_upgrade_plan(
    current: &Manifest,
    add_packages: &[ResolvedPackage],
    remove_packages: &[String],
) -> Result<UpgradePlan, PlanConflict> {
    // 1. Reject names that appear in both add and remove lists.
    let remove_set: BTreeSet<&str> = remove_packages.iter().map(String::as_str).collect();
    for pkg in add_packages {
        if remove_set.contains(pkg.name.as_str()) {
            return Err(PlanConflict::AddAndRemove(pkg.name.clone()));
        }
    }

    // 2. Reject duplicate names in the add list with mismatched specs;
    //    silently dedupe exact duplicates.
    let mut add_by_name: BTreeMap<&str, &ResolvedPackage> = BTreeMap::new();
    for pkg in add_packages {
        if let Some(prev) = add_by_name.get(pkg.name.as_str()) {
            if prev.version != pkg.version || prev.store_path != pkg.store_path {
                return Err(PlanConflict::DuplicateAddMismatch {
                    name: pkg.name.clone(),
                    first_version: prev.version.clone(),
                    first_store_path: prev.store_path.clone(),
                    second_version: pkg.version.clone(),
                    second_store_path: pkg.store_path.clone(),
                });
            }
            // exact duplicate — silently deduped
            continue;
        }
        add_by_name.insert(pkg.name.as_str(), pkg);
    }

    // 3. Reject duplicate remove entries.
    //    Note: this is a separate set from `remove_set` above because the
    //    spec requires DuplicateRemove to surface AFTER AddAndRemove and
    //    DuplicateAddMismatch. Merging the two checks into one pass would
    //    change which error the caller sees when multiple conflicts exist.
    let mut seen_remove: BTreeSet<&str> = BTreeSet::new();
    for name in remove_packages {
        if !seen_remove.insert(name.as_str()) {
            return Err(PlanConflict::DuplicateRemove(name.clone()));
        }
    }

    // 4. Reject removal of a not-installed package.
    for name in remove_packages {
        if !current.packages.contains_key(name) {
            return Err(PlanConflict::RemoveNotInstalled(name.clone()));
        }
    }

    // ── plan construction ─────────────────────────────────────────────
    let mut packages = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();

    for (name, pkg) in &current.packages {
        if remove_set.contains(name.as_str()) {
            removed.push(PackageVersion {
                name: name.clone(),
                version: pkg.version.clone(),
            });
            continue;
        }

        if let Some(&new_pkg) = add_by_name.get(name.as_str()) {
            if new_pkg.version != pkg.version || new_pkg.store_path != pkg.store_path {
                changed.push(PackageChange {
                    name: name.clone(),
                    from_version: pkg.version.clone(),
                    to_version: new_pkg.version.clone(),
                    from_store_path: pkg.store_path.clone(),
                    to_store_path: new_pkg.store_path.clone(),
                });
            }
            packages.push(new_pkg.clone());
            continue;
        }

        packages.push(manifest_package_to_resolved(name, pkg));
    }

    // Add new packages that aren't replacing existing ones. Iterate
    // `add_by_name` so exact-duplicate adds are added only once.
    for (&name, &pkg) in &add_by_name {
        if !current.packages.contains_key(name) {
            added.push(PackageVersion {
                name: pkg.name.clone(),
                version: pkg.version.clone(),
            });
            packages.push(pkg.clone());
        }
    }

    Ok(UpgradePlan {
        packages,
        added,
        removed,
        changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::types::{InstalledBy, ManifestPackage, PinStrategy};

    fn test_package(name: &str, store_path: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            store_path: store_path.into(),
            cache_url: Some("https://cache.example.com".into()),
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
        assert_eq!(
            parsed
                .packages
                .get("pkg-b")
                .expect("BUG: pkg-b should exist")
                .version,
            "1.0.0"
        );
    }

    #[test]
    fn build_manifest_uses_local_cache() {
        let packages = vec![test_package("some-pkg", "/nix/store/xyz-some-pkg-1.0.0")];

        let manifest = build_manifest(&packages);
        let pkg = manifest
            .packages
            .get("some-pkg")
            .expect("BUG: some-pkg should be present in manifest");

        assert_eq!(pkg.cache, "local");
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
    fn compute_upgrade_plan_detects_new_package() {
        let current = Manifest {
            packages: BTreeMap::new(),
        };
        let new_pkg = test_package("new-widget", "/nix/store/new");
        let plan =
            compute_upgrade_plan(&current, &[new_pkg], &[]).expect("BUG: plan should succeed");
        assert_eq!(plan.added.len(), 1);
        assert_eq!(plan.added[0].name, "new-widget");
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
        let new_pkg = test_package("add-pkg", "/nix/store/add");
        let plan = compute_upgrade_plan(&current, &[new_pkg], &["remove-pkg".into()])
            .expect("BUG: plan should succeed");
        assert_eq!(plan.added.len(), 1);
        assert_eq!(plan.removed.len(), 1);
        // keep-pkg + add-pkg
        assert_eq!(plan.packages.len(), 2);
    }

    #[test]
    fn upgrade_plan_add_replaces_existing() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", PinStrategy::None),
            )]),
        };
        let mut new_pkg = test_package("widget", "/nix/store/widget-2");
        new_pkg.version = "2.0.0".into();
        let plan =
            compute_upgrade_plan(&current, &[new_pkg], &[]).expect("BUG: plan should succeed");
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
        let plan = compute_upgrade_plan(&current, &[], &[]).expect("BUG: plan should succeed");
        assert!(plan.changed.is_empty());
        assert!(plan.added.is_empty());
        assert!(plan.removed.is_empty());
        assert_eq!(plan.packages.len(), 2);
    }

    #[test]
    fn upgrade_plan_empty_manifest_with_adds() {
        let current = Manifest {
            packages: BTreeMap::new(),
        };
        let pkgs = vec![
            test_package("app-a", "/nix/store/a"),
            test_package("app-b", "/nix/store/b"),
        ];
        let plan = compute_upgrade_plan(&current, &pkgs, &[]).expect("BUG: plan should succeed");
        assert_eq!(plan.added.len(), 2);
        assert_eq!(plan.packages.len(), 2);
        assert!(plan.changed.is_empty());
        assert!(plan.removed.is_empty());
    }

    #[test]
    fn upgrade_plan_detects_store_path_only_change() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                ManifestPackage {
                    version: "1.0.0".into(),
                    cache: "default".into(),
                    store_path: "/nix/store/aaa-widget-1.0.0".into(),
                    category: None,
                    description: None,
                    upgrade_strategy: None,
                    install_strategy: None,
                    installed_by: InstalledBy::System,
                    installed_from: "server_a".into(),
                    pinned: PinStrategy::None,
                },
            )]),
        };
        // Same name, same version, DIFFERENT store path.
        let mut new_pkg = test_package("widget", "/nix/store/bbb-widget-1.0.0");
        new_pkg.version = "1.0.0".into();

        let plan =
            compute_upgrade_plan(&current, &[new_pkg], &[]).expect("BUG: plan should succeed");

        assert_eq!(
            plan.changed.len(),
            1,
            "store-path-only change must be flagged"
        );
        assert_eq!(plan.changed[0].name, "widget");
        assert_eq!(plan.changed[0].from_version, "1.0.0");
        assert_eq!(plan.changed[0].to_version, "1.0.0");
        assert_eq!(
            plan.changed[0].from_store_path,
            "/nix/store/aaa-widget-1.0.0"
        );
        assert_eq!(plan.changed[0].to_store_path, "/nix/store/bbb-widget-1.0.0");
    }

    #[test]
    fn compute_upgrade_plan_rejects_add_and_remove_same_name() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", PinStrategy::None),
            )]),
        };
        let add = vec![test_package("widget", "/nix/store/widget-2")];
        let remove = vec!["widget".to_owned()];

        let err = compute_upgrade_plan(&current, &add, &remove)
            .expect_err("expected AddAndRemove conflict");
        assert!(
            matches!(err, PlanConflict::AddAndRemove(ref name) if name == "widget"),
            "got unexpected error: {err:?}"
        );
    }

    #[test]
    fn compute_upgrade_plan_rejects_duplicate_add_mismatch() {
        let current = Manifest::default();
        let mut p1 = test_package("foo", "/nix/store/foo-a");
        p1.version = "1.0.0".into();
        let mut p2 = test_package("foo", "/nix/store/foo-b");
        p2.version = "2.0.0".into();

        let err = compute_upgrade_plan(&current, &[p1, p2], &[])
            .expect_err("expected DuplicateAddMismatch");
        assert!(
            matches!(err, PlanConflict::DuplicateAddMismatch { ref name, .. } if name == "foo"),
            "got unexpected error: {err:?}"
        );
    }

    #[test]
    fn compute_upgrade_plan_allows_exact_duplicate_add() {
        let current = Manifest::default();
        let p1 = test_package("foo", "/nix/store/foo-a");
        let p2 = p1.clone();

        let plan = compute_upgrade_plan(&current, &[p1, p2], &[])
            .expect("BUG: exact duplicate adds should be deduped silently");
        assert_eq!(plan.packages.len(), 1);
        assert_eq!(plan.added.len(), 1);
        assert_eq!(plan.added[0].name, "foo");
    }

    #[test]
    fn compute_upgrade_plan_rejects_duplicate_remove() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", PinStrategy::None),
            )]),
        };
        let err = compute_upgrade_plan(&current, &[], &["widget".into(), "widget".into()])
            .expect_err("expected DuplicateRemove");
        assert!(
            matches!(err, PlanConflict::DuplicateRemove(ref name) if name == "widget"),
            "got unexpected error: {err:?}"
        );
    }

    #[test]
    fn compute_upgrade_plan_rejects_remove_not_installed() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", PinStrategy::None),
            )]),
        };
        let err = compute_upgrade_plan(&current, &[], &["ghost".into()])
            .expect_err("expected RemoveNotInstalled");
        assert!(
            matches!(err, PlanConflict::RemoveNotInstalled(ref name) if name == "ghost"),
            "got unexpected error: {err:?}"
        );
    }
}
