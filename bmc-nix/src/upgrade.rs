// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::Path;

use crate::types::{InstallResult, Manifest, ResolvedPackage, StrategySummary};
use crate::{activation, manifest, profile, store};

/// Errors that can occur during an install/upgrade operation.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("profile lock failed: {0}")]
    Lock(#[source] profile::BuildProfileError),
    #[error("failed to read current manifest: {0}")]
    ReadManifest(#[from] manifest::ReadManifestError),
    #[error(transparent)]
    CopyStorePaths(#[from] store::CopyStorePathsError),
    #[error(transparent)]
    BuildProfile(#[from] profile::BuildProfileError),
    #[error("activation failed: {0}")]
    Activation(#[from] activation::ActivationError),
}

/// Apply an add/remove change to the current profile.
///
/// Acquires the profile lock, reads the current manifest, computes the
/// upgrade plan, verifies store paths, builds a new generation, and
/// optionally activates it.
///
/// When `reset` is true the current manifest is ignored and all
/// `add_packages` are treated as fresh installs (used by reset-profile).
pub async fn apply_profile_change(
    profile_dir: &Path,
    add_packages: &[ResolvedPackage],
    remove_packages: &[String],
    reset: bool,
    activate: bool,
    hooks_dir: &str,
    hooks_override_path: Option<&Path>,
) -> Result<InstallResult, InstallError> {
    // 1. Acquire profile lock
    let lock = profile::lock_profile(profile_dir)
        .await
        .map_err(InstallError::Lock)?;

    // 2. Read manifest under lock (or use empty for reset)
    let base_manifest = if reset {
        Manifest::default()
    } else {
        manifest::read_current_manifest(profile_dir)?
    };
    let plan = manifest::compute_upgrade_plan(&base_manifest, add_packages, remove_packages);

    // 3. Verify all store paths exist
    store::verify_store_paths(&store::TokioCommandRunner, &plan.packages).await?;

    // 4. Build new profile generation
    let gen_number = profile::next_generation_number(profile_dir)?;
    let generation = profile::build_profile(
        profile_dir,
        gen_number,
        &plan.packages,
        hooks_dir,
        hooks_override_path,
    )
    .await?;

    // 5. Optionally activate
    if activate {
        profile::activate_profile(
            profile_dir,
            generation.number,
            &generation.path,
            Some(&lock),
        )
        .await?;
    }

    Ok(InstallResult {
        strategies: StrategySummary::from_packages(&plan.packages),
        generation,
    })
}
