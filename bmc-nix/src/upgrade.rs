// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};

use crate::manifest::PlanConflict;
use crate::types::{InstallResult, Manifest, ProfileGeneration, ResolvedPackage, StrategySummary};
use crate::{activation, manifest, profile, store};
use tracing::warn;

/// Errors that can occur during an install/upgrade operation.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("profile lock failed: {0}")]
    Lock(#[source] profile::BuildProfileError),
    #[error("failed to read current manifest: {0}")]
    ReadManifest(#[from] manifest::ReadManifestError),
    #[error(transparent)]
    StorePaths(#[from] store::StorePathError),
    #[error(transparent)]
    BuildProfile(#[from] profile::BuildProfileError),
    #[error("activation failed: {0}")]
    Activation(#[from] activation::ActivationError),
    #[error(transparent)]
    Plan(#[from] PlanConflict),
    #[error("failed to resolve current profile symlink: {0}")]
    ResolveCurrent(#[source] std::io::Error),
    #[error("`current` symlink target does not follow the `<N>-link` convention: {}", target.display())]
    MalformedCurrent { target: PathBuf },
}

/// Apply an add/remove change to a profile.
///
/// Acquires the profile lock, resolves the base manifest, computes
/// the upgrade plan, verifies store paths, builds a new generation,
/// and optionally activates it.
///
/// `base_manifest`:
/// - `None` — default path: read the current manifest under the
///   lock via [`manifest::read_current_manifest`]. If the `current`
///   symlink is missing ([`manifest::ReadManifestError::CurrentNotFound`]),
///   log a warning and fall back to [`manifest::read_latest_manifest`]
///   so a broken-symlink profile is not silently treated as empty.
/// - `Some(m)` — use `m` directly (resolved by the caller, possibly
///   before taking the lock). This is the path used by explicit
///   `--base` selections and by `reset-profile` (which passes an
///   empty manifest).
///
/// The no-op short-circuit (empty plan → skip rebuild, return the
/// resolved current generation) applies ONLY when `base_manifest`
/// is `None`. With an explicit base, a new generation is always
/// built even if the plan is empty against that base.
pub async fn apply_profile_change(
    profile_dir: &Path,
    base_manifest: Option<Manifest>,
    add_packages: &[ResolvedPackage],
    remove_packages: &[String],
    activate: bool,
    hooks_dir: &str,
    hooks_override_path: Option<&Path>,
) -> Result<InstallResult, InstallError> {
    // 1. Acquire profile lock
    let lock = profile::lock_profile(profile_dir)
        .await
        .map_err(InstallError::Lock)?;

    // 2. Resolve the base manifest. Track whether the caller passed
    //    one so we can scope the no-op short-circuit correctly.
    let explicit_base = base_manifest.is_some();
    let base = match base_manifest {
        Some(m) => m,
        None => match manifest::read_current_manifest(profile_dir) {
            Ok(m) => m,
            Err(manifest::ReadManifestError::CurrentNotFound { path }) => {
                warn!(
                    %path,
                    "`current` symlink missing; falling back to latest generation"
                );
                manifest::read_latest_manifest(profile_dir)?
            }
            Err(other) => return Err(other.into()),
        },
    };

    let plan = manifest::compute_upgrade_plan(&base, add_packages, remove_packages)?;

    // 3. No-op short-circuit — only applies on the default (None) base
    //    path. Explicit bases always build a new generation.
    if !explicit_base && plan.added.is_empty() && plan.removed.is_empty() && plan.changed.is_empty()
    {
        let generation = resolve_current_generation(profile_dir)?;
        return Ok(InstallResult {
            strategies: StrategySummary::from_packages(&plan.packages),
            generation,
            added: plan.added,
            removed: plan.removed,
            changed: plan.changed,
        });
    }

    // 4. Realise store paths, then verify as defense-in-depth
    store::realize_store_paths(&store::TokioCommandRunner, &plan.packages, None).await?;
    store::verify_store_paths(&store::TokioCommandRunner, &plan.packages).await?;

    // 5. Build new profile generation
    let gen_number = profile::max_generation(profile_dir)?.unwrap_or(0) + 1;
    let generation = profile::build_profile(
        profile_dir,
        gen_number,
        &plan.packages,
        hooks_dir,
        hooks_override_path,
    )
    .await?;

    // 6. Optionally activate
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
/// Returns [`InstallError::MalformedCurrent`] when the target does not
/// follow the `<N>-link` convention.
fn resolve_current_generation(
    profile_dir: &Path,
) -> Result<Option<ProfileGeneration>, InstallError> {
    let Some(gen_path) =
        profile::current_generation_link(profile_dir).map_err(InstallError::ResolveCurrent)?
    else {
        return Ok(None);
    };

    let number = gen_path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(profile::parse_generation_link_name)
        .ok_or_else(|| InstallError::MalformedCurrent {
            target: gen_path.clone(),
        })?;

    let manifest = manifest::read_manifest(&gen_path)?;

    Ok(Some(ProfileGeneration {
        number,
        path: gen_path,
        manifest,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_current_generation_rejects_malformed_target() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        // Target that does NOT follow the `<N>-link` convention.
        let bogus_target = profile_dir.join("42-nope");
        std::fs::create_dir_all(&bogus_target).expect("BUG: mkdir target");
        std::os::unix::fs::symlink("42-nope", profile_dir.join("current"))
            .expect("BUG: symlink current");

        let result = resolve_current_generation(&profile_dir);

        assert!(
            matches!(result, Err(InstallError::MalformedCurrent { .. })),
            "expected MalformedCurrent, got {result:?}"
        );
    }
}
