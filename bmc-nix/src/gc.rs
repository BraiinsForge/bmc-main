// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tracing::{info, warn};

use crate::store::{CommandRunner, StoreOperations};
use crate::types::GcConfig;

/// Marker file written by the superseded interval-driven design. Nothing
/// writes it any more; cleanup removes it wherever it is still found.
pub const LAST_GC_MARKER: &str = ".last-gc";

#[derive(Debug, thiserror::Error)]
pub enum LoadGcConfigError {
    #[error("failed to read gc config at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse gc config at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub fn load_gc_config(path: &Path) -> Result<GcConfig, LoadGcConfigError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GcConfig::default());
        }
        Err(source) => {
            return Err(LoadGcConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    serde_json::from_str(&contents).map_err(|source| LoadGcConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// What to do when another writer holds the profile lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnBusy {
    /// Block until the lock is free. For callers that must collect.
    Wait,
    /// Give up and report [`ProfileGcOutcome::Busy`].
    Skip,
}

/// When to run the expensive `nix-collect-garbage` sweep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sweep {
    Always,
    /// Only when generation cleanup unrooted something, so an idle device
    /// does not scan the whole store for nothing.
    WhenGenerationsRemoved,
}

/// The two independent decisions collection callers differ on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GcRequest {
    /// What to do when another writer holds the profile lock.
    pub on_busy: OnBusy,
    /// When to run the store sweep.
    pub sweep: Sweep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileGcOutcome {
    Collected,
    /// The sweep completed but cleanup failed on some entries: what was
    /// unrooted is reclaimed, and the failed entries stay for a later retry.
    SweptDespiteCleanupFailure,
    /// Cleanup unrooted nothing and the request did not force a sweep.
    NothingToCollect,
    /// Another writer holds the profile lock.
    Busy,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileGcError {
    #[error("failed to lock the profile: {0}")]
    Lock(#[source] crate::profile::BuildProfileError),
    #[error(transparent)]
    Cleanup(#[from] CleanupGenerationsError),
    #[error("store collection failed after removing {removed} profile entries: {source}")]
    Sweep {
        removed: usize,
        #[source]
        source: CollectGarbageError,
    },
}

impl ProfileGcError {
    /// Entries this run removed that no completed sweep accounted for.
    ///
    /// Nonzero means store garbage is left that no later cleanup can
    /// rediscover — the generations rooting it are already gone — so the next
    /// run must sweep unconditionally.
    #[must_use]
    pub fn unswept_removals(&self) -> usize {
        match self {
            Self::Lock(_) => 0,
            Self::Cleanup(err) => err.removed(),
            Self::Sweep { removed, .. } => *removed,
        }
    }
}

/// Clean up profile generations and, depending on `request`, sweep the store.
///
/// Holds the profile lock across both steps: no build, staging, or
/// activation can be mid-swap while entries are removed, and the sweep
/// cannot delete a concurrent upgrade's realized but not-yet-rooted
/// store paths.
pub async fn collect_profile_garbage(
    store: &impl StoreOperations,
    profile_dir: &Path,
    config: &GcConfig,
    request: GcRequest,
    progress: Option<&dyn CollectGarbageProgress>,
) -> Result<ProfileGcOutcome, ProfileGcError> {
    let _lock = match request.on_busy {
        OnBusy::Skip => {
            let Some(lock) =
                crate::profile::try_lock_profile(profile_dir).map_err(ProfileGcError::Lock)?
            else {
                return Ok(ProfileGcOutcome::Busy);
            };
            lock
        }
        OnBusy::Wait => crate::profile::lock_profile(profile_dir)
            .await
            .map_err(ProfileGcError::Lock)?,
    };

    // A failed cleanup does not abort collection: the generations it removed
    // are already unrooted, and only a sweep can reclaim what they rooted.
    // Entries that resisted removal still root their own paths,
    // so sweeping past them is safe; they stay for the next run to retry.
    let (removed, cleanup_failure) = {
        let profile_dir = profile_dir.to_path_buf();
        let config = config.clone();
        tokio::task::spawn_blocking(
            move || match cleanup_generations(&profile_dir, &config, &[]) {
                Ok(removed) => (removed, None),
                Err(err) => (err.removed(), Some(err)),
            },
        )
        .await
        .expect("BUG: cleanup task should not panic")
    };

    if matches!(request.sweep, Sweep::WhenGenerationsRemoved) && removed == 0 {
        return match cleanup_failure {
            Some(err) => Err(err.into()),
            None => Ok(ProfileGcOutcome::NothingToCollect),
        };
    }

    if let Some(err) = &cleanup_failure {
        warn!(error = %err, removed, "generation cleanup failed; sweeping what it unrooted");
    }

    store
        .collect_garbage(progress)
        .await
        .map_err(|source| ProfileGcError::Sweep { removed, source })?;

    Ok(match cleanup_failure {
        Some(_) => ProfileGcOutcome::SweptDespiteCleanupFailure,
        None => ProfileGcOutcome::Collected,
    })
}

/// Errors that can occur when cleaning up generations.
#[derive(Debug, thiserror::Error)]
pub enum CleanupGenerationsError {
    #[error("generation cleanup failed: {0}")]
    Cleanup(#[source] std::io::Error),
    #[error(
        "failed to remove {failed} of {attempted} profile entries; \
         first failure was {first_entry}: {first_error}"
    )]
    RemovalFailures {
        /// Entries removed before the run gave up. They are already gone, so
        /// no later cleanup can rediscover them as a reason to sweep.
        removed: usize,
        attempted: usize,
        failed: usize,
        first_entry: String,
        #[source]
        first_error: std::io::Error,
    },
}

impl CleanupGenerationsError {
    /// Entries this failed cleanup nonetheless removed.
    #[must_use]
    pub fn removed(&self) -> usize {
        match self {
            // Enumeration and metadata failures precede every removal.
            Self::Cleanup(_) => 0,
            Self::RemovalFailures { removed, .. } => *removed,
        }
    }
}

/// Errors that can occur when running `nix-collect-garbage`.
#[derive(Debug, thiserror::Error)]
pub enum CollectGarbageError {
    #[error("nix-collect-garbage failed: {0}")]
    NixCommand(#[source] std::io::Error),
}

/// Failure of the post-activation GC sweep, unifying both GC steps.
///
/// The new generation is already built and activated before GC runs, so
/// these errors are reported to the operator rather than failing the
/// upgrade.
#[derive(Debug, thiserror::Error)]
pub enum GcError {
    #[error(transparent)]
    Cleanup(#[from] CleanupGenerationsError),
    #[error(transparent)]
    Collect(#[from] CollectGarbageError),
}

/// Coarse phase of a `nix-collect-garbage` run, parsed from its output.
///
/// nix exposes no structured progress for garbage collection (no activity
/// in the `internal-json` protocol), so these are recovered from its
/// plain-text status lines. The two phases precede any deletion; the
/// liveness trace is the long, silent step before paths start being freed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectGarbagePhase {
    /// Enumerating garbage-collector roots.
    FindingRoots,
    /// Tracing reachability to separate live paths from dead ones.
    DeterminingLiveness,
}

impl CollectGarbagePhase {
    /// Stable machine-readable name for the `internal-json` progress format.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FindingRoots => "finding_roots",
            Self::DeterminingLiveness => "determining_liveness",
        }
    }
}

impl TryFrom<&str> for CollectGarbagePhase {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "finding_roots" => Ok(CollectGarbagePhase::FindingRoots),
            "determining_liveness" => Ok(CollectGarbagePhase::DeterminingLiveness),
            _ => Err(()),
        }
    }
}

/// Progress callback for a `nix-collect-garbage` sweep.
///
/// nix does not stream the number of bytes freed; the only live signal is
/// the count of deleted store paths, with the freed-bytes total available
/// once at the end. `on_deleted` therefore reports a running path count,
/// and `on_finished` carries nix's final tally.
pub trait CollectGarbageProgress: Send + Sync {
    /// A coarse phase change (roots, liveness trace).
    fn on_phase(&self, phase: CollectGarbagePhase);
    /// Running count of store paths deleted so far.
    fn on_deleted(&self, deleted_paths: usize);
    /// Final tally once the sweep completes. `freed_bytes` is `None` when
    /// the summary line could not be parsed.
    fn on_finished(&self, deleted_paths: usize, freed_bytes: Option<u64>);
}

/// A single meaningful line parsed from `nix-collect-garbage` output.
enum GcOutputLine {
    Phase(CollectGarbagePhase),
    PathDeleted,
    Summary {
        deleted_paths: usize,
        freed_bytes: Option<u64>,
    },
}

/// Classify one line of `nix-collect-garbage` plain output, or `None` when
/// the line carries no progress signal.
fn parse_gc_line(line: &str) -> Option<GcOutputLine> {
    let line = line.trim();
    if line.starts_with("finding garbage collector roots") {
        return Some(GcOutputLine::Phase(CollectGarbagePhase::FindingRoots));
    }
    if line.starts_with("determining live/dead paths") {
        return Some(GcOutputLine::Phase(
            CollectGarbagePhase::DeterminingLiveness,
        ));
    }
    // Per-path deletions are quoted (`deleting '/nix/store/…'`); the
    // unquoted "deleting unused links..." line is not a path deletion.
    if line.starts_with("deleting '") {
        return Some(GcOutputLine::PathDeleted);
    }
    parse_gc_summary(line)
}

/// Parse the final summary line, e.g. `1 store paths deleted, 0.00 MiB freed`.
fn parse_gc_summary(line: &str) -> Option<GcOutputLine> {
    let (count, rest) = line.split_once(" store paths deleted")?;
    let deleted_paths: usize = count.trim().parse().ok()?;
    let freed_bytes = rest
        .trim()
        .strip_prefix(',')
        .map(str::trim)
        .and_then(|s| s.strip_suffix("freed"))
        .map(str::trim)
        .and_then(parse_binary_size);
    Some(GcOutputLine::Summary {
        deleted_paths,
        freed_bytes,
    })
}

/// Parse a binary-unit size as nix prints it (`12.34 MiB`) into bytes.
/// Returns `None` for an unrecognised unit or a malformed number.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "freed size is for display only; sub-byte precision and the \
              theoretical >u64 overflow are irrelevant"
)]
fn parse_binary_size(s: &str) -> Option<u64> {
    let (value, unit) = s.split_once(' ')?;
    let value: f64 = value.parse().ok()?;
    if value < 0.0 {
        return None;
    }
    let multiplier: f64 = match unit {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((value * multiplier) as u64)
}

/// Leftover temp entry in a profile directory: any `*.tmp`. The
/// atomic-swap idioms stage `<N>-link.tmp` (build), `.next.tmp` (staging),
/// and `current.tmp` (activation repoint), then rename them into place; a
/// crash between create and rename orphans the temp, and because it sits
/// under the gcroots profile directory every symlink inside pins its store
/// path until the entry is removed. The sweep holds the profile lock, so
/// any `*.tmp` present is necessarily orphaned — an in-flight swap would
/// hold the lock itself.
fn is_leftover_tmp_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
}

/// Remove a profile entry, which may be a directory tree (`<N>-link`,
/// `<N>-link.tmp`) or something else — a bare symlink (`.next.tmp`)
/// or a stray regular file squatting on a generation name.
fn remove_profile_entry(path: &Path) -> Result<(), std::io::Error> {
    if std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Generation number named by a `<N>-link` symlink target, if any.
fn generation_number_of(target: &Path) -> Option<usize> {
    let name = target.file_name()?.to_str()?;
    crate::profile::parse_generation_link_name(name)
}

/// Read the `current` symlink in `profile_dir` and return the generation
/// number it points to.
///
/// `Ok(None)` when `current` is absent or does not name a `<N>-link`. A
/// read failure other than absence is propagated so gc aborts rather than
/// silently treating the protected generation as unprotected.
fn current_generation_number(profile_dir: &Path) -> Result<Option<usize>, std::io::Error> {
    Ok(
        crate::profile::current_generation_link(profile_dir)?
            .and_then(|t| generation_number_of(&t)),
    )
}

/// Generation numbers referenced by deferred-activation markers
/// (`next.<bos-version>`, or a bare `next`) in `profile_dir`, with the
/// same error semantics as [`current_generation_number`]. Markers for
/// firmware versions other than the running one stay protected until an
/// activator or staging run sweeps them.
fn next_generation_numbers(profile_dir: &Path) -> Result<Vec<usize>, std::io::Error> {
    let entries = match std::fs::read_dir(profile_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut numbers = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if !crate::activation::is_next_marker_name(name_str) {
            continue;
        }
        match std::fs::read_link(entry.path()) {
            Ok(target) => numbers.extend(generation_number_of(&target)),
            // A marker-named entry that is not a symlink yields EINVAL
            // (`InvalidInput`) from `read_link`. Treat it like an absent
            // marker (`NotFound`) — a tolerated stray to skip, not a reason
            // to abort every gc run. `sweep_next_markers` guards the same
            // case with an `is_symlink` check.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidInput
                ) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(numbers)
}

/// Drop the marker file left behind by the superseded interval-driven design.
///
/// It roots nothing, so failing to remove it is not a reason to fail the
/// cleanup that found it.
fn remove_stale_gc_marker(profile_dir: &Path) {
    let path = profile_dir.join(LAST_GC_MARKER);
    info!(path = %path.display(), "removing stale gc marker");
    if let Err(e) = std::fs::remove_file(&path) {
        warn!(path = %path.display(), "failed to remove stale gc marker: {e}");
    }
}

/// Generation numbers [`cleanup_generations`] must not remove, per the
/// retention rules documented there. `generations` is sorted ascending.
fn generations_to_keep(
    profile_dir: &Path,
    gc_config: &GcConfig,
    keep_extra: &[usize],
    generations: &[usize],
) -> Result<HashSet<usize>, CleanupGenerationsError> {
    let mut keep: HashSet<usize> = HashSet::new();

    if let Some(current) =
        current_generation_number(profile_dir).map_err(CleanupGenerationsError::Cleanup)?
    {
        keep.insert(current);
    }

    keep.extend(next_generation_numbers(profile_dir).map_err(CleanupGenerationsError::Cleanup)?);

    if let Some(&latest) = generations.last() {
        keep.insert(latest);
    }

    for &gen_num in &gc_config.protected_generations {
        keep.insert(gen_num);
    }

    for &gen_num in keep_extra {
        keep.insert(gen_num);
    }

    for &gen_num in generations.iter().rev().take(gc_config.keep_generations) {
        keep.insert(gen_num);
    }

    if let Some(days) = gc_config.keep_days {
        let cutoff =
            SystemTime::now() - std::time::Duration::from_secs((days as u64) * 24 * 60 * 60);

        for &gen_num in generations {
            let gen_dir = profile_dir.join(crate::profile::generation_link_name(gen_num));
            match std::fs::metadata(&gen_dir).and_then(|meta| meta.modified()) {
                Ok(mtime) => {
                    if mtime > cutoff {
                        keep.insert(gen_num);
                    }
                }
                // Failing to date one entry must not fail the whole sweep,
                // and a generation is always a directory. Only an entry
                // proven not to be one — a stray dangling link squatting on
                // a generation name — forfeits its age protection here;
                // anything undatable for any other reason keeps it.
                Err(e) => {
                    let stray =
                        std::fs::symlink_metadata(&gen_dir).is_ok_and(|meta| !meta.is_dir());
                    if !stray {
                        keep.insert(gen_num);
                    }
                    warn!(
                        generation = gen_num,
                        path = %gen_dir.display(),
                        stray,
                        "cannot read generation mtime: {e}"
                    );
                }
            }
        }
    }

    Ok(keep)
}

/// Remove old profile generations according to GC policy.
///
/// Keeps:
/// - the current generation (pointed to by the `current` symlink);
/// - next-boot generations (pointed to by deferred-activation markers);
/// - the latest numbered generation, even when it is not current yet
///   (preserves generations built for deferred activation);
/// - protected generations from `gc_config.protected_generations`;
/// - the most recent `gc_config.keep_generations` generations by number;
/// - generations whose directory mtime is newer than `keep_days` days when
///   `gc_config.keep_days` is `Some`; `None` disables age-based retention;
/// - any generation listed in `keep_extra` (transient per-call protection,
///   e.g. the previous generation the orchestration layer still needs to
///   read after activation).
///
/// Removes everything else by deleting the generation directory, along
/// with any leftover `*.tmp` entries orphaned by a failed build, staging,
/// or activation swap.
///
/// Returns the number of removed entries — generation directories plus
/// leftover `*.tmp` entries — since either can hold the last reference to a
/// store path. Returns `Ok(0)` when `profile_dir` does not exist.
pub fn cleanup_generations(
    profile_dir: &Path,
    gc_config: &GcConfig,
    keep_extra: &[usize],
) -> Result<usize, CleanupGenerationsError> {
    if !profile_dir.exists() {
        return Ok(0);
    }

    let entries = std::fs::read_dir(profile_dir).map_err(CleanupGenerationsError::Cleanup)?;

    let mut generations: Vec<usize> = Vec::new();
    let mut leftover_tmp: Vec<String> = Vec::new();
    let mut stale_marker = false;
    for entry in entries {
        let entry = entry.map_err(CleanupGenerationsError::Cleanup)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(num) = crate::profile::parse_generation_link_name(&name) {
            generations.push(num);
        } else if is_leftover_tmp_name(&name) {
            leftover_tmp.push(name.into_owned());
        } else if name == LAST_GC_MARKER {
            stale_marker = true;
        }
    }

    // Left behind by the superseded interval-driven design. It roots nothing,
    // so its removal is not a reason to sweep, and failing to remove it is not
    // a reason to fail the cleanup.
    if stale_marker {
        remove_stale_gc_marker(profile_dir);
    }

    if generations.is_empty() && leftover_tmp.is_empty() {
        return Ok(0);
    }

    generations.sort_unstable();

    let keep = generations_to_keep(profile_dir, gc_config, keep_extra, &generations)?;

    // Remove generations not in keep set. Track failures so the caller can
    // detect partial cleanup; do not abort on the first failure (a
    // permissions glitch on one generation must not leave later
    // generations un-attempted).
    let mut attempted: usize = 0;
    let mut first_failure: Option<(String, std::io::Error)> = None;
    let mut failed: usize = 0;
    let mut removed: usize = 0;
    for &gen_num in &generations {
        if keep.contains(&gen_num) {
            continue;
        }

        attempted += 1;
        let gen_name = crate::profile::generation_link_name(gen_num);
        let gen_dir = profile_dir.join(&gen_name);
        info!(generation = gen_num, path = %gen_dir.display(), "removing old generation");
        match remove_profile_entry(&gen_dir) {
            Ok(()) => removed += 1,
            Err(e) => {
                warn!(
                    generation = gen_num,
                    path = %gen_dir.display(),
                    "failed to remove generation: {e}"
                );
                failed += 1;
                if first_failure.is_none() {
                    first_failure = Some((gen_name, e));
                }
            }
        }
    }

    // Sweep leftover temp entries orphaned by failed builds or staging.
    // The caller holds the profile lock for the whole cleanup, and builds
    // and staging run under that same lock, so no concurrent build can be
    // using these entries here.
    for name in &leftover_tmp {
        attempted += 1;
        let path = profile_dir.join(name);
        info!(path = %path.display(), "removing leftover temp entry");
        match remove_profile_entry(&path) {
            Ok(()) => removed += 1,
            Err(e) => {
                warn!(path = %path.display(), "failed to remove leftover temp entry: {e}");
                failed += 1;
                if first_failure.is_none() {
                    first_failure = Some((name.clone(), e));
                }
            }
        }
    }

    if let Some((first_entry, first_error)) = first_failure {
        return Err(CleanupGenerationsError::RemovalFailures {
            removed,
            attempted,
            failed,
            first_entry,
            first_error,
        });
    }

    Ok(removed)
}

/// Run `nix-collect-garbage` to remove unreachable store paths.
///
/// This is expensive (scans the entire store) and should be called on a
/// periodic timer or when disk space is low, NOT after every upgrade.
///
/// Streams the command's output line-by-line so `progress` can surface the
/// running count of deleted paths and the final freed-bytes tally; pass
/// `None` to run silently.
pub async fn collect_garbage(
    runner: &impl CommandRunner,
    progress: Option<&dyn CollectGarbageProgress>,
) -> Result<(), CollectGarbageError> {
    let mut deleted_paths: usize = 0;
    let mut summary: Option<(usize, Option<u64>)> = None;

    let output = runner
        .run_with_stderr_lines("nix-collect-garbage", &[], |line| {
            match parse_gc_line(line) {
                Some(GcOutputLine::Phase(phase)) => {
                    if let Some(p) = progress {
                        p.on_phase(phase);
                    }
                }
                Some(GcOutputLine::PathDeleted) => {
                    deleted_paths += 1;
                    if let Some(p) = progress {
                        p.on_deleted(deleted_paths);
                    }
                }
                Some(GcOutputLine::Summary {
                    deleted_paths,
                    freed_bytes,
                }) => {
                    summary = Some((deleted_paths, freed_bytes));
                }
                None => {}
            }
        })
        .await
        .map_err(CollectGarbageError::NixCommand)?;

    if !output.status.success() {
        return Err(CollectGarbageError::NixCommand(std::io::Error::other(
            format!(
                "nix-collect-garbage exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ),
        )));
    }

    if let Some(p) = progress {
        // Prefer nix's own summary count; fall back to the lines we counted
        // when the summary could not be parsed.
        let (total, freed_bytes) = summary.unwrap_or((deleted_paths, None));
        p.on_finished(total, freed_bytes);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct RecordingStore {
        collect_calls: std::sync::atomic::AtomicUsize,
        fail_collection: std::sync::atomic::AtomicBool,
    }

    impl RecordingStore {
        fn failing() -> Self {
            Self {
                fail_collection: std::sync::atomic::AtomicBool::new(true),
                ..Self::default()
            }
        }

        fn collect_calls(&self) -> usize {
            self.collect_calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl StoreOperations for RecordingStore {
        async fn estimate_realization(
            &self,
            _packages: &[crate::types::ResolvedPackage],
        ) -> Result<crate::store::RealizeEstimate, crate::store::StorePathError> {
            unreachable!("BUG: profile gc never estimates realization")
        }

        fn store_free_bytes(&self, _profile_dir: &std::path::Path) -> std::io::Result<u64> {
            unreachable!("BUG: profile gc never measures free space")
        }

        async fn realize_store_paths(
            &self,
            _packages: &[crate::types::ResolvedPackage],
            _progress: Option<&dyn crate::store::RealizeProgress>,
        ) -> Result<(), crate::store::StorePathError> {
            unreachable!("BUG: profile gc never realizes store paths")
        }

        async fn verify_store_paths(
            &self,
            _packages: &[crate::types::ResolvedPackage],
        ) -> Result<(), crate::store::StorePathError> {
            unreachable!("BUG: profile gc never verifies store paths")
        }

        fn collect_garbage(
            &self,
            _progress: Option<&dyn CollectGarbageProgress>,
        ) -> impl std::future::Future<Output = Result<(), CollectGarbageError>> + Send {
            let call = self
                .collect_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let fail = self
                .fail_collection
                .load(std::sync::atomic::Ordering::SeqCst);
            async move {
                if fail {
                    Err(CollectGarbageError::NixCommand(std::io::Error::other(
                        format!("collection {call} failed"),
                    )))
                } else {
                    Ok(())
                }
            }
        }
    }

    #[test]
    fn load_gc_config_missing_uses_defaults() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let config = load_gc_config(&tmp.path().join("missing.json"))
            .expect("BUG: missing config uses defaults");

        assert_eq!(
            config.keep_generations,
            GcConfig::default().keep_generations
        );
        assert_eq!(config.keep_days, GcConfig::default().keep_days);
        assert_eq!(
            config.protected_generations,
            GcConfig::default().protected_generations
        );
    }

    #[test]
    fn load_gc_config_partial_uses_field_defaults() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let path = tmp.path().join("gc.json");
        std::fs::write(&path, r#"{"keep_generations":9}"#).expect("BUG: write config");

        let config = load_gc_config(&path).expect("BUG: load partial config");

        assert_eq!(config.keep_generations, 9);
        assert_eq!(config.keep_days, GcConfig::default().keep_days);
        assert_eq!(
            config.protected_generations,
            GcConfig::default().protected_generations
        );
    }

    #[test]
    fn load_gc_config_reads_valid_file() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let path = tmp.path().join("gc.json");
        std::fs::write(
            &path,
            r#"{"keep_generations":7,"keep_days":14,"protected_generations":[2,5]}"#,
        )
        .expect("BUG: write config");

        let config = load_gc_config(&path).expect("BUG: load valid config");

        assert_eq!(config.keep_generations, 7);
        assert_eq!(config.keep_days, Some(14));
        assert_eq!(config.protected_generations, vec![2, 5]);
    }

    #[test]
    fn load_gc_config_read_failure_is_distinct() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");

        let error = load_gc_config(tmp.path()).expect_err("BUG: directory cannot be config file");

        assert!(matches!(error, LoadGcConfigError::Read { .. }));
    }

    #[test]
    fn load_gc_config_parse_failure_is_distinct() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let path = tmp.path().join("gc.json");
        std::fs::write(&path, "not json").expect("BUG: write malformed config");

        let error = load_gc_config(&path).expect_err("BUG: malformed config must fail");

        assert!(matches!(error, LoadGcConfigError::Parse { .. }));
    }

    #[test]
    fn default_gc_config_keeps_two_generations() {
        assert_eq!(GcConfig::default().keep_generations, 2);
    }

    fn create_generation(profile_dir: &Path, number: usize) {
        let gen_dir = profile_dir.join(format!("{number}-link"));
        std::fs::create_dir_all(&gen_dir).expect("BUG: mkdir");
        std::fs::write(gen_dir.join("manifest"), r#"{"packages":{}}"#)
            .expect("BUG: write manifest");
    }

    fn set_current(profile_dir: &Path, number: usize) {
        let current_link = profile_dir.join("current");
        let _ = std::fs::remove_file(&current_link);
        std::os::unix::fs::symlink(format!("{number}-link"), &current_link).expect("BUG: symlink");
    }

    fn set_next(profile_dir: &Path, number: usize) {
        let next_link = profile_dir.join("next");
        let _ = std::fs::remove_file(&next_link);
        std::os::unix::fs::symlink(format!("{number}-link"), &next_link)
            .expect("BUG: symlink next");
    }

    fn generation_exists(profile_dir: &Path, number: usize) -> bool {
        profile_dir.join(format!("{number}-link")).exists()
    }

    #[test]
    fn cleanup_generations_aborts_when_current_symlink_unreadable() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");
        for n in 1..=3 {
            create_generation(&profile_dir, n);
        }
        // `current` is a regular file, not a symlink: read_link fails with a
        // non-NotFound error. GC must abort rather than silently treat the
        // current generation as unprotected and prune it.
        std::fs::write(profile_dir.join("current"), b"not a symlink").expect("BUG: write");

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        let err = cleanup_generations(&profile_dir, &gc_config, &[])
            .expect_err("BUG: an unreadable current symlink must abort GC");
        assert!(
            matches!(err, CleanupGenerationsError::Cleanup(_)),
            "got {err:?}"
        );
        for n in 1..=3 {
            assert!(
                generation_exists(&profile_dir, n),
                "no generation may be pruned once GC aborts"
            );
        }
    }

    #[test]
    fn cleanup_keeps_protected_generations() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=3 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 3);

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            protected_generations: vec![1],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[]).expect("BUG: cleanup failed");

        // Generation 1 is protected, 3 is current -> 2 is removed.
        assert!(generation_exists(&profile_dir, 1));
        assert!(!generation_exists(&profile_dir, 2));
        assert!(generation_exists(&profile_dir, 3));
    }

    #[test]
    fn cleanup_keeps_recent_by_count() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=5 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 5);

        let gc_config = GcConfig {
            keep_generations: 2,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[]).expect("BUG: cleanup failed");

        // keep_generations=2 keeps 4 and 5.
        assert!(!generation_exists(&profile_dir, 1));
        assert!(!generation_exists(&profile_dir, 2));
        assert!(!generation_exists(&profile_dir, 3));
        assert!(generation_exists(&profile_dir, 4));
        assert!(generation_exists(&profile_dir, 5));
    }

    #[test]
    fn cleanup_prunes_around_an_undatable_generation() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in [1, 2, 5] {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 5);
        let old = SystemTime::now() - std::time::Duration::from_hours(60 * 24);
        let times = std::fs::FileTimes::new().set_modified(old);
        let gen1 = std::fs::File::open(profile_dir.join("1-link")).expect("BUG: open gen 1");
        gen1.set_times(times).expect("BUG: set mtime");

        // A dangling generation symlink is listed like any other generation,
        // so the keep_days pass has to survive one it cannot date.
        // Were it to abort, every scheduled cleanup would fail
        // for as long as the stray entry sat there.
        std::os::unix::fs::symlink("missing", profile_dir.join("4-link"))
            .expect("BUG: symlink dangling gen");

        let gc_config = GcConfig {
            keep_generations: 0,
            keep_days: Some(30),
            protected_generations: vec![],
            ..GcConfig::default()
        };
        let removed = cleanup_generations(&profile_dir, &gc_config, &[])
            .expect("BUG: an undatable generation must not abort cleanup");

        assert_eq!(removed, 2, "the aged generation and the stray entry go");
        assert!(
            !generation_exists(&profile_dir, 1),
            "gen 1 is older than keep_days and not otherwise protected -> removed"
        );
        assert!(
            generation_exists(&profile_dir, 2),
            "gen 2 is within keep_days -> kept by age-based retention"
        );
        assert!(generation_exists(&profile_dir, 5), "current gen is kept");
        assert!(
            std::fs::symlink_metadata(profile_dir.join("4-link")).is_err(),
            "the undatable stray earns no age protection, so it is swept"
        );
    }

    #[test]
    fn cleanup_keeps_recent_removes_old_by_days() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=4 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 1);

        // Age generation 2 well past the retention window; generation 3 stays
        // fresh. Neither is current (1) nor latest (4), so with
        // keep_generations=0 their fate is decided solely by keep_days.
        let old = SystemTime::now() - std::time::Duration::from_hours(60 * 24);
        let times = std::fs::FileTimes::new().set_modified(old);
        let gen2 = std::fs::File::open(profile_dir.join("2-link")).expect("BUG: open gen 2");
        gen2.set_times(times).expect("BUG: set mtime");

        let gc_config = GcConfig {
            keep_generations: 0,
            keep_days: Some(30),
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[]).expect("BUG: cleanup failed");

        assert!(generation_exists(&profile_dir, 1), "current gen is kept");
        assert!(
            !generation_exists(&profile_dir, 2),
            "gen 2 is older than keep_days and not otherwise protected -> removed"
        );
        assert!(
            generation_exists(&profile_dir, 3),
            "gen 3 is within keep_days -> kept by age-based retention"
        );
        assert!(generation_exists(&profile_dir, 4), "latest gen is kept");
    }

    #[test]
    fn cleanup_keeps_current_generation() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=5 {
            create_generation(&profile_dir, n);
        }
        // current points to an old generation outside the recent-count window.
        set_current(&profile_dir, 2);

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[]).expect("BUG: cleanup failed");

        // current (2) and latest (5) survive; the rest are removed.
        assert!(!generation_exists(&profile_dir, 1));
        assert!(generation_exists(&profile_dir, 2), "current gen is kept");
        assert!(!generation_exists(&profile_dir, 3));
        assert!(!generation_exists(&profile_dir, 4));
        assert!(generation_exists(&profile_dir, 5), "latest gen is kept");
    }

    #[test]
    fn cleanup_keeps_latest_even_when_not_current() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        create_generation(&profile_dir, 1);
        create_generation(&profile_dir, 2);
        set_current(&profile_dir, 1);

        let gc_config = GcConfig {
            keep_generations: 0,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[]).expect("BUG: cleanup failed");

        assert!(generation_exists(&profile_dir, 1), "current gen is kept");
        assert!(
            generation_exists(&profile_dir, 2),
            "latest non-current gen is kept for deferred activation"
        );
    }

    #[test]
    fn cleanup_generations_protects_next_target() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=5 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 5);
        set_next(&profile_dir, 3);
        // A versioned marker — possibly staged for another firmware —
        // protects its generation just like a bare one until an
        // activator or staging run sweeps it.
        std::os::unix::fs::symlink("2-link", profile_dir.join("next.9.9"))
            .expect("BUG: symlink next.9.9");

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[]).expect("BUG: cleanup failed");

        assert!(
            generation_exists(&profile_dir, 3),
            "generation targeted by next must survive cleanup"
        );
        assert!(
            generation_exists(&profile_dir, 2),
            "generation targeted by a versioned marker must survive cleanup"
        );
        assert!(generation_exists(&profile_dir, 5), "current gen is kept");
    }

    #[test]
    fn cleanup_tolerates_non_symlink_next_marker() {
        // A directory (or plain file) squatting on a `next.*` marker name makes
        // `read_link` return EINVAL. GC must treat it like an absent marker and
        // still prune, not abort every run — the same stray `sweep_next_markers`
        // skips with its `is_symlink` guard.
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=3 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 3);
        std::fs::create_dir(profile_dir.join("next.junk")).expect("BUG: mk next.junk");

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[])
            .expect("BUG: a non-symlink next marker must not abort cleanup");

        // GC ran to completion rather than aborting on the junk marker.
        assert!(
            !generation_exists(&profile_dir, 2),
            "unprotected generation is still pruned"
        );
        assert!(generation_exists(&profile_dir, 3), "current gen is kept");
        assert!(
            profile_dir.join("next.junk").is_dir(),
            "the stray marker-named dir is left in place"
        );
    }

    #[test]
    fn cleanup_keeps_freshly_built_generation_staged_for_next_boot() {
        // A --next-boot upgrade builds the highest-numbered generation and
        // stages it as `next`, while `current` still points at the
        // pre-upgrade generation. The freshly built generation must survive
        // cleanup on its own, with no explicit `keep_extra` entry.
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=3 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 1);
        set_next(&profile_dir, 3);

        let gc_config = GcConfig {
            keep_generations: 0,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[]).expect("BUG: cleanup failed");

        assert!(
            generation_exists(&profile_dir, 3),
            "freshly built generation staged for next boot must survive cleanup"
        );
        assert!(
            !generation_exists(&profile_dir, 2),
            "an unprotected intermediate generation is still collected"
        );
        assert!(
            generation_exists(&profile_dir, 1),
            "pre-upgrade current gen is kept"
        );
    }

    #[test]
    fn cleanup_keeps_transient_extra_generation() {
        // Post-activation case: current=3 (newly activated), latest=3, but
        // the orchestrator still needs gen 2 (the pre-activation current).
        // With keep_generations=1 and no other protection, gen 2 would be
        // gc'd unless explicitly listed in `keep_extra`.
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=3 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 3);

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[2]).expect("BUG: cleanup failed");

        assert!(
            !generation_exists(&profile_dir, 1),
            "gen 1 not in keep_extra and not current"
        );
        assert!(
            generation_exists(&profile_dir, 2),
            "gen 2 must be kept because it is in keep_extra"
        );
        assert!(generation_exists(&profile_dir, 3), "gen 3 is current");
    }

    #[test]
    fn cleanup_removes_leftover_tmp_entries() {
        // A crashed atomic swap can orphan any `*.tmp` under the gcroots
        // profile directory — `<N>-link.tmp` from a build, `.next.tmp` from
        // staging, `current.tmp` from an activation repoint — where every
        // symlink inside keeps store paths alive. GC sweeps every `*.tmp`
        // regardless of prefix while leaving real generations and `current`
        // untouched.
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=2 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 2);

        let stray_build = profile_dir.join("7-link.tmp");
        std::fs::create_dir_all(&stray_build).expect("BUG: mkdir stray build");
        std::os::unix::fs::symlink("/nix/store/fake-pkg", stray_build.join("pkg"))
            .expect("BUG: symlink in stray build");
        std::os::unix::fs::symlink("7-link", profile_dir.join(".next.tmp"))
            .expect("BUG: symlink stray next");
        std::os::unix::fs::symlink("2-link", profile_dir.join("current.tmp"))
            .expect("BUG: symlink stray current");
        std::os::unix::fs::symlink("1-link", profile_dir.join("orphan.tmp"))
            .expect("BUG: symlink arbitrary stray");

        let gc_config = GcConfig {
            keep_generations: 2,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[]).expect("BUG: cleanup failed");

        assert!(
            profile_dir.join("7-link.tmp").symlink_metadata().is_err(),
            "leftover build tmp dir must be removed"
        );
        assert!(
            profile_dir.join(".next.tmp").symlink_metadata().is_err(),
            "leftover staging tmp symlink must be removed"
        );
        assert!(
            profile_dir.join("current.tmp").symlink_metadata().is_err(),
            "leftover activation swap tmp must be removed"
        );
        assert!(
            profile_dir.join("orphan.tmp").symlink_metadata().is_err(),
            "any *.tmp must be removed regardless of prefix"
        );
        assert!(generation_exists(&profile_dir, 1));
        assert!(generation_exists(&profile_dir, 2));
        assert_eq!(
            std::fs::read_link(profile_dir.join("current")).expect("BUG: read current"),
            Path::new("2-link"),
            "current must be left intact"
        );
    }

    #[test]
    fn cleanup_missing_profile_dir_succeeds() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("nonexistent");

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[]).expect("BUG: cleanup failed");
    }

    #[test]
    fn cleanup_reports_removal_failures() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=3 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 3);

        // Remove write permission on the parent so remove_dir_all fails when
        // attempting to remove an unprotected generation, while keeping
        // read+execute so the directory can still be scanned. Restore the
        // captured permissions before asserting so temp-dir cleanup works
        // even if the assertion panics.
        let original_perms = std::fs::metadata(&profile_dir)
            .expect("BUG: stat profile_dir")
            .permissions();
        let mut readonly_perms = original_perms.clone();
        readonly_perms.set_mode(0o555);
        std::fs::set_permissions(&profile_dir, readonly_perms).expect("BUG: chmod");

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        let result = cleanup_generations(&profile_dir, &gc_config, &[]);

        std::fs::set_permissions(&profile_dir, original_perms).expect("BUG: chmod restore");

        let err = result.expect_err("BUG: removal must fail under readonly parent");
        assert!(
            matches!(
                err,
                CleanupGenerationsError::RemovalFailures {
                    attempted, failed, ..
                } if attempted >= 1 && failed >= 1
            ),
            "expected RemovalFailures with >=1 attempted/failed, got {err:?}"
        );
    }

    #[test]
    fn cleanup_generations_keeps_previous_current_generation_after_activation() {
        // After activation, the orchestration layer passes the pre-activation
        // generation as `keep_extra` so GC does not reclaim it before the
        // caller can read its manifest. This scenario: gen 3 is the freshly
        // built (latest) generation, current still points to gen 2 (not yet
        // activated in this simulation), and gen 2 is also in keep_extra.
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=3 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 2);

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            protected_generations: vec![],
            ..GcConfig::default()
        };
        cleanup_generations(&profile_dir, &gc_config, &[2]).expect("BUG: cleanup failed");

        assert!(
            !generation_exists(&profile_dir, 1),
            "gen 1 is not current, not latest, not protected"
        );
        assert!(
            generation_exists(&profile_dir, 2),
            "gen 2 is current and in keep_extra"
        );
        assert!(
            generation_exists(&profile_dir, 3),
            "gen 3 is the latest generation"
        );
    }

    #[test]
    fn parse_binary_size_converts_each_unit() {
        assert_eq!(parse_binary_size("512 B"), Some(512));
        assert_eq!(parse_binary_size("1 KiB"), Some(1024));
        assert_eq!(
            parse_binary_size("1.50 MiB"),
            Some(1024 * 1024 + 512 * 1024)
        );
        assert_eq!(parse_binary_size("2 GiB"), Some(2 * 1024 * 1024 * 1024));
    }

    #[test]
    fn parse_binary_size_rejects_unknown_unit_and_garbage() {
        assert_eq!(parse_binary_size("1.0 PiB"), None);
        assert_eq!(parse_binary_size("not-a-number MiB"), None);
        assert_eq!(parse_binary_size("12"), None);
        assert_eq!(parse_binary_size("-1.0 MiB"), None);
    }

    #[test]
    fn parse_gc_line_classifies_phases_deletions_and_summary() {
        assert!(matches!(
            parse_gc_line("finding garbage collector roots..."),
            Some(GcOutputLine::Phase(CollectGarbagePhase::FindingRoots))
        ));
        assert!(matches!(
            parse_gc_line("determining live/dead paths..."),
            Some(GcOutputLine::Phase(
                CollectGarbagePhase::DeterminingLiveness
            ))
        ));
        assert!(matches!(
            parse_gc_line("deleting '/nix/store/aaa-foo'"),
            Some(GcOutputLine::PathDeleted)
        ));
        // The unused-links line also begins with "deleting " but is not a
        // per-path deletion, so it must not inflate the count.
        assert!(parse_gc_line("deleting unused links...").is_none());
        assert!(parse_gc_line("some unrelated noise").is_none());
    }

    #[test]
    fn parse_gc_summary_extracts_count_and_freed_bytes() {
        match parse_gc_line("2 store paths deleted, 1.50 MiB freed") {
            Some(GcOutputLine::Summary {
                deleted_paths,
                freed_bytes,
            }) => {
                assert_eq!(deleted_paths, 2);
                assert_eq!(freed_bytes, Some(1024 * 1024 + 512 * 1024));
            }
            _ => panic!("expected a summary line"),
        }
    }

    #[test]
    fn parse_gc_summary_tolerates_unparsable_freed_amount() {
        match parse_gc_line("7 store paths deleted") {
            Some(GcOutputLine::Summary {
                deleted_paths,
                freed_bytes,
            }) => {
                assert_eq!(deleted_paths, 7);
                assert_eq!(freed_bytes, None);
            }
            _ => panic!("expected a summary line even without a freed amount"),
        }
    }

    /// Minimal [`CommandRunner`] that replays a fixed list of output lines.
    struct MockGcRunner {
        lines: Vec<String>,
        success: bool,
    }

    impl CommandRunner for MockGcRunner {
        async fn run(
            &self,
            _program: &str,
            _args: &[&str],
        ) -> Result<std::process::Output, std::io::Error> {
            use std::os::unix::process::ExitStatusExt as _;
            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }

        async fn run_with_stderr_lines<F>(
            &self,
            _program: &str,
            _args: &[&str],
            mut on_line: F,
        ) -> Result<std::process::Output, std::io::Error>
        where
            F: FnMut(&str) + Send,
        {
            use std::os::unix::process::ExitStatusExt as _;
            let mut stderr_bytes = Vec::new();
            for line in &self.lines {
                on_line(line);
                stderr_bytes.extend_from_slice(line.as_bytes());
                stderr_bytes.push(b'\n');
            }
            let code = i32::from(!self.success);
            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(code << 8),
                stdout: Vec::new(),
                stderr: stderr_bytes,
            })
        }
    }

    #[derive(Default)]
    struct RecordingProgress {
        phases: std::sync::Mutex<Vec<CollectGarbagePhase>>,
        deleted: std::sync::Mutex<Vec<usize>>,
        finished: std::sync::Mutex<Option<(usize, Option<u64>)>>,
    }

    impl CollectGarbageProgress for RecordingProgress {
        fn on_phase(&self, phase: CollectGarbagePhase) {
            self.phases.lock().expect("BUG: poisoned").push(phase);
        }
        fn on_deleted(&self, deleted_paths: usize) {
            self.deleted
                .lock()
                .expect("BUG: poisoned")
                .push(deleted_paths);
        }
        fn on_finished(&self, deleted_paths: usize, freed_bytes: Option<u64>) {
            *self.finished.lock().expect("BUG: poisoned") = Some((deleted_paths, freed_bytes));
        }
    }

    #[tokio::test]
    async fn collect_garbage_streams_running_count_and_final_summary() {
        let runner = MockGcRunner {
            lines: vec![
                "finding garbage collector roots...".to_owned(),
                "determining live/dead paths...".to_owned(),
                "deleting '/nix/store/aaa'".to_owned(),
                "deleting '/nix/store/bbb'".to_owned(),
                "deleting unused links...".to_owned(),
                "2 store paths deleted, 1.50 MiB freed".to_owned(),
            ],
            success: true,
        };
        let progress = RecordingProgress::default();

        collect_garbage(&runner, Some(&progress))
            .await
            .expect("BUG: gc should succeed");

        assert_eq!(
            *progress.phases.lock().expect("BUG: poisoned"),
            vec![
                CollectGarbagePhase::FindingRoots,
                CollectGarbagePhase::DeterminingLiveness,
            ]
        );
        assert_eq!(
            *progress.deleted.lock().expect("BUG: poisoned"),
            vec![1, 2],
            "count increments once per deleted path, ignoring the unused-links line"
        );
        assert_eq!(
            *progress.finished.lock().expect("BUG: poisoned"),
            Some((2, Some(1024 * 1024 + 512 * 1024))),
            "final tally comes from nix's summary line"
        );
    }

    #[tokio::test]
    async fn collect_garbage_reports_error_on_nonzero_exit() {
        let runner = MockGcRunner {
            lines: vec!["error: failed to delete".to_owned()],
            success: false,
        };

        let err = collect_garbage(&runner, None)
            .await
            .expect_err("BUG: non-zero exit must surface an error");
        assert!(matches!(err, CollectGarbageError::NixCommand(_)));
    }

    fn periodic_request() -> GcRequest {
        GcRequest {
            on_busy: OnBusy::Skip,
            sweep: Sweep::WhenGenerationsRemoved,
        }
    }

    fn forced_request() -> GcRequest {
        GcRequest {
            on_busy: OnBusy::Wait,
            sweep: Sweep::Always,
        }
    }

    /// Three generations with `current` on the last, so a `keep_generations`
    /// of one leaves two removable.
    fn profile_with_removable_generations(profile_dir: &Path) {
        std::fs::create_dir(profile_dir).expect("BUG: create profile");
        for number in 1..=3 {
            create_generation(profile_dir, number);
        }
        set_current(profile_dir, 3);
    }

    fn keep_one() -> GcConfig {
        GcConfig {
            keep_generations: 1,
            ..GcConfig::default()
        }
    }

    #[tokio::test]
    async fn a_store_sweep_failure_surfaces() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        let store = RecordingStore::failing();

        let error = collect_profile_garbage(
            &store,
            &profile_dir,
            &GcConfig::default(),
            forced_request(),
            None,
        )
        .await
        .expect_err("BUG: store failure must surface");

        assert!(matches!(error, ProfileGcError::Sweep { .. }));
        assert_eq!(store.collect_calls(), 1);
    }

    /// A generation whose removal fails: a read-only subdirectory forbids
    /// unlinking the file inside it. `TempDir` cleanup ignores the leftovers.
    fn create_unremovable_generation(profile_dir: &Path, number: usize) {
        use std::os::unix::fs::PermissionsExt as _;
        create_generation(profile_dir, number);
        let poison = profile_dir.join(format!("{number}-link")).join("poison");
        std::fs::create_dir(&poison).expect("BUG: create poison dir");
        std::fs::write(poison.join("file"), b"x").expect("BUG: write poison file");
        std::fs::set_permissions(&poison, std::fs::Permissions::from_mode(0o555))
            .expect("BUG: chmod poison");
    }

    #[tokio::test]
    async fn cleanup_removes_generation_entries_that_are_not_directories() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        std::fs::write(profile_dir.join("1-link"), b"not a directory")
            .expect("BUG: write bogus generation");
        std::os::unix::fs::symlink("nowhere", profile_dir.join("2-link"))
            .expect("BUG: symlink bogus generation");
        create_generation(&profile_dir, 3);
        set_current(&profile_dir, 3);
        let store = RecordingStore::default();

        let outcome =
            collect_profile_garbage(&store, &profile_dir, &keep_one(), periodic_request(), None)
                .await
                .expect("BUG: a stray entry on a generation name must not fail cleanup");

        assert_eq!(outcome, ProfileGcOutcome::Collected);
        assert!(
            !profile_dir.join("1-link").exists(),
            "a regular file is unlinked"
        );
        assert!(
            profile_dir.join("2-link").symlink_metadata().is_err(),
            "a dangling symlink is unlinked, not followed"
        );
    }

    #[tokio::test]
    async fn a_partial_cleanup_failure_still_sweeps_what_it_unrooted() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        create_unremovable_generation(&profile_dir, 1);
        create_generation(&profile_dir, 2);
        create_generation(&profile_dir, 3);
        set_current(&profile_dir, 3);
        let store = RecordingStore::default();

        let outcome =
            collect_profile_garbage(&store, &profile_dir, &keep_one(), periodic_request(), None)
                .await
                .expect("BUG: a partial cleanup failure must not abort collection");

        assert_eq!(outcome, ProfileGcOutcome::SweptDespiteCleanupFailure);
        assert_eq!(
            store.collect_calls(),
            1,
            "generation 2 was unrooted, so only a sweep can reclaim what it rooted"
        );
        assert!(!generation_exists(&profile_dir, 2));
        assert!(generation_exists(&profile_dir, 1));
    }

    #[tokio::test]
    async fn a_cleanup_failure_that_unrooted_nothing_surfaces_without_a_sweep() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        create_unremovable_generation(&profile_dir, 1);
        create_generation(&profile_dir, 2);
        set_current(&profile_dir, 2);
        let store = RecordingStore::default();

        let error =
            collect_profile_garbage(&store, &profile_dir, &keep_one(), periodic_request(), None)
                .await
                .expect_err("BUG: a cleanup that removed nothing has nothing to sweep");

        assert!(matches!(error, ProfileGcError::Cleanup(_)));
        assert_eq!(
            error.unswept_removals(),
            0,
            "the failed entry is still in place for the next run to retry"
        );
        assert_eq!(store.collect_calls(), 0);
    }

    #[tokio::test]
    async fn a_forced_sweep_runs_past_a_cleanup_failure() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        create_unremovable_generation(&profile_dir, 1);
        create_generation(&profile_dir, 2);
        set_current(&profile_dir, 2);
        let store = RecordingStore::default();

        let outcome =
            collect_profile_garbage(&store, &profile_dir, &keep_one(), forced_request(), None)
                .await
                .expect("BUG: a forced sweep must not be blocked by cleanup");

        assert_eq!(outcome, ProfileGcOutcome::SweptDespiteCleanupFailure);
        assert_eq!(
            store.collect_calls(),
            1,
            "a persistent cleanup failure must not block forced collection"
        );
    }

    #[tokio::test]
    async fn a_sweep_failure_after_partial_cleanup_reports_the_removals() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        create_unremovable_generation(&profile_dir, 1);
        create_generation(&profile_dir, 2);
        create_generation(&profile_dir, 3);
        set_current(&profile_dir, 3);
        let store = RecordingStore::failing();

        let error =
            collect_profile_garbage(&store, &profile_dir, &keep_one(), periodic_request(), None)
                .await
                .expect_err("BUG: store failure must surface");

        assert!(matches!(error, ProfileGcError::Sweep { .. }));
        assert_eq!(
            error.unswept_removals(),
            1,
            "generation 2 is gone and no later cleanup will count it again, \
             so the next occurrence must sweep unconditionally"
        );
    }

    #[tokio::test]
    async fn conditional_sweep_is_skipped_when_cleanup_removed_nothing() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        create_generation(&profile_dir, 1);
        set_current(&profile_dir, 1);
        let store = RecordingStore::default();

        let outcome = collect_profile_garbage(
            &store,
            &profile_dir,
            &GcConfig::default(),
            periodic_request(),
            None,
        )
        .await
        .expect("BUG: gc succeeds");

        assert_eq!(outcome, ProfileGcOutcome::NothingToCollect);
        assert_eq!(
            store.collect_calls(),
            0,
            "an idle device runs no store sweep"
        );
    }

    #[tokio::test]
    async fn conditional_sweep_runs_when_cleanup_removed_something() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        profile_with_removable_generations(&profile_dir);
        let store = RecordingStore::default();

        let outcome =
            collect_profile_garbage(&store, &profile_dir, &keep_one(), periodic_request(), None)
                .await
                .expect("BUG: gc succeeds");

        assert_eq!(outcome, ProfileGcOutcome::Collected);
        assert_eq!(store.collect_calls(), 1);
    }

    #[tokio::test]
    async fn unconditional_sweep_runs_with_nothing_removed() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        let store = RecordingStore::default();

        let outcome = collect_profile_garbage(
            &store,
            &profile_dir,
            &GcConfig::default(),
            forced_request(),
            None,
        )
        .await
        .expect("BUG: gc succeeds");

        assert_eq!(outcome, ProfileGcOutcome::Collected);
        assert_eq!(store.collect_calls(), 1);
    }

    #[tokio::test]
    async fn a_disabling_configuration_does_not_stop_collection() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        profile_with_removable_generations(&profile_dir);
        let store = RecordingStore::default();
        let config = GcConfig {
            periodic: crate::types::PeriodicGcMode::Disabled,
            ..keep_one()
        };

        let outcome =
            collect_profile_garbage(&store, &profile_dir, &config, periodic_request(), None)
                .await
                .expect("BUG: gc succeeds");

        assert_eq!(
            outcome,
            ProfileGcOutcome::Collected,
            "the toggle is the periodic job's decision, not this function's"
        );
    }

    #[tokio::test]
    async fn skip_on_busy_reports_busy_while_the_profile_lock_is_held() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        let store = RecordingStore::default();
        let held = crate::profile::lock_profile(&profile_dir)
            .await
            .expect("BUG: lock profile");

        let outcome = collect_profile_garbage(
            &store,
            &profile_dir,
            &GcConfig::default(),
            periodic_request(),
            None,
        )
        .await
        .expect("BUG: gc succeeds");

        assert_eq!(outcome, ProfileGcOutcome::Busy);
        assert_eq!(store.collect_calls(), 0);
        drop(held);
    }

    #[tokio::test]
    async fn wait_on_busy_collects_once_the_profile_lock_is_released() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        let store = RecordingStore::default();
        let held = crate::profile::lock_profile(&profile_dir)
            .await
            .expect("BUG: lock profile");

        let config = GcConfig::default();
        let mut gc = Box::pin(collect_profile_garbage(
            &store,
            &profile_dir,
            &config,
            forced_request(),
            None,
        ));

        // Deterministic, not timing-based: with the lock held the future
        // cannot finish, and it must finish once the lock is gone.
        assert!(
            futures::poll!(&mut gc).is_pending(),
            "waiting must not collect while another writer holds the profile"
        );
        assert_eq!(store.collect_calls(), 0);

        drop(held);

        assert_eq!(
            gc.await.expect("BUG: gc succeeds"),
            ProfileGcOutcome::Collected
        );
    }

    #[tokio::test]
    async fn a_failed_sweep_reports_the_removals_it_left_unswept() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        profile_with_removable_generations(&profile_dir);
        let store = RecordingStore::failing();

        let error =
            collect_profile_garbage(&store, &profile_dir, &keep_one(), periodic_request(), None)
                .await
                .expect_err("BUG: gc must fail");

        assert_eq!(
            error.unswept_removals(),
            2,
            "the removed generations are gone; only an escalated sweep can reclaim them"
        );
    }

    #[test]
    fn cleanup_generations_counts_removed_generations_and_tmp_entries() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        // Generations 1..=4 with `current` on 4; keep_generations = 2 keeps 3 and 4.
        for number in 1..=4 {
            create_generation(&profile_dir, number);
        }
        set_current(&profile_dir, 4);
        std::fs::create_dir_all(profile_dir.join("5-link.tmp")).expect("BUG: create leftover tmp");

        let config = GcConfig {
            keep_generations: 2,
            ..GcConfig::default()
        };

        let removed =
            cleanup_generations(&profile_dir, &config, &[]).expect("BUG: cleanup succeeds");

        // Generations 1 and 2, plus the leftover tmp entry.
        assert_eq!(removed, 3);
        assert!(!generation_exists(&profile_dir, 1));
        assert!(!profile_dir.join("5-link.tmp").exists());
    }

    #[test]
    fn cleanup_generations_reports_zero_when_nothing_is_removable() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        create_generation(&profile_dir, 1);
        set_current(&profile_dir, 1);

        let removed = cleanup_generations(&profile_dir, &GcConfig::default(), &[])
            .expect("BUG: cleanup succeeds");

        assert_eq!(removed, 0);
    }

    #[test]
    fn cleanup_generations_removes_a_stale_last_gc_marker_without_counting_it() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        create_generation(&profile_dir, 1);
        set_current(&profile_dir, 1);
        let marker_path = profile_dir.join(LAST_GC_MARKER);
        std::fs::write(&marker_path, b"").expect("BUG: write marker");

        let removed = cleanup_generations(&profile_dir, &GcConfig::default(), &[])
            .expect("BUG: cleanup succeeds");

        assert_eq!(
            removed, 0,
            "the marker roots nothing, so it is not a sweep reason"
        );
        assert!(!marker_path.exists());
    }

    #[test]
    fn a_partial_cleanup_reports_what_it_did_remove() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        // Generation 1's removal fails while the other removable entry
        // succeeds. A real failure, not a hand-built error: this is what
        // catches a `removed` counter that stops incrementing.
        create_unremovable_generation(&profile_dir, 1);
        for number in 2..=4 {
            create_generation(&profile_dir, number);
        }
        set_current(&profile_dir, 4);

        let config = GcConfig {
            keep_generations: 2,
            ..GcConfig::default()
        };

        let error = cleanup_generations(&profile_dir, &config, &[])
            .expect_err("BUG: the failed removal must surface");

        // Generations 1 and 2 are droppable; 2 goes, 1 fails.
        assert_eq!(error.removed(), 1);
        assert!(matches!(
            error,
            CleanupGenerationsError::RemovalFailures {
                attempted: 2,
                failed: 1,
                ..
            }
        ));
    }

    #[test]
    fn gc_config_without_the_periodic_field_stays_enabled() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let path = tmp.path().join("gc.json");
        std::fs::write(&path, br#"{"keep_generations": 3}"#).expect("BUG: write config");

        let config = load_gc_config(&path).expect("BUG: load config");

        assert_eq!(config.keep_generations, 3);
        assert_eq!(config.periodic, crate::types::PeriodicGcMode::Enabled);
    }

    #[test]
    fn gc_config_can_disable_periodic_collection() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let path = tmp.path().join("gc.json");
        std::fs::write(&path, br#"{"periodic": "disabled"}"#).expect("BUG: write config");

        let config = load_gc_config(&path).expect("BUG: load config");

        assert_eq!(config.periodic, crate::types::PeriodicGcMode::Disabled);
        assert_eq!(
            config.keep_generations,
            GcConfig::default().keep_generations,
            "an unrelated field keeps its default"
        );
    }

    #[test]
    fn an_enumeration_failure_reports_no_removals() {
        let error = CleanupGenerationsError::Cleanup(std::io::Error::other("boom"));

        assert_eq!(
            error.removed(),
            0,
            "enumeration failures happen before any removal"
        );
    }
}
