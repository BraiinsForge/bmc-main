// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::index::{self, ResolvePackageError};
use crate::types::{
    InstalledBy, Manifest, ManifestPackage, MergedIndex, PackageChange, PackageVersion,
    ResolvedPackage, UpgradePlan,
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
    #[error("failed to write generated manifest to {path}: {source}")]
    WriteGenerated {
        path: String,
        #[source]
        source: crate::generation_path::GenerationPathError,
    },
}

/// Error type for manifest read operations.
#[derive(Debug, thiserror::Error)]
pub enum ReadManifestError {
    #[error("manifest read failed: {0}")]
    Read(#[source] std::io::Error),
    #[error("manifest parse failed: {0}")]
    Parse(#[source] serde_json::Error),
    /// The `current` symlink is missing, dangling, or otherwise
    /// unreadable. Emitted by `read_current_manifest`; callers that
    /// want graceful degradation (e.g. `apply_profile_change`) catch
    /// this variant and fall back to `read_latest_manifest`.
    #[error("current generation not found at `{path}`")]
    CurrentNotFound { path: String },
    /// Requested generation `N` has no `<N>-link` directory.
    #[error("generation {generation} not found at `{path}`")]
    GenerationNotFound { generation: usize, path: String },
    /// A scan of `profile_dir` (to locate the latest generation) failed
    /// with an I/O error — permission denied, ENOTDIR, or a mid-scan
    /// `DirEntry` failure.
    #[error("failed to scan generations: {0}")]
    ScanGenerations(#[from] crate::profile::BuildProfileError),
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

/// Error type for upgrade plan computation.
#[derive(Debug, thiserror::Error)]
pub enum ComputeUpgradePlanError {
    #[error(transparent)]
    Conflict(#[from] PlanConflict),
    #[error("system packages missing from every index: {}", names.join(", "))]
    MissingSystemPackages { names: Vec<String> },
    #[error("failed to resolve package `{name}`: {source}")]
    Resolve {
        name: String,
        #[source]
        source: ResolvePackageError,
    },
}

/// Build a [`Manifest`] from a slice of resolved packages.
///
/// Each [`ResolvedPackage`] is converted into a [`ManifestPackage`] and keyed
/// by its name. Cache metadata is not persisted — it lives only in the
/// package index.
#[must_use]
pub fn build_manifest(packages: &[ResolvedPackage]) -> Manifest {
    let packages = packages
        .iter()
        .map(|pkg| {
            let entry = ManifestPackage {
                version: pkg.version.clone(),
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
/// The target is prepared through the generated-file helper so a package
/// `manifest` symlink is replaced in the profile without mutating the store.
pub fn write_manifest(profile_path: &Path, manifest: &Manifest) -> Result<(), WriteManifestError> {
    let json = serde_json::to_string_pretty(manifest).map_err(WriteManifestError::Serialize)?;
    crate::generation_path::write_generated_file(
        profile_path,
        Path::new("manifest"),
        json.as_bytes(),
        0o644,
    )
    .map_err(|source| WriteManifestError::WriteGenerated {
        path: profile_path.join("manifest").display().to_string(),
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

/// Read the manifest from the `current` symlink in `profile_dir`.
///
/// Returns [`ReadManifestError::CurrentNotFound`] when the symlink is
/// missing. Prior versions returned an empty manifest in this case.
pub fn read_current_manifest(profile_dir: &Path) -> Result<Manifest, ReadManifestError> {
    let current_link = profile_dir.join(crate::profile::CURRENT_LINK_NAME);
    if current_link.exists() {
        read_manifest(&current_link)
    } else {
        Err(ReadManifestError::CurrentNotFound {
            path: current_link.display().to_string(),
        })
    }
}

/// Read the manifest from generation `N` at `<profile_dir>/<N>-link/manifest`.
///
/// Errors with [`ReadManifestError::GenerationNotFound`] when the
/// generation directory does not exist.
pub fn read_generation_manifest(
    profile_dir: &Path,
    generation: usize,
) -> Result<Manifest, ReadManifestError> {
    let gen_path = profile_dir.join(crate::profile::generation_link_name(generation));
    if !gen_path.exists() {
        return Err(ReadManifestError::GenerationNotFound {
            generation,
            path: gen_path.display().to_string(),
        });
    }
    read_manifest(&gen_path)
}

/// Read the manifest from the highest-numbered generation.
///
/// Returns [`Manifest::default`] when no generations exist — matches
/// the "empty starting state" semantics that existed pre-change on
/// `read_current_manifest`.
pub fn read_latest_manifest(profile_dir: &Path) -> Result<Manifest, ReadManifestError> {
    match crate::profile::max_generation(profile_dir)? {
        Some(n) => read_generation_manifest(profile_dir, n),
        None => Ok(Manifest::default()),
    }
}

/// Dispatch by selector. `BaseSelector::Current` uses
/// [`read_current_manifest`] (surfaces `CurrentNotFound`); the CLI
/// wrapper does NOT fall back — the fallback lives in
/// `apply_profile_change` where it runs under the profile lock.
pub fn read_manifest_by_selector(
    profile_dir: &Path,
    selector: &crate::types::BaseSelector,
) -> Result<Manifest, ReadManifestError> {
    match selector {
        crate::types::BaseSelector::Current => read_current_manifest(profile_dir),
        crate::types::BaseSelector::Latest => read_latest_manifest(profile_dir),
        crate::types::BaseSelector::Generation(n) => read_generation_manifest(profile_dir, *n),
    }
}

/// Convert a manifest package back to a resolved package.
///
/// Manifests do not persist cache metadata. The store path is expected
/// to already be present locally and will be realised through configured
/// Nix substituters if needed.
#[must_use]
pub fn manifest_package_to_resolved(name: &str, mp: &ManifestPackage) -> ResolvedPackage {
    ResolvedPackage {
        name: name.to_owned(),
        version: mp.version.clone(),
        store_path: mp.store_path.clone(),
        category: mp.category.clone(),
        description: mp.description.clone(),
        upgrade_strategy: mp.upgrade_strategy.clone(),
        install_strategy: mp.install_strategy.clone(),
        installed_by: mp.installed_by.clone(),
        installed_from: mp.installed_from.clone(),
        pinned: mp.pinned.clone(),
        metadata: std::collections::BTreeMap::new(),
    }
}

/// Compute an upgrade plan by diffing the current manifest against
/// add/remove requests, optionally resolving each kept package against
/// a merged index.
///
/// - Existing packages not being removed or replaced are kept. When
///   `merged` is `None` they are carried at their current version. When
///   `merged` is `Some`, each kept package is resolved through
///   [`index::resolve_installed_package`]: a newer version satisfying the
///   package's pin strategy becomes a `changed` entry; a package missing
///   from the index is an error for system packages and `stale` for user
///   packages. A package with no satisfying version is reported as `stale`
///   for both kinds and carried at its current version.
/// - `add_packages` are new packages to install (or replace existing ones).
/// - `remove_packages` are package names to remove from the profile.
///
/// Returns [`ComputeUpgradePlanError::Conflict`] when the request is
/// self-contradictory or cannot be honoured against the current manifest
/// (e.g. removing a package that is not installed). Conflicts are checked
/// in a deterministic order: add/remove overlap, duplicate add, duplicate
/// remove, remove of a not-installed package. Returns
/// [`ComputeUpgradePlanError::MissingSystemPackages`] when a system package is
/// absent from every merged index. Returns [`ComputeUpgradePlanError::Resolve`]
/// when a kept package has an invalid version constraint or resolves
/// ambiguously against the merged index.
#[expect(
    clippy::too_many_lines,
    reason = "single-pass plan construction is clearer than splitting"
)]
pub fn compute_upgrade_plan(
    current: &Manifest,
    merged: Option<&MergedIndex>,
    add_packages: &[ResolvedPackage],
    remove_packages: &[String],
) -> Result<UpgradePlan, ComputeUpgradePlanError> {
    // 1. Reject names that appear in both add and remove lists.
    let remove_set: BTreeSet<&str> = remove_packages.iter().map(String::as_str).collect();
    for pkg in add_packages {
        if remove_set.contains(pkg.name.as_str()) {
            return Err(PlanConflict::AddAndRemove(pkg.name.clone()).into());
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
                }
                .into());
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
            return Err(PlanConflict::DuplicateRemove(name.clone()).into());
        }
    }

    // 4. Reject removal of a not-installed package.
    for name in remove_packages {
        if !current.packages.contains_key(name) {
            return Err(PlanConflict::RemoveNotInstalled(name.clone()).into());
        }
    }

    // ── plan construction ─────────────────────────────────────────────
    let mut packages = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut stale = Vec::new();
    let mut missing_system: BTreeSet<String> = BTreeSet::new();

    for (name, pkg) in &current.packages {
        if remove_set.contains(name.as_str()) {
            removed.push(PackageVersion {
                name: name.clone(),
                version: pkg.version.clone(),
            });
            continue;
        }

        if let Some(&new_pkg) = add_by_name.get(name.as_str()) {
            // An explicit install always requests "latest" (version: None),
            // so a resolved version strictly older than the installed one can
            // only be an accidental rollback (e.g. the origin server rolled
            // back). Keep the installed package rather than silently
            // downgrading it. A newer or sideways move (different server,
            // same-or-higher version) still applies. A malformed version on
            // either side disables the guard rather than blocking the move.
            let is_downgrade = matches!(
                (
                    index::parse_package_version(&pkg.version),
                    index::parse_package_version(&new_pkg.version),
                ),
                (Some(installed), Some(resolved)) if resolved < installed
            );
            if is_downgrade {
                tracing::warn!(
                    package = name,
                    installed = pkg.version,
                    resolved = new_pkg.version,
                    "refusing to downgrade an explicitly installed package; keeping installed version"
                );
                packages.push(manifest_package_to_resolved(name, pkg));
                continue;
            }

            // An explicit install may move the version/server, but must not
            // change the package's ownership class or category: demoting a
            // System package to User would turn a later index missing that
            // package into a silent stale carry instead of the loud
            // `MissingSystemPackages` error that a broken image demands.
            let mut replacement = new_pkg.clone();
            replacement.installed_by = pkg.installed_by.clone();
            replacement.category.clone_from(&pkg.category);

            if replacement.version != pkg.version || replacement.store_path != pkg.store_path {
                changed.push(PackageChange {
                    name: name.clone(),
                    from_version: pkg.version.clone(),
                    to_version: replacement.version.clone(),
                    from_store_path: pkg.store_path.clone(),
                    to_store_path: replacement.store_path.clone(),
                });
            }
            packages.push(replacement);
            continue;
        }

        // Resolve kept packages against the merged index for upgrade
        // detection when one is available.
        if let Some(merged) = merged {
            match index::resolve_installed_package(merged, name, pkg) {
                Ok(resolved) => {
                    // The resolver renders normalized versions ("0.8" →
                    // "0.8.0") while manifest versions are verbatim, so
                    // only the store path decides whether the package
                    // actually changes.
                    if resolved.store_path != pkg.store_path {
                        changed.push(PackageChange {
                            name: name.clone(),
                            from_version: pkg.version.clone(),
                            to_version: resolved.version.clone(),
                            from_store_path: pkg.store_path.clone(),
                            to_store_path: resolved.store_path.clone(),
                        });
                    }
                    packages.push(resolved);
                }
                Err(ResolvePackageError::PackageNotFound(_))
                    if pkg.installed_by == InstalledBy::System =>
                {
                    missing_system.insert(name.clone());
                }
                Err(
                    ResolvePackageError::PackageNotFound(_)
                    | ResolvePackageError::VersionNotFound { .. },
                ) => {
                    // User packages missing from every index and packages with
                    // no matching version stay stale.
                    stale.push(PackageVersion {
                        name: name.clone(),
                        version: pkg.version.clone(),
                    });
                    packages.push(manifest_package_to_resolved(name, pkg));
                }
                Err(err @ ResolvePackageError::InvalidVersionConstraint { .. })
                    if pkg.installed_by != InstalledBy::System =>
                {
                    // A user package with an unparseable pin must not
                    // block upgrades of everything else; keep it stale
                    // like a package that vanished from the indexes.
                    tracing::warn!(package = name, %err, "ignoring unparseable pin");
                    stale.push(PackageVersion {
                        name: name.clone(),
                        version: pkg.version.clone(),
                    });
                    packages.push(manifest_package_to_resolved(name, pkg));
                }
                Err(
                    e @ (ResolvePackageError::Ambiguous { .. }
                    | ResolvePackageError::InvalidVersionConstraint { .. }),
                ) => {
                    return Err(ComputeUpgradePlanError::Resolve {
                        name: name.clone(),
                        source: e,
                    });
                }
            }
            continue;
        }

        packages.push(manifest_package_to_resolved(name, pkg));
    }

    if !missing_system.is_empty() {
        return Err(ComputeUpgradePlanError::MissingSystemPackages {
            names: missing_system.into_iter().collect(),
        });
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
        stale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::index::ResolvePackageError;
    use crate::types::{InstalledBy, ManifestPackage, MergedPackageEntry};
    use semver::Version;

    /// Build a [`MergedIndex`] from a set of entries, deriving `by_name`.
    fn merged_index_with(entries: Vec<MergedPackageEntry>) -> MergedIndex {
        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, entry) in entries.iter().enumerate() {
            by_name.entry(entry.name.clone()).or_default().push(i);
        }
        MergedIndex {
            packages: entries,
            by_name,
        }
    }

    fn merged_entry(name: &str, version: &str, priority: u32) -> MergedPackageEntry {
        MergedPackageEntry {
            name: name.into(),
            version: Version::parse(version).expect("BUG: test version is valid semver"),
            store_path: format!("/nix/store/hash-{name}-{version}"),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            server_id: "forge".into(),
            server_priority: priority,
            metadata: BTreeMap::new(),
        }
    }

    fn test_package(name: &str, store_path: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            store_path: store_path.into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: None,
            metadata: BTreeMap::new(),
        }
    }

    fn test_manifest_package(
        version: &str,
        installed_from: &str,
        pinned: Option<String>,
    ) -> ManifestPackage {
        test_manifest_package_installed_by(version, installed_from, pinned, InstalledBy::System)
    }

    fn test_manifest_package_installed_by(
        version: &str,
        installed_from: &str,
        pinned: Option<String>,
        installed_by: InstalledBy,
    ) -> ManifestPackage {
        ManifestPackage {
            version: version.into(),
            store_path: format!("/nix/store/hash-pkg-{version}"),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by,
            installed_from: installed_from.into(),
            pinned,
        }
    }

    fn empty_merged_index() -> MergedIndex {
        merged_index_with(Vec::new())
    }

    /// A manifest minted from a factory index (the `build-profile` /
    /// `reset-profile` path) must keep upgrading when a factory-shipped
    /// widget disappears from every server index — the widget goes
    /// stale — while a missing required package (`core`, `nix`) must
    /// still abort the plan loudly.
    #[test]
    fn factory_profile_widget_vanishing_from_index_does_not_block_upgrade() {
        use crate::types::{PackageEntry, PackageIndex};

        let entry = |name: &str| PackageEntry {
            name: name.into(),
            version: "1.0.0".into(),
            cache: None,
            store_path: format!("/nix/store/abc-{name}-1.0.0"),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            server_id: String::new(),
            metadata: BTreeMap::new(),
        };
        let factory_index = PackageIndex {
            version: 1,
            provenance: None,
            indexes: vec![],
            caches: vec![],
            packages: vec![entry("core"), entry("widget-clock")],
        };
        let factory_packages =
            crate::index::resolve_all_from_index(&factory_index, &["core".into()])
                .expect("the factory index contains every required system package");
        let manifest = build_manifest(&factory_packages);

        let without_widget = merged_index_with(vec![merged_entry("core", "2.0.0", 10)]);
        let plan = compute_upgrade_plan(&manifest, Some(&without_widget), &[], &[])
            .expect("a vanished factory widget must not abort the upgrade");
        assert_eq!(plan.stale.len(), 1);
        assert_eq!(plan.stale[0].name, "widget-clock");

        let without_core = merged_index_with(vec![merged_entry("widget-clock", "2.0.0", 10)]);
        let err = compute_upgrade_plan(&manifest, Some(&without_core), &[], &[])
            .expect_err("a vanished core must abort the upgrade");
        assert!(matches!(
            err,
            ComputeUpgradePlanError::MissingSystemPackages { names } if names == ["core"]
        ));
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
    fn build_manifest_does_not_persist_cache_metadata() {
        let pkg = test_package("some-pkg", "/nix/store/xyz-some-pkg-1.0.0");
        let manifest = build_manifest(&[pkg]);
        let entry = manifest
            .packages
            .get("some-pkg")
            .expect("BUG: some-pkg should be present");

        assert_eq!(entry.store_path, "/nix/store/xyz-some-pkg-1.0.0");
        assert_eq!(entry.installed_from, "local");
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

    // ---- read_generation_manifest tests ----

    #[test]
    fn read_generation_manifest_returns_manifest() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = dir.path().join("bmc");
        let gen_dir = profile_dir.join("2-link");
        std::fs::create_dir_all(&gen_dir).expect("BUG: mkdir gen");

        let packages = vec![test_package("pkg-a", "/nix/store/aaa")];
        let manifest = build_manifest(&packages);
        write_manifest(&gen_dir, &manifest).expect("BUG: write manifest");

        let read_back = read_generation_manifest(&profile_dir, 2).expect("BUG: read generation 2");
        assert!(read_back.packages.contains_key("pkg-a"));
    }

    #[test]
    fn read_generation_manifest_missing_errors_with_generation_not_found() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = dir.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        let err =
            read_generation_manifest(&profile_dir, 7).expect_err("missing generation must error");
        assert!(
            matches!(
                err,
                ReadManifestError::GenerationNotFound { generation: 7, .. }
            ),
            "expected GenerationNotFound, got {err:?}"
        );
    }

    // ---- read_latest_manifest tests ----

    #[test]
    fn read_latest_manifest_returns_highest_generation() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = dir.path().join("bmc");

        for (n, pkg_name) in [(1, "pkg-old"), (2, "pkg-mid"), (3, "pkg-new")] {
            let gen_dir = profile_dir.join(format!("{n}-link"));
            std::fs::create_dir_all(&gen_dir).expect("BUG: mkdir gen");
            let manifest =
                build_manifest(&[test_package(pkg_name, &format!("/nix/store/{pkg_name}"))]);
            write_manifest(&gen_dir, &manifest).expect("BUG: write manifest");
        }

        let latest = read_latest_manifest(&profile_dir).expect("BUG: latest should read");
        assert!(latest.packages.contains_key("pkg-new"));
        assert!(!latest.packages.contains_key("pkg-mid"));
    }

    #[test]
    fn read_latest_manifest_propagates_scan_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = dir.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        let mut perms = std::fs::metadata(&profile_dir)
            .expect("BUG: stat")
            .permissions();
        perms.set_mode(0o000);
        std::fs::set_permissions(&profile_dir, perms).expect("BUG: chmod");

        let result = read_latest_manifest(&profile_dir);

        // Restore perms before assert so cleanup works even if we panic below.
        let mut restore = std::fs::metadata(&profile_dir)
            .expect("BUG: stat")
            .permissions();
        restore.set_mode(0o755);
        std::fs::set_permissions(&profile_dir, restore).expect("BUG: chmod");

        assert!(
            matches!(result, Err(ReadManifestError::ScanGenerations(_))),
            "expected ScanGenerations error, got {result:?}"
        );
    }

    #[test]
    fn read_latest_manifest_empty_dir_returns_default() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = dir.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        let latest = read_latest_manifest(&profile_dir).expect("BUG: empty latest");
        assert!(latest.packages.is_empty());
    }

    // ---- read_manifest_by_selector tests ----

    #[test]
    fn read_manifest_by_selector_current_latest_and_generation() {
        use crate::types::BaseSelector;

        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = dir.path().join("bmc");

        // Build gen 1 (pkg-one) and gen 2 (pkg-two); activate gen 1.
        for (n, pkg_name) in [(1, "pkg-one"), (2, "pkg-two")] {
            let gen_dir = profile_dir.join(format!("{n}-link"));
            std::fs::create_dir_all(&gen_dir).expect("BUG: mkdir gen");
            let manifest =
                build_manifest(&[test_package(pkg_name, &format!("/nix/store/{pkg_name}"))]);
            write_manifest(&gen_dir, &manifest).expect("BUG: write manifest");
        }
        std::os::unix::fs::symlink("1-link", profile_dir.join("current"))
            .expect("BUG: symlink current -> 1-link");

        let cur = read_manifest_by_selector(&profile_dir, &BaseSelector::Current)
            .expect("BUG: current read");
        assert!(cur.packages.contains_key("pkg-one"));

        let latest = read_manifest_by_selector(&profile_dir, &BaseSelector::Latest)
            .expect("BUG: latest read");
        assert!(latest.packages.contains_key("pkg-two"));

        let g2 = read_manifest_by_selector(&profile_dir, &BaseSelector::Generation(2))
            .expect("BUG: gen 2 read");
        assert!(g2.packages.contains_key("pkg-two"));
    }

    // ---- compute_upgrade_plan tests ----

    #[test]
    fn compute_upgrade_plan_detects_new_package() {
        let current = Manifest {
            packages: BTreeMap::new(),
        };
        let new_pkg = test_package("new-widget", "/nix/store/new");
        let plan = compute_upgrade_plan(&current, None, &[new_pkg], &[])
            .expect("BUG: plan should succeed");
        assert_eq!(plan.added.len(), 1);
        assert_eq!(plan.added[0].name, "new-widget");
    }

    #[test]
    fn upgrade_plan_with_add_and_remove() {
        let current = Manifest {
            packages: BTreeMap::from([
                (
                    "keep-pkg".into(),
                    test_manifest_package("1.0.0", "server_a", None),
                ),
                (
                    "remove-pkg".into(),
                    test_manifest_package("1.0.0", "server_a", None),
                ),
            ]),
        };
        let new_pkg = test_package("add-pkg", "/nix/store/add");
        let plan = compute_upgrade_plan(&current, None, &[new_pkg], &["remove-pkg".into()])
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
                test_manifest_package("1.0.0", "server_a", None),
            )]),
        };
        let mut new_pkg = test_package("widget", "/nix/store/widget-2");
        new_pkg.version = "2.0.0".into();
        let plan = compute_upgrade_plan(&current, None, &[new_pkg], &[])
            .expect("BUG: plan should succeed");
        // Should count as a change, not an add
        assert_eq!(plan.changed.len(), 1);
        assert!(plan.added.is_empty());
        assert_eq!(plan.packages.len(), 1);
    }

    #[test]
    fn install_keeps_installed_when_resolved_is_older() {
        // A server rollback can make "latest available" older than what is
        // installed. An install request (always "latest") must not become a
        // silent downgrade: keep the installed version, record no change.
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("2.0.0", "server_a", None),
            )]),
        };
        let mut older = test_package("widget", "/nix/store/widget-1");
        older.version = "1.0.0".into();
        older.installed_by = InstalledBy::User;

        let plan =
            compute_upgrade_plan(&current, None, &[older], &[]).expect("BUG: plan should succeed");

        assert!(
            plan.changed.is_empty(),
            "downgrade must not record a change"
        );
        assert_eq!(plan.packages.len(), 1);
        let kept = &plan.packages[0];
        assert_eq!(kept.version, "2.0.0", "installed version must be kept");
        assert_eq!(kept.store_path, "/nix/store/hash-pkg-2.0.0");
    }

    #[test]
    fn install_applies_sideways_move_at_same_version() {
        // Installing a name already present at the same version but a
        // different store path (e.g. a rebuild or a different server) is not a
        // downgrade and must still apply — the relaxed install path is intact.
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", None),
            )]),
        };
        let mut sideways = test_package("widget", "/nix/store/widget-from-server-b");
        sideways.version = "1.0.0".into();

        let plan = compute_upgrade_plan(&current, None, &[sideways], &[])
            .expect("BUG: plan should succeed");

        assert_eq!(plan.changed.len(), 1, "store-path move is a change");
        assert_eq!(
            plan.packages[0].store_path,
            "/nix/store/widget-from-server-b"
        );
    }

    #[test]
    fn install_preserves_system_ownership_so_later_missing_stays_loud() {
        // Re-installing a name that is present as a System package must not
        // demote it to User: a later index missing it must still raise the
        // loud `MissingSystemPackages` error, not a silent stale carry.
        let mut system_pkg =
            test_manifest_package_installed_by("1.0.0", "server_a", None, InstalledBy::System);
        system_pkg.category = Some("system".into());
        let current = Manifest {
            packages: BTreeMap::from([("core".into(), system_pkg)]),
        };
        let mut user_install = test_package("core", "/nix/store/core-2");
        user_install.version = "2.0.0".into();
        user_install.installed_by = InstalledBy::User;
        user_install.category = Some("widget".into());

        let plan = compute_upgrade_plan(&current, None, &[user_install], &[])
            .expect("BUG: plan should succeed");

        assert_eq!(plan.packages.len(), 1);
        let replaced = &plan.packages[0];
        assert_eq!(replaced.version, "2.0.0", "version move still applies");
        assert_eq!(
            replaced.installed_by,
            InstalledBy::System,
            "ownership class must be preserved"
        );
        assert_eq!(replaced.category.as_deref(), Some("system"));

        // Feed the replaced package back as a manifest and resolve it against
        // an index that no longer lists it: it must fail loud as a missing
        // system package, proving ownership survived the install.
        let after = build_manifest(&plan.packages);
        let err = compute_upgrade_plan(&after, Some(&empty_merged_index()), &[], &[])
            .expect_err("BUG: missing system package must error");
        assert!(matches!(
            err,
            ComputeUpgradePlanError::MissingSystemPackages { names } if names == vec!["core".to_owned()]
        ));
    }

    #[test]
    fn upgrade_plan_offline_mode_keeps_all_current() {
        let current = Manifest {
            packages: BTreeMap::from([
                (
                    "widget".into(),
                    test_manifest_package("1.0.0", "server_a", None),
                ),
                (
                    "gadget".into(),
                    test_manifest_package("3.0.0", "server_a", None),
                ),
            ]),
        };
        let plan =
            compute_upgrade_plan(&current, None, &[], &[]).expect("BUG: plan should succeed");
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
        let plan =
            compute_upgrade_plan(&current, None, &pkgs, &[]).expect("BUG: plan should succeed");
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
                    store_path: "/nix/store/aaa-widget-1.0.0".into(),
                    category: None,
                    description: None,
                    upgrade_strategy: None,
                    install_strategy: None,
                    installed_by: InstalledBy::System,
                    installed_from: "server_a".into(),
                    pinned: None,
                },
            )]),
        };
        // Same name, same version, DIFFERENT store path.
        let mut new_pkg = test_package("widget", "/nix/store/bbb-widget-1.0.0");
        new_pkg.version = "1.0.0".into();

        let plan = compute_upgrade_plan(&current, None, &[new_pkg], &[])
            .expect("BUG: plan should succeed");

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
                test_manifest_package("1.0.0", "server_a", None),
            )]),
        };
        let add = vec![test_package("widget", "/nix/store/widget-2")];
        let remove = vec!["widget".to_owned()];

        let err = compute_upgrade_plan(&current, None, &add, &remove)
            .expect_err("expected AddAndRemove conflict");
        assert!(
            matches!(
                err,
                ComputeUpgradePlanError::Conflict(PlanConflict::AddAndRemove(ref name))
                    if name == "widget"
            ),
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

        let err = compute_upgrade_plan(&current, None, &[p1, p2], &[])
            .expect_err("expected DuplicateAddMismatch");
        assert!(
            matches!(
                err,
                ComputeUpgradePlanError::Conflict(PlanConflict::DuplicateAddMismatch {
                    ref name,
                    ..
                }) if name == "foo"
            ),
            "got unexpected error: {err:?}"
        );
    }

    #[test]
    fn compute_upgrade_plan_allows_exact_duplicate_add() {
        let current = Manifest::default();
        let p1 = test_package("foo", "/nix/store/foo-a");
        let p2 = p1.clone();

        let plan = compute_upgrade_plan(&current, None, &[p1, p2], &[])
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
                test_manifest_package("1.0.0", "server_a", None),
            )]),
        };
        let err = compute_upgrade_plan(&current, None, &[], &["widget".into(), "widget".into()])
            .expect_err("expected DuplicateRemove");
        assert!(
            matches!(
                err,
                ComputeUpgradePlanError::Conflict(PlanConflict::DuplicateRemove(ref name))
                    if name == "widget"
            ),
            "got unexpected error: {err:?}"
        );
    }

    #[test]
    fn read_current_manifest_missing_symlink_errors_with_current_not_found() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = dir.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        let err = read_current_manifest(&profile_dir)
            .expect_err("missing `current` symlink must now error");
        assert!(
            matches!(err, ReadManifestError::CurrentNotFound { .. }),
            "expected CurrentNotFound, got {err:?}"
        );
    }

    #[test]
    fn compute_upgrade_plan_rejects_remove_not_installed() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".into(),
                test_manifest_package("1.0.0", "server_a", None),
            )]),
        };
        let err = compute_upgrade_plan(&current, None, &[], &["ghost".into()])
            .expect_err("expected RemoveNotInstalled");
        assert!(
            matches!(
                err,
                ComputeUpgradePlanError::Conflict(PlanConflict::RemoveNotInstalled(ref name))
                    if name == "ghost"
            ),
            "got unexpected error: {err:?}"
        );
    }

    #[test]
    fn compute_upgrade_plan_without_merged_index_keeps_existing_behavior() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "clock".to_owned(),
                test_manifest_package("1.0.0", "local", None),
            )]),
        };

        let plan = compute_upgrade_plan(&current, None, &[], &[])
            .expect("BUG: unchanged manifest should plan");

        assert_eq!(plan.packages.len(), 1);
        assert!(plan.added.is_empty());
        assert!(plan.removed.is_empty());
        assert!(plan.changed.is_empty());
        assert!(plan.stale.is_empty());
    }

    #[test]
    fn compute_upgrade_plan_reports_stale_package_missing_from_merged_index() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "clock".to_owned(),
                test_manifest_package_installed_by("1.0.0", "forge", None, InstalledBy::User),
            )]),
        };
        let merged = empty_merged_index();

        let plan = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect("BUG: stale package should be carried");

        assert_eq!(plan.packages[0].name, "clock");
        assert_eq!(plan.stale.len(), 1);
        assert_eq!(plan.stale[0].name, "clock");
    }

    #[test]
    fn compute_upgrade_plan_fails_when_system_package_missing_from_index() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "core-pkg".to_owned(),
                test_manifest_package_installed_by("1.0.0", "forge", None, InstalledBy::System),
            )]),
        };
        let merged = empty_merged_index();

        let err = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect_err("missing system package must fail");

        assert!(
            matches!(
                err,
                ComputeUpgradePlanError::MissingSystemPackages { ref names }
                    if names == &["core-pkg".to_owned()]
            ),
            "got unexpected error: {err:?}"
        );
    }

    #[test]
    fn compute_upgrade_plan_reports_all_missing_system_packages_sorted() {
        let current = Manifest {
            packages: BTreeMap::from([
                (
                    "nix".to_owned(),
                    test_manifest_package_installed_by("1.0.0", "forge", None, InstalledBy::System),
                ),
                (
                    "core".to_owned(),
                    test_manifest_package_installed_by("1.0.0", "forge", None, InstalledBy::System),
                ),
            ]),
        };
        let merged = empty_merged_index();

        let err = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect_err("missing system packages must fail");

        let ComputeUpgradePlanError::MissingSystemPackages { names } = err else {
            panic!("expected MissingSystemPackages, got: {err:?}");
        };
        assert_eq!(names, vec!["core".to_owned(), "nix".to_owned()]);
    }

    #[test]
    fn compute_upgrade_plan_keeps_stale_user_package_missing_from_index() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".to_owned(),
                test_manifest_package_installed_by("1.0.0", "forge", None, InstalledBy::User),
            )]),
        };
        let merged = empty_merged_index();

        let plan = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect("BUG: missing user package must stay stale");

        assert_eq!(plan.stale.len(), 1);
        assert_eq!(plan.stale[0].name, "widget");
    }

    #[test]
    fn compute_upgrade_plan_keeps_system_package_with_no_candidate_version() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "core-pkg".to_owned(),
                test_manifest_package_installed_by("2.0.0", "forge", None, InstalledBy::System),
            )]),
        };
        let merged = merged_index_with(vec![merged_entry("core-pkg", "1.0.0", 0)]);

        let plan = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect("BUG: filtered-out system package must stay stale");

        assert_eq!(plan.stale.len(), 1);
        assert_eq!(plan.stale[0].name, "core-pkg");
    }

    #[test]
    fn compute_upgrade_plan_changes_installed_package_from_merged_index() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "clock".to_owned(),
                test_manifest_package("1.0.0", "forge", None),
            )]),
        };
        let merged = merged_index_with(vec![merged_entry("clock", "1.1.0", 0)]);

        let plan = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect("BUG: newer version should plan as a change");

        assert_eq!(plan.changed.len(), 1);
        assert_eq!(plan.changed[0].name, "clock");
        assert_eq!(plan.changed[0].from_version, "1.0.0");
        assert_eq!(plan.changed[0].to_version, "1.1.0");
        assert_eq!(plan.packages[0].version, "1.1.0");
        assert!(plan.stale.is_empty());
    }

    #[test]
    fn compute_upgrade_plan_returns_resolve_error_on_ambiguous_installed_package() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "clock".to_owned(),
                test_manifest_package("1.0.0", "forge", None),
            )]),
        };
        // Two entries at the same version and priority with different
        // store paths cannot be disambiguated, so resolution fails.
        let mut other = merged_entry("clock", "1.1.0", 0);
        other.store_path = "/nix/store/other-clock-1.1.0".to_owned();
        let merged = merged_index_with(vec![merged_entry("clock", "1.1.0", 0), other]);

        let err = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect_err("expected ambiguous resolution");
        assert!(
            matches!(
                err,
                ComputeUpgradePlanError::Resolve {
                    ref name,
                    source: ResolvePackageError::Ambiguous { .. },
                } if name == "clock"
            ),
            "got unexpected error: {err:?}"
        );
    }

    #[test]
    fn compute_upgrade_plan_succeeds_for_legacy_none_pin_manifest() {
        // A pre-branch manifest persists `"pinned": "none"`. After
        // deserialization it must read as unpinned and plan a normal
        // upgrade, not abort with a Resolve error on the bogus constraint.
        let manifest_json = r#"{
            "packages": {
                "clock": {
                    "version": "1.0.0",
                    "store_path": "/nix/store/hash-clock-1.0.0",
                    "installed_by": "system",
                    "installed_from": "forge",
                    "pinned": "none"
                }
            }
        }"#;
        let current: Manifest =
            serde_json::from_str(manifest_json).expect("BUG: legacy manifest should deserialize");
        let merged = merged_index_with(vec![merged_entry("clock", "1.1.0", 0)]);

        let plan = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect("BUG: legacy none-pin package must plan, not error");

        assert!(plan.stale.is_empty(), "legacy none-pin must not be stale");
        assert_eq!(plan.changed.len(), 1);
        assert_eq!(plan.changed[0].name, "clock");
        assert_eq!(plan.changed[0].to_version, "1.1.0");
    }

    #[test]
    fn compute_upgrade_plan_keeps_installed_when_index_only_older() {
        // The index offers only a version older than installed; the
        // no-downgrade guard marks the package stale and keeps the
        // installed store path rather than activating the older one.
        let current = Manifest {
            packages: BTreeMap::from([(
                "clock".to_owned(),
                test_manifest_package("1.5.0", "forge", Some("^1.0.0".to_owned())),
            )]),
        };
        let merged = merged_index_with(vec![merged_entry("clock", "1.4.0", 0)]);

        let plan = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect("BUG: older-only index should keep installed, not error");

        assert!(plan.changed.is_empty(), "must not record a downgrade");
        assert_eq!(plan.stale.len(), 1);
        assert_eq!(plan.stale[0].name, "clock");
        assert_eq!(plan.stale[0].version, "1.5.0");
        assert_eq!(plan.packages[0].version, "1.5.0");
        assert_eq!(plan.packages[0].store_path, "/nix/store/hash-pkg-1.5.0");
    }

    #[test]
    fn compute_upgrade_plan_returns_error_for_invalid_pin_constraint() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "clock".to_owned(),
                test_manifest_package("1.0.0", "forge", Some("not-a-version".to_owned())),
            )]),
        };
        let merged = merged_index_with(vec![merged_entry("clock", "1.0.0", 0)]);

        let err = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect_err("expected resolve error for invalid pin constraint");
        assert!(
            matches!(
                err,
                ComputeUpgradePlanError::Resolve {
                    ref name,
                    source: ResolvePackageError::InvalidVersionConstraint { .. },
                } if name == "clock"
            ),
            "got unexpected error: {err:?}"
        );
    }

    #[test]
    fn compute_upgrade_plan_keeps_user_package_with_invalid_pin_stale() {
        // A user package's broken pin must degrade like a vanished user
        // package instead of blocking every other upgrade in the plan.
        let current = Manifest {
            packages: BTreeMap::from([(
                "widget".to_owned(),
                test_manifest_package_installed_by(
                    "1.0.0",
                    "forge",
                    Some("not-a-version".to_owned()),
                    InstalledBy::User,
                ),
            )]),
        };
        let merged = merged_index_with(vec![merged_entry("widget", "1.1.0", 0)]);

        let plan = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect("BUG: a user package's broken pin must not abort the plan");

        assert_eq!(plan.stale.len(), 1);
        assert_eq!(plan.stale[0].name, "widget");
        assert_eq!(plan.packages[0].store_path, "/nix/store/hash-pkg-1.0.0");
        assert!(plan.changed.is_empty());
    }

    #[test]
    fn compute_upgrade_plan_ignores_version_normalization_without_store_path_change() {
        // The resolver renders "0.8" as "0.8.0"; with an identical store
        // path that rendering difference is not a package change.
        let entry = merged_entry("clock", "0.8.0", 0);
        let mut manifest_pkg = test_manifest_package("0.8", "forge", None);
        manifest_pkg.store_path.clone_from(&entry.store_path);
        let current = Manifest {
            packages: BTreeMap::from([("clock".to_owned(), manifest_pkg)]),
        };
        let merged = merged_index_with(vec![entry]);

        let plan = compute_upgrade_plan(&current, Some(&merged), &[], &[])
            .expect("BUG: plan should succeed");

        assert!(
            plan.changed.is_empty(),
            "a phantom change with identical store paths must not be planned"
        );
        assert!(plan.stale.is_empty());
    }
}
