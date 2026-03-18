// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::path::Path;

use crate::types::{
    GcConfig, InstallResult, Manifest, ProfileGeneration, ResolvedPackage, StrategySummary,
    UpgradePlan,
};
use crate::{activation, gc, manifest, profile, store};

/// Errors that can occur during an install/upgrade operation.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("profile lock failed: {0}")]
    Lock(#[source] profile::BuildProfileError),
    #[error(transparent)]
    CopyStorePaths(#[from] store::CopyStorePathsError),
    #[error(transparent)]
    BuildProfile(#[from] profile::BuildProfileError),
    #[error("activation failed: {0}")]
    Activation(#[from] activation::ActivationError),
    #[error(transparent)]
    CleanupGenerations(#[from] gc::CleanupGenerationsError),
    #[error(transparent)]
    ReadManifest(#[from] manifest::ReadManifestError),
}

/// Progress callback for upgrade phases.
pub trait UpgradeProgress: Send + Sync {
    fn on_phase(&self, phase: &str);
}

/// Merge the current manifest with new packages.
///
/// New packages replace existing entries by name. Existing packages
/// not present in `packages` are converted from manifest entries and
/// kept.
#[must_use]
pub fn merge_installed_with_new(
    current: &Manifest,
    packages: &[ResolvedPackage],
) -> Vec<ResolvedPackage> {
    let mut result: BTreeMap<String, ResolvedPackage> = current
        .packages
        .iter()
        .map(|(name, mp)| {
            (
                name.clone(),
                manifest::manifest_package_to_resolved(name, mp),
            )
        })
        .collect();

    for pkg in packages {
        result.insert(pkg.name.clone(), pkg.clone());
    }

    result.into_values().collect()
}

/// Apply already-resolved packages into the current profile.
///
/// The caller is responsible for fetching indexes, resolving
/// package names, and running checker packages beforehand.
///
/// `current` is the active profile generation (used to merge
/// existing manifest packages with the upgrade plan). Pass
/// `None` for first-time installation.
#[expect(clippy::too_many_arguments)]
pub async fn apply_profile_change(
    current: Option<&ProfileGeneration>,
    profile_dir: &Path,
    gc_config: Option<&GcConfig>,
    plan: &UpgradePlan,
    activate: bool,
    progress: Option<&dyn UpgradeProgress>,
    hooks_dir: &str,
    hooks_override_path: Option<&Path>,
) -> Result<InstallResult, InstallError> {
    // 1. Acquire profile lock
    let _lock = profile::lock_profile(profile_dir)
        .await
        .map_err(InstallError::Lock)?;

    // 2. Merge existing manifest with new packages
    let all_packages = if let Some(gen_info) = current {
        let current_manifest = manifest::read_manifest(&gen_info.path)?;
        merge_installed_with_new(&current_manifest, &plan.packages)
    } else {
        plan.packages.clone()
    };

    // 3. Copy store paths — ONLY packages from the upgrade plan
    // (which have valid cache_url). Kept packages from the manifest
    // have empty cache_url and must NOT be passed to copy_store_paths.
    if let Some(p) = progress {
        p.on_phase("copying");
    }
    store::copy_store_paths(&store::TokioCommandRunner, &plan.packages, None).await?;

    // 4. Build new profile generation
    if let Some(p) = progress {
        p.on_phase("building");
    }
    let gen_number = profile::next_generation_number(profile_dir)?;
    let generation = profile::build_profile(
        profile_dir,
        gen_number,
        &all_packages,
        hooks_dir,
        hooks_override_path,
    )
    .await?;

    // 5. Optionally activate
    if activate {
        if let Some(p) = progress {
            p.on_phase("activating");
        }
        profile::activate_profile(profile_dir, generation.number, &generation.path).await?;
    }

    // 6. GC old generations (optional)
    if let Some(gc_config) = gc_config {
        gc::cleanup_generations(profile_dir, gc_config)?;
    }

    Ok(InstallResult {
        strategies: StrategySummary::from_packages(&all_packages),
        generation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{InstalledBy, ManifestPackage, PinStrategy};

    fn test_resolved_package(name: &str, store_path: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            store_path: store_path.into(),
            cache_url: "https://cache.example.com".into(),
            cache_name: "default".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        }
    }

    fn test_manifest_package(version: &str, cache: &str) -> ManifestPackage {
        ManifestPackage {
            version: version.into(),
            cache: cache.into(),
            store_path: format!("/nix/store/hash-pkg-{version}"),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        }
    }

    #[test]
    fn merge_installed_with_new_replaces_existing() {
        let current = Manifest {
            packages: BTreeMap::from([
                ("pkg-a".into(), test_manifest_package("1.0.0", "default")),
                ("pkg-b".into(), test_manifest_package("2.0.0", "default")),
            ]),
        };
        let new_packages = vec![test_resolved_package("pkg-a", "/nix/store/new-a")];
        let result = merge_installed_with_new(&current, &new_packages);

        assert_eq!(result.len(), 2);
        let a = result
            .iter()
            .find(|p| p.name == "pkg-a")
            .expect("BUG: pkg-a");
        assert_eq!(a.store_path, "/nix/store/new-a");
        let b = result
            .iter()
            .find(|p| p.name == "pkg-b")
            .expect("BUG: pkg-b");
        assert_eq!(b.store_path, "/nix/store/hash-pkg-2.0.0");
    }

    #[test]
    fn merge_installed_with_new_adds_new_packages() {
        let current = Manifest {
            packages: BTreeMap::from([(
                "existing".into(),
                test_manifest_package("1.0.0", "default"),
            )]),
        };
        let new_packages = vec![test_resolved_package("brand-new", "/nix/store/brand-new")];
        let result = merge_installed_with_new(&current, &new_packages);

        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|p| p.name == "existing"));
        assert!(result.iter().any(|p| p.name == "brand-new"));
    }

    #[test]
    fn merge_installed_with_empty_manifest() {
        let current = Manifest::default();
        let new_packages = vec![
            test_resolved_package("a", "/nix/store/a"),
            test_resolved_package("b", "/nix/store/b"),
        ];
        let result = merge_installed_with_new(&current, &new_packages);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn merge_installed_with_empty_new_packages() {
        let current = Manifest {
            packages: BTreeMap::from([
                ("pkg-a".into(), test_manifest_package("1.0.0", "default")),
                ("pkg-b".into(), test_manifest_package("2.0.0", "default")),
            ]),
        };
        let result = merge_installed_with_new(&current, &[]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn manifest_to_resolved_sets_empty_cache_url() {
        let mp = test_manifest_package("1.0.0", "my-cache");
        let resolved = manifest::manifest_package_to_resolved("test", &mp);
        assert!(resolved.cache_url.is_none());
        assert_eq!(resolved.cache_name, "my-cache");
    }
}
