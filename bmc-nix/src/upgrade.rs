// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};

use crate::manifest::ComputeUpgradePlanError;
use crate::types::{
    GcConfig, InstallResult, Manifest, MergedIndex, ProfileGeneration, ResolvedPackage,
    StrategySummary,
};
use crate::{activation, gc, manifest, profile, store};
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
    Plan(#[from] ComputeUpgradePlanError),
    #[error("failed to resolve current profile symlink: {0}")]
    ResolveCurrent(#[source] std::io::Error),
    #[error("`current` symlink target does not follow the `<N>-link` convention: {}", target.display())]
    MalformedCurrent { target: PathBuf },
}

/// Coarse-grained phases reported during an upgrade run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradePhase {
    Realizing,
    Verifying,
    Building,
    Activating,
    Cleaning,
    /// A garbage-collection sub-phase, wrapping the phase `collect_garbage`
    /// reports so it can travel through the single `on_phase` channel during
    /// the broader `Cleaning` step.
    CollectingGarbage(gc::CollectGarbagePhase),
}

impl UpgradePhase {
    /// Stable lowercase name used in progress output. The `match` is
    /// exhaustive with no wildcard so a new variant fails to compile
    /// until it is given a name here.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            UpgradePhase::Realizing => "realizing",
            UpgradePhase::Verifying => "verifying",
            UpgradePhase::Building => "building",
            UpgradePhase::Activating => "activating",
            UpgradePhase::Cleaning => "cleaning",
            UpgradePhase::CollectingGarbage(phase) => phase.as_str(),
        }
    }
}

/// Progress callback for upgrade phases and store-path realization.
pub trait UpgradeProgress: Send + Sync {
    fn on_phase(&self, phase: UpgradePhase);
    fn on_realization_started(&self, total_paths: usize);
    fn on_realization_finished(&self);
    fn on_download_status(&self, snapshot: &store::progress::DownloadSnapshot);
    /// Running count of store paths deleted during garbage collection.
    fn on_gc_deleted(&self, deleted_paths: usize);
    /// Final garbage-collection tally; `freed_bytes` is `None` when nix's
    /// summary line could not be parsed.
    fn on_gc_finished(&self, deleted_paths: usize, freed_bytes: Option<u64>);
}

/// Adapter exposing an [`UpgradeProgress`] as a [`store::RealizeProgress`]
/// so realization can report through the same sink as the upgrade phases.
struct UpgradeRealizeProgress<'a>(&'a dyn UpgradeProgress);

impl store::RealizeProgress for UpgradeRealizeProgress<'_> {
    fn on_realization_started(&self, total_paths: usize) {
        self.0.on_realization_started(total_paths);
    }

    fn on_realization_finished(&self) {
        self.0.on_realization_finished();
    }

    fn on_download_status(&self, snapshot: &store::progress::DownloadSnapshot) {
        self.0.on_download_status(snapshot);
    }
}

/// Adapter exposing an [`UpgradeProgress`] as a [`gc::CollectGarbageProgress`]
/// so the post-activation GC sweep reports through the same sink as the
/// upgrade phases. GC phases are wrapped into [`UpgradePhase::CollectingGarbage`];
/// deletion progress and the final tally map onto the trait's GC methods.
struct UpgradeCollectGarbageProgress<'a>(&'a dyn UpgradeProgress);

impl gc::CollectGarbageProgress for UpgradeCollectGarbageProgress<'_> {
    fn on_phase(&self, phase: gc::CollectGarbagePhase) {
        self.0.on_phase(UpgradePhase::CollectingGarbage(phase));
    }

    fn on_deleted(&self, deleted_paths: usize) {
        self.0.on_gc_deleted(deleted_paths);
    }

    fn on_finished(&self, deleted_paths: usize, freed_bytes: Option<u64>) {
        self.0.on_gc_finished(deleted_paths, freed_bytes);
    }
}

/// Apply an add/remove change to a profile.
///
/// Acquires the profile lock, resolves the base manifest, computes
/// the upgrade plan, realises and verifies store paths, builds a new
/// generation, optionally activates it, and optionally garbage-collects
/// older generations.
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
/// `merged`:
/// - `None` — kept packages are carried at their current version
///   (offline path; the result is the manifest plus any explicit
///   add/remove changes).
/// - `Some(&MergedIndex)` — kept packages are resolved through the
///   merged index, so a newer version satisfying the package's pin
///   strategy becomes a `changed` entry; a package missing from the
///   index is reported as `stale` and carried at its current version.
///
/// The no-op short-circuit (empty plan → skip rebuild, return the
/// resolved current generation) applies ONLY when `base_manifest`
/// is `None`. With an explicit base, a new generation is always
/// built even if the plan is empty against that base.
#[expect(
    clippy::too_many_arguments,
    reason = "orchestration entrypoint - every parameter is required"
)]
pub async fn apply_profile_change(
    profile_dir: &Path,
    base_manifest: Option<Manifest>,
    merged: Option<&MergedIndex>,
    add_packages: &[ResolvedPackage],
    remove_packages: &[String],
    activate: bool,
    gc_config: Option<&GcConfig>,
    progress: Option<&dyn UpgradeProgress>,
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

    // Capture the pre-activation `current` generation number, if any, so
    // GC doesn't reclaim it before the orchestration layer (service
    // restart planning, rollback) can read its manifest.
    let previous_generation: Option<usize> = profile::current_generation_link(profile_dir)
        .map_err(InstallError::ResolveCurrent)?
        .and_then(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(profile::parse_generation_link_name)
        });

    let plan = manifest::compute_upgrade_plan(&base, merged, add_packages, remove_packages)?;

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
            stale: plan.stale,
            gc: Ok(()),
        });
    }

    // 4. Realise store paths, then verify as defense-in-depth.
    if let Some(p) = progress {
        p.on_phase(UpgradePhase::Realizing);
    }
    let realize_progress = progress.map(UpgradeRealizeProgress);
    store::realize_store_paths(
        &store::TokioCommandRunner,
        &plan.packages,
        realize_progress
            .as_ref()
            .map(|p| p as &dyn store::RealizeProgress),
    )
    .await?;
    if let Some(p) = progress {
        p.on_phase(UpgradePhase::Verifying);
    }
    store::verify_store_paths(&store::TokioCommandRunner, &plan.packages).await?;

    // 5. Build new profile generation
    if let Some(p) = progress {
        p.on_phase(UpgradePhase::Building);
    }
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
        if let Some(p) = progress {
            p.on_phase(UpgradePhase::Activating);
        }
        profile::activate_profile(
            profile_dir,
            generation.number,
            &generation.path,
            Some(&lock),
        )
        .await?;
    }

    // 7. GC old generations (optional). Protect the pre-activation
    //    generation from this run's cleanup so the caller can still
    //    diff against it; persistent protection lives in `GcConfig`.
    //
    //    The generation is already built and activated, so a GC failure
    //    is captured into the result rather than aborting the run.
    let gc = match gc_config {
        Some(gc_config) => {
            if let Some(p) = progress {
                p.on_phase(UpgradePhase::Cleaning);
            }
            let keep_extra: Vec<usize> = previous_generation.into_iter().collect();
            run_gc(profile_dir, gc_config, &keep_extra, progress).await
        }
        None => Ok(()),
    };

    Ok(InstallResult {
        strategies: StrategySummary::from_packages(&plan.packages),
        generation: Some(generation),
        added: plan.added,
        removed: plan.removed,
        changed: plan.changed,
        stale: plan.stale,
        gc,
    })
}

/// Run the post-activation GC sweep: prune old generation links, then
/// collect now-unreferenced store paths. Cleanup failing short-circuits
/// the heavier collection, since a broken cleanup suggests the profile
/// directory is in a bad state.
async fn run_gc(
    profile_dir: &Path,
    gc_config: &GcConfig,
    keep_extra: &[usize],
    progress: Option<&dyn UpgradeProgress>,
) -> Result<(), gc::GcError> {
    gc::cleanup_generations(profile_dir, gc_config, keep_extra)?;
    let gc_progress = progress.map(UpgradeCollectGarbageProgress);
    gc::collect_garbage(
        &store::TokioCommandRunner,
        gc_progress
            .as_ref()
            .map(|p| p as &dyn gc::CollectGarbageProgress),
    )
    .await?;
    Ok(())
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
    use std::sync::Mutex;

    use crate::gc;
    use crate::types::GcConfig;

    /// Records the phases reported through [`UpgradeProgress`].
    #[derive(Default)]
    struct PhaseCollector {
        phases: Mutex<Vec<UpgradePhase>>,
    }

    impl PhaseCollector {
        fn phases(&self) -> Vec<UpgradePhase> {
            self.phases
                .lock()
                .expect("BUG: phase lock poisoned")
                .clone()
        }
    }

    impl UpgradeProgress for PhaseCollector {
        fn on_phase(&self, phase: UpgradePhase) {
            self.phases
                .lock()
                .expect("BUG: phase lock poisoned")
                .push(phase);
        }

        fn on_realization_started(&self, _total_paths: usize) {}
        fn on_realization_finished(&self) {}
        fn on_download_status(&self, _snapshot: &store::progress::DownloadSnapshot) {}
        fn on_gc_deleted(&self, _deleted_paths: usize) {}
        fn on_gc_finished(&self, _deleted_paths: usize, _freed_bytes: Option<u64>) {}
    }

    /// Records every event reached through [`UpgradeProgress`] as a string,
    /// so a test can assert the GC adapter routes to the right methods.
    #[derive(Default)]
    struct EventRecorder {
        events: Mutex<Vec<String>>,
    }

    impl UpgradeProgress for EventRecorder {
        fn on_phase(&self, phase: UpgradePhase) {
            self.push(format!("phase:{}", phase.as_str()));
        }
        fn on_realization_started(&self, _total_paths: usize) {}
        fn on_realization_finished(&self) {}
        fn on_download_status(&self, _snapshot: &store::progress::DownloadSnapshot) {}
        fn on_gc_deleted(&self, deleted_paths: usize) {
            self.push(format!("gc_deleted:{deleted_paths}"));
        }
        fn on_gc_finished(&self, deleted_paths: usize, freed_bytes: Option<u64>) {
            self.push(format!("gc_finished:{deleted_paths}:{freed_bytes:?}"));
        }
    }

    impl EventRecorder {
        fn push(&self, event: String) {
            self.events
                .lock()
                .expect("BUG: event lock poisoned")
                .push(event);
        }
    }

    #[tokio::test]
    async fn apply_profile_change_reports_realizing_and_verifying_phases() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        let collector = PhaseCollector::default();

        // An explicit (empty) base disables the no-op short-circuit, so a
        // generation is built and the realize/verify phases are reported.
        // An empty package set keeps the store steps as in-process no-ops,
        // so the test does not depend on a live Nix store.
        apply_profile_change(
            &profile_dir,
            Some(Manifest::default()),
            None,
            &[],
            &[],
            /* activate = */ false,
            None,
            Some(&collector),
            "hooks",
            None,
        )
        .await
        .expect("BUG: profile change should succeed");

        let phases = collector.phases();
        assert!(
            phases.len() >= 2,
            "expected at least the realize and verify phases, got {phases:?}"
        );
        assert_eq!(
            &phases[..2],
            &[UpgradePhase::Realizing, UpgradePhase::Verifying],
            "phases must begin with Realizing then Verifying, got {phases:?}"
        );
        assert!(
            phases.contains(&UpgradePhase::Building),
            "build phase must follow verification, got {phases:?}"
        );
    }

    #[test]
    fn upgrade_phase_as_str_covers_every_variant() {
        assert_eq!(UpgradePhase::Realizing.as_str(), "realizing");
        assert_eq!(UpgradePhase::Verifying.as_str(), "verifying");
        assert_eq!(UpgradePhase::Building.as_str(), "building");
        assert_eq!(UpgradePhase::Activating.as_str(), "activating");
        assert_eq!(UpgradePhase::Cleaning.as_str(), "cleaning");
        assert_eq!(
            UpgradePhase::CollectingGarbage(gc::CollectGarbagePhase::FindingRoots).as_str(),
            "finding_roots"
        );
        assert_eq!(
            UpgradePhase::CollectingGarbage(gc::CollectGarbagePhase::DeterminingLiveness).as_str(),
            "determining_liveness"
        );
    }

    #[test]
    fn collect_garbage_adapter_routes_events_to_upgrade_sink() {
        use gc::CollectGarbageProgress as _;

        let recorder = EventRecorder::default();
        let adapter = UpgradeCollectGarbageProgress(&recorder);

        adapter.on_phase(gc::CollectGarbagePhase::FindingRoots);
        adapter.on_deleted(42);
        adapter.on_finished(7, Some(1536));

        assert_eq!(
            recorder
                .events
                .lock()
                .expect("BUG: event lock poisoned")
                .as_slice(),
            &[
                "phase:finding_roots".to_owned(),
                "gc_deleted:42".to_owned(),
                "gc_finished:7:Some(1536)".to_owned(),
            ],
        );
    }

    #[test]
    fn cleanup_generations_keeps_previous_current_generation_after_activation() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        // Simulate generations 1, 2, 3 with current pointing at the latest
        // (3), as it would after an activation whose pre-activation current
        // was 2.
        for n in 1..=3 {
            let gen_dir = profile_dir.join(format!("{n}-link"));
            std::fs::create_dir_all(&gen_dir).expect("BUG: mkdir gen");
            std::fs::write(gen_dir.join("manifest"), r#"{"packages":{}}"#)
                .expect("BUG: write manifest");
        }
        let current_link = profile_dir.join("current");
        std::os::unix::fs::symlink("3-link", &current_link).expect("BUG: symlink current");

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            min_free_space: "0".into(),
            protected_generations: vec![],
        };
        // `keep_extra = [2]` is the pre-activation current that the
        // orchestration layer still needs after activation.
        gc::cleanup_generations(&profile_dir, &gc_config, &[2]).expect("BUG: cleanup failed");

        assert!(
            !profile_dir.join("1-link").exists(),
            "gen 1 is unprotected and outside the keep window"
        );
        assert!(
            profile_dir.join("2-link").exists(),
            "previous current (gen 2) must be preserved via keep_extra"
        );
        assert!(
            profile_dir.join("3-link").exists(),
            "current/latest gen 3 must be preserved"
        );
    }

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
