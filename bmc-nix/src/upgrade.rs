// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};

use crate::manifest::PlanConflict;
use crate::types::{InstallResult, Manifest, ProfileGeneration, ResolvedPackage, StrategySummary};
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
    #[error(transparent)]
    Plan(#[from] PlanConflict),
    #[error("failed to resolve current profile symlink: {0}")]
    ResolveCurrent(#[source] std::io::Error),
}

/// Apply an add/remove change to the current profile.
///
/// Acquires the profile lock, reads the current manifest, computes the
/// upgrade plan, verifies store paths, builds a new generation, and
/// optionally activates it.
///
/// When `reset` is true the current manifest is ignored and all
/// `add_packages` are treated as fresh installs (used by reset-profile).
///
/// When the computed plan has no adds, removes, or changes, the rebuild
/// is skipped entirely and the returned [`InstallResult`] points at the
/// existing `current` generation (or `None` if no prior profile exists).
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
    let plan = manifest::compute_upgrade_plan(&base_manifest, add_packages, remove_packages)?;

    // 2a. Short-circuit on no-op: nothing to build.
    if plan.added.is_empty() && plan.removed.is_empty() && plan.changed.is_empty() {
        let generation = resolve_current_generation(profile_dir)?;
        return Ok(InstallResult {
            strategies: StrategySummary::from_packages(&plan.packages),
            generation,
            added: plan.added,
            removed: plan.removed,
            changed: plan.changed,
        });
    }

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
        generation: Some(generation),
        added: plan.added,
        removed: plan.removed,
        changed: plan.changed,
    })
}

/// Resolve `profile_dir/current` into a `ProfileGeneration` when present.
///
/// Returns `Ok(None)` if no `current` symlink exists yet (fresh profile).
/// `ProfileGeneration.number` is reconstructed from the symlink target
/// (`<N>-link`); `manifest` is read from the generation directory.
fn resolve_current_generation(
    profile_dir: &Path,
) -> Result<Option<ProfileGeneration>, InstallError> {
    let current = profile_dir.join("current");
    let target = match std::fs::read_link(&current) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(InstallError::ResolveCurrent(e)),
    };

    let gen_path: PathBuf = if target.is_absolute() {
        target
    } else {
        profile_dir.join(target)
    };

    let number = gen_path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix("-link"))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);

    let manifest = manifest::read_manifest(&gen_path)?;

    Ok(Some(ProfileGeneration {
        number,
        path: gen_path,
        manifest,
    }))
}
