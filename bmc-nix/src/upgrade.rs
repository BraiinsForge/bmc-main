// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};

use crate::manifest::ComputeUpgradePlanError;
use crate::types::{
    GcConfig, InstallResult, Manifest, MergedIndex, ProfileGeneration, ResolvedPackage,
    StrategySummary, UpgradePlan,
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
    #[error("failed to stage next-boot activation marker: {0}")]
    StageNext(#[source] std::io::Error),
}

/// Activation behavior for a newly built profile generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationMode {
    /// Swap `current` and run the generation's activation entrypoint now.
    Activate,
    /// Build the generation but leave `current` untouched.
    Skip,
    /// Build the generation and stage it as `next.<bos-version>` for the
    /// boot-time activator of that firmware version.
    NextBoot { bos_version: String },
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

impl TryFrom<&str> for UpgradePhase {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "realizing" => Ok(UpgradePhase::Realizing),
            "verifying" => Ok(UpgradePhase::Verifying),
            "building" => Ok(UpgradePhase::Building),
            "activating" => Ok(UpgradePhase::Activating),
            "cleaning" => Ok(UpgradePhase::Cleaning),
            _ => Err(()),
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
/// generation, applies the selected activation mode, and optionally
/// garbage-collects older generations.
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
    store: &impl store::StoreOperations,
    profile_dir: &Path,
    base_manifest: Option<Manifest>,
    merged: Option<&MergedIndex>,
    add_packages: &[ResolvedPackage],
    remove_packages: &[String],
    activation: ActivationMode,
    gc_config: Option<&GcConfig>,
    progress: Option<&dyn UpgradeProgress>,
    hooks_dir: &str,
    hooks_override_path: Option<&Path>,
) -> Result<InstallResult, InstallError> {
    // 1. Acquire profile lock
    let lock = profile::lock_profile(profile_dir)
        .await
        .map_err(InstallError::Lock)?;

    let (base, explicit_base) = resolve_base_manifest(profile_dir, base_manifest)?;

    // Capture the pre-activation `current` generation number, if any, so
    // GC doesn't reclaim it before the orchestration layer (service
    // restart planning, rollback) can read its manifest.
    let previous_generation = previous_generation_number(profile_dir)?;

    let plan = manifest::compute_upgrade_plan(&base, merged, add_packages, remove_packages)?;

    // 3. No-op short-circuit — only applies on the default (None) base
    //    path. Explicit bases always build a new generation.
    if should_skip_rebuild(explicit_base, &plan) {
        remove_stale_next(profile_dir)?;
        let generation = resolve_current_generation(profile_dir)?;
        return Ok(install_result_from_plan(plan, generation, Ok(())));
    }

    let generation = realize_and_build_generation(
        store,
        profile_dir,
        &plan,
        progress,
        hooks_dir,
        hooks_override_path,
    )
    .await?;

    // 6. Apply the requested activation behavior. Stale markers are
    //    swept before activating: a sweep failure aborts the run while
    //    nothing has applied yet, and no crash window is left in which
    //    an already-activated profile still carries a stale marker the
    //    boot activator would promote into a re-application.
    remove_stale_next(profile_dir)?;
    match activation {
        ActivationMode::Activate => {
            activate_generation(profile_dir, &generation, &lock, progress).await?;
        }
        ActivationMode::Skip => {}
        ActivationMode::NextBoot { ref bos_version } => {
            stage_next_boot(profile_dir, &generation, bos_version)?;
        }
    }

    // 7. GC old generations (optional). Protect the pre-activation
    //    generation from this run's cleanup so the caller can still
    //    diff against it; persistent protection lives in `GcConfig`.
    //
    //    The generation is already built and activated or staged, so a
    //    GC failure is captured into the result rather than aborting
    //    the run.
    let gc =
        run_gc_if_configured(store, profile_dir, gc_config, previous_generation, progress).await;

    Ok(install_result_from_plan(plan, Some(generation), gc))
}

fn resolve_base_manifest(
    profile_dir: &Path,
    base_manifest: Option<Manifest>,
) -> Result<(Manifest, bool), InstallError> {
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
    Ok((base, explicit_base))
}

fn previous_generation_number(profile_dir: &Path) -> Result<Option<usize>, InstallError> {
    Ok(profile::current_generation_link(profile_dir)
        .map_err(InstallError::ResolveCurrent)?
        .and_then(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(profile::parse_generation_link_name)
        }))
}

fn should_skip_rebuild(explicit_base: bool, plan: &UpgradePlan) -> bool {
    !explicit_base && plan.added.is_empty() && plan.removed.is_empty() && plan.changed.is_empty()
}

async fn realize_and_build_generation(
    store: &impl store::StoreOperations,
    profile_dir: &Path,
    plan: &UpgradePlan,
    progress: Option<&dyn UpgradeProgress>,
    hooks_dir: &str,
    hooks_override_path: Option<&Path>,
) -> Result<ProfileGeneration, InstallError> {
    if let Some(p) = progress {
        p.on_phase(UpgradePhase::Realizing);
    }
    let realize_progress = progress.map(UpgradeRealizeProgress);
    store
        .realize_store_paths(
            &plan.packages,
            realize_progress
                .as_ref()
                .map(|p| p as &dyn store::RealizeProgress),
        )
        .await?;

    if let Some(p) = progress {
        p.on_phase(UpgradePhase::Verifying);
    }
    store.verify_store_paths(&plan.packages).await?;

    if let Some(p) = progress {
        p.on_phase(UpgradePhase::Building);
    }
    let gen_number = profile::max_generation(profile_dir)?.unwrap_or(0) + 1;
    profile::build_profile(
        profile_dir,
        gen_number,
        &plan.packages,
        hooks_dir,
        hooks_override_path,
    )
    .await
    .map_err(InstallError::from)
}

async fn run_gc_if_configured(
    store: &impl store::StoreOperations,
    profile_dir: &Path,
    gc_config: Option<&GcConfig>,
    previous_generation: Option<usize>,
    progress: Option<&dyn UpgradeProgress>,
) -> Result<(), gc::GcError> {
    let Some(gc_config) = gc_config else {
        return Ok(());
    };
    if let Some(p) = progress {
        p.on_phase(UpgradePhase::Cleaning);
    }
    run_gc(store, profile_dir, gc_config, previous_generation, progress).await
}

fn install_result_from_plan(
    plan: UpgradePlan,
    generation: Option<ProfileGeneration>,
    gc: Result<(), gc::GcError>,
) -> InstallResult {
    InstallResult {
        strategies: StrategySummary::from_packages(&plan.packages),
        generation,
        added: plan.added,
        removed: plan.removed,
        changed: plan.changed,
        stale: plan.stale,
        gc,
    }
}

/// Remove stale deferred-activation markers superseded by this run.
///
/// The removal is a publication too: without the directory fsync an
/// upgrade can report success while a superseded marker survives a
/// crash and downgrades the device at the next boot.
fn remove_stale_next(profile_dir: &Path) -> Result<(), InstallError> {
    crate::activation::sweep_next_markers(profile_dir, None).map_err(InstallError::StageNext)?;
    crate::fs_sync::fsync_dir(profile_dir).map_err(InstallError::StageNext)
}

/// Stage a built generation as `next.<bos-version>` for the boot-time
/// activator of that firmware version, replacing any older marker.
///
/// The generation itself is already durable (build_profile syncs before
/// publishing), and a symlink's only content is its target string, so
/// only the directory fsync is needed to keep a reported staging from
/// evaporating on power loss.
fn stage_next_boot(
    profile_dir: &Path,
    generation: &ProfileGeneration,
    bos_version: &str,
) -> Result<(), InstallError> {
    remove_stale_next(profile_dir)?;
    let link_name = profile::generation_link_name(generation.number);
    let tmp = profile_dir.join(".next.tmp");
    remove_file_if_present(&tmp).map_err(InstallError::StageNext)?;
    std::os::unix::fs::symlink(&link_name, &tmp).map_err(InstallError::StageNext)?;
    std::fs::rename(
        &tmp,
        profile_dir.join(crate::activation::next_marker_name(bos_version)),
    )
    .map_err(InstallError::StageNext)?;
    crate::fs_sync::fsync_dir(profile_dir).map_err(InstallError::StageNext)
}

fn remove_file_if_present(path: &Path) -> Result<(), std::io::Error> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Activate a freshly built generation, reporting the phase.
async fn activate_generation(
    profile_dir: &Path,
    generation: &ProfileGeneration,
    lock: &profile::ProfileLock,
    progress: Option<&dyn UpgradeProgress>,
) -> Result<(), InstallError> {
    if let Some(p) = progress {
        p.on_phase(UpgradePhase::Activating);
    }
    profile::activate_profile(profile_dir, generation.number, &generation.path, Some(lock)).await?;
    Ok(())
}

/// Run the post-activation GC sweep: prune old generation links, then
/// collect now-unreferenced store paths. Cleanup failing short-circuits
/// the heavier collection, since a broken cleanup suggests the profile
/// directory is in a bad state.
async fn run_gc(
    store: &impl store::StoreOperations,
    profile_dir: &Path,
    gc_config: &GcConfig,
    previous_generation: Option<usize>,
    progress: Option<&dyn UpgradeProgress>,
) -> Result<(), gc::GcError> {
    // Protect the pre-activation generation: after activation it is
    // neither `current` nor the latest, so cleanup would otherwise prune
    // it. The freshly built generation is the latest and is already
    // retained by `cleanup_generations`.
    let keep_extra: Vec<usize> = previous_generation.into_iter().collect();
    gc::cleanup_generations(profile_dir, gc_config, &keep_extra)?;
    let gc_progress = progress.map(UpgradeCollectGarbageProgress);
    store
        .collect_garbage(
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
    use std::path::{Path, PathBuf};
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

    /// A [`StoreOperations`] that never touches nix: it records which store
    /// operations the orchestration invoked and returns typed outcomes, so
    /// `apply_profile_change` can be driven end-to-end without a live store.
    #[derive(Debug, Default)]
    struct FakeStore {
        calls: std::sync::Arc<Mutex<Vec<&'static str>>>,
        realize_fails: bool,
    }

    impl FakeStore {
        fn record(&self, op: &'static str) {
            self.calls.lock().expect("BUG: store calls lock").push(op);
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().expect("BUG: store calls lock").clone()
        }
    }

    impl store::StoreOperations for FakeStore {
        fn estimate_realization(
            &self,
            _packages: &[ResolvedPackage],
        ) -> impl std::future::Future<
            Output = Result<store::RealizeEstimate, store::StorePathError>,
        > + Send {
            self.record("estimate");
            std::future::ready(Ok(store::RealizeEstimate::default()))
        }

        fn realize_store_paths(
            &self,
            _packages: &[ResolvedPackage],
            _progress: Option<&dyn store::RealizeProgress>,
        ) -> impl std::future::Future<Output = Result<(), store::StorePathError>> + Send {
            self.record("realize");
            let result = if self.realize_fails {
                use std::os::unix::process::ExitStatusExt as _;
                Err(store::StorePathError::RealiseExited {
                    status: std::process::ExitStatus::from_raw(1 << 8),
                    messages: vec!["fake realize failure".to_owned()],
                })
            } else {
                Ok(())
            };
            std::future::ready(result)
        }

        fn verify_store_paths(
            &self,
            _packages: &[ResolvedPackage],
        ) -> impl std::future::Future<Output = Result<(), store::StorePathError>> + Send {
            self.record("verify");
            std::future::ready(Ok(()))
        }

        fn collect_garbage(
            &self,
            _progress: Option<&dyn gc::CollectGarbageProgress>,
        ) -> impl std::future::Future<Output = Result<(), gc::CollectGarbageError>> + Send {
            self.record("gc");
            std::future::ready(Ok(()))
        }
    }

    fn create_empty_generation(profile_dir: &Path, number: usize) {
        let gen_dir = profile_dir.join(format!("{number}-link"));
        std::fs::create_dir_all(&gen_dir).expect("BUG: mkdir generation");
        std::fs::write(gen_dir.join("manifest"), r#"{"packages":{}}"#)
            .expect("BUG: write manifest");
    }

    fn set_current(profile_dir: &Path, number: usize) {
        let current = profile_dir.join("current");
        let _ = std::fs::remove_file(&current);
        std::os::unix::fs::symlink(format!("{number}-link"), current)
            .expect("BUG: symlink current");
    }

    #[tokio::test]
    async fn apply_profile_change_next_boot_stages_next_symlink_without_touching_current() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");
        create_empty_generation(&profile_dir, 1);
        set_current(&profile_dir, 1);
        // Markers from earlier staging runs, bare and for another
        // firmware version, must be replaced by this run's marker.
        std::os::unix::fs::symlink("1-link", profile_dir.join("next"))
            .expect("BUG: symlink bare next");
        std::os::unix::fs::symlink("1-link", profile_dir.join("next.0.9"))
            .expect("BUG: symlink next.0.9");

        let result = apply_profile_change(
            &FakeStore::default(),
            &profile_dir,
            Some(Manifest::default()),
            None,
            &[],
            &[],
            ActivationMode::NextBoot {
                bos_version: "1.0".to_owned(),
            },
            None,
            None,
            "hooks",
            None,
        )
        .await
        .expect("BUG: staged build must succeed");

        let generation = result
            .generation
            .expect("BUG: a built generation must be reported");
        let target = std::fs::read_link(profile_dir.join("next.1.0"))
            .expect("BUG: next.1.0 symlink must exist");
        assert_eq!(target, PathBuf::from(format!("{}-link", generation.number)));
        assert_eq!(
            std::fs::read_link(profile_dir.join("current")).expect("BUG: current must survive"),
            PathBuf::from("1-link"),
        );
        assert!(profile_dir.join("next").symlink_metadata().is_err());
        assert!(profile_dir.join("next.0.9").symlink_metadata().is_err());
    }

    #[tokio::test]
    async fn failed_run_preserves_previously_staged_next() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");
        create_empty_generation(&profile_dir, 1);
        set_current(&profile_dir, 1);
        std::os::unix::fs::symlink("2-link", profile_dir.join("next"))
            .expect("BUG: symlink staged next");

        let package = ResolvedPackage {
            name: "pkg".into(),
            version: "1.0.0".into(),
            store_path: "/nix/store/00000000000000000000000000000000-pkg".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: crate::types::InstalledBy::System,
            installed_from: "local".into(),
            pinned: None,
        };

        let store = FakeStore {
            realize_fails: true,
            ..Default::default()
        };
        let result = apply_profile_change(
            &store,
            &profile_dir,
            Some(Manifest::default()),
            None,
            &[package],
            &[],
            ActivationMode::Skip,
            None,
            None,
            "hooks",
            None,
        )
        .await;

        assert!(
            matches!(result, Err(InstallError::StorePaths(_))),
            "expected a store-path realization failure, got {result:?}"
        );

        let next = profile_dir.join("next");
        assert_eq!(
            std::fs::read_link(&next).expect("BUG: next must survive a failed run"),
            PathBuf::from("2-link"),
        );
    }

    #[tokio::test]
    async fn apply_profile_change_removes_stale_next_before_noop_short_circuit() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");
        create_empty_generation(&profile_dir, 1);
        set_current(&profile_dir, 1);
        std::os::unix::fs::symlink("99-link", profile_dir.join("next"))
            .expect("BUG: symlink stale next");

        let result = apply_profile_change(
            &FakeStore::default(),
            &profile_dir,
            None,
            None,
            &[],
            &[],
            ActivationMode::Skip,
            None,
            None,
            "hooks",
            None,
        )
        .await
        .expect("BUG: no-op profile change must succeed");

        assert_eq!(
            result
                .generation
                .expect("BUG: current generation must be reported")
                .number,
            1,
        );
        assert!(
            profile_dir.join("next").symlink_metadata().is_err(),
            "stale next must be invalidated even when no new generation is built"
        );
    }

    #[tokio::test]
    async fn apply_profile_change_reports_realizing_and_verifying_phases() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        let collector = PhaseCollector::default();

        // An explicit (empty) base disables the no-op short-circuit, so a
        // generation is built and the realize/verify phases are reported.
        apply_profile_change(
            &FakeStore::default(),
            &profile_dir,
            Some(Manifest::default()),
            None,
            &[],
            &[],
            ActivationMode::Skip,
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

    #[tokio::test]
    async fn apply_profile_change_runs_the_full_store_sequence_without_nix() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");
        create_empty_generation(&profile_dir, 1);
        set_current(&profile_dir, 1);

        // Explicit base builds a generation; GC is enabled so the sweep runs
        // after the build. Every nix step is served by the fake store, so the
        // orchestration is exercised end-to-end with no live store.
        let store = FakeStore::default();
        let result = apply_profile_change(
            &store,
            &profile_dir,
            Some(Manifest::default()),
            None,
            &[],
            &[],
            ActivationMode::Skip,
            Some(&GcConfig::default()),
            None,
            "hooks",
            None,
        )
        .await
        .expect("BUG: profile change should succeed");

        assert_eq!(
            result
                .generation
                .expect("BUG: a built generation must be reported")
                .number,
            2,
        );
        assert!(result.gc.is_ok(), "gc must succeed, got {:?}", result.gc);
        assert_eq!(
            store.calls(),
            vec!["realize", "verify", "gc"],
            "the orchestration must realize, verify, then collect garbage in order"
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
    fn upgrade_phase_str_roundtrip() {
        for p in [
            UpgradePhase::Realizing,
            UpgradePhase::Verifying,
            UpgradePhase::Building,
            UpgradePhase::Activating,
            UpgradePhase::Cleaning,
        ] {
            assert_eq!(UpgradePhase::try_from(p.as_str()), Ok(p));
        }
        assert!(UpgradePhase::try_from("collecting_garbage").is_err());
        assert!(UpgradePhase::try_from("bogus").is_err());
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
