// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::path::Path;

use crate::types::{
    GcConfig, InstallResult, Manifest, ManifestPackage, ProfileGeneration, ResolvedPackage,
    StrategySummary, UpgradePlan,
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

/// Convert a manifest package back to a resolved package.
///
/// This is lossy — `ManifestPackage` does not store `cache_url`,
/// only `cache` (name). The `cache_url` is set to `None`; the
/// store path is expected to already be present locally.
fn manifest_package_to_resolved(name: &str, mp: &ManifestPackage) -> ResolvedPackage {
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
        .map(|(name, mp)| (name.clone(), manifest_package_to_resolved(name, mp)))
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
pub async fn apply_profile_change(
    current: Option<&ProfileGeneration>,
    profile_dir: &Path,
    gc_config: &GcConfig,
    plan: &UpgradePlan,
    activate: bool,
    progress: Option<&dyn UpgradeProgress>,
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

    // 3. Copy store paths from caches, then verify ALL paths exist.
    // Only plan.packages have valid cache_url and are copied. Kept
    // packages from the manifest are expected to already be in the
    // store — verify_store_paths catches any that went missing.
    if let Some(p) = progress {
        p.on_phase("copying");
    }
    store::copy_store_paths(&store::TokioCommandRunner, &plan.packages, None).await?;
    store::verify_store_paths(&store::TokioCommandRunner, &all_packages).await?;

    // 4. Build new profile generation
    if let Some(p) = progress {
        p.on_phase("building");
    }
    let gen_number = profile::next_generation_number(profile_dir)?;
    let generation =
        profile::build_profile(profile_dir, gen_number, &all_packages, "hooks", None).await?;

    // 5. Optionally activate
    if activate {
        if let Some(p) = progress {
            p.on_phase("activating");
        }
        profile::activate_profile(profile_dir, &generation).await?;
    }

    // 6. GC old generations
    gc::cleanup_generations(profile_dir, gc_config)?;

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
            cache_url: Some("https://cache.example.com".into()),
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
        let current = Manifest {
            packages: BTreeMap::new(),
        };
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
    fn manifest_to_resolved_sets_no_cache_url() {
        let mp = test_manifest_package("1.0.0", "my-cache");
        let resolved = manifest_package_to_resolved("test", &mp);
        assert!(resolved.cache_url.is_none());
        assert_eq!(resolved.cache_name, "my-cache");
    }
}
