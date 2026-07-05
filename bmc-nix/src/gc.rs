// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;

use tracing::{info, warn};

use crate::store::CommandRunner;
use crate::types::GcConfig;

/// Errors that can occur when cleaning up generations.
#[derive(Debug, thiserror::Error)]
pub enum CleanupGenerationsError {
    #[error("generation cleanup failed: {0}")]
    Cleanup(#[source] std::io::Error),
    #[error(
        "failed to remove {failed} of {attempted} generation(s); \
         first failure was generation {first_gen}: {first_error}"
    )]
    RemovalFailures {
        attempted: usize,
        failed: usize,
        first_gen: usize,
        #[source]
        first_error: std::io::Error,
    },
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
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(numbers)
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
/// Removes everything else by deleting the generation directory. Returns
/// `Ok(())` when `profile_dir` does not exist.
pub fn cleanup_generations(
    profile_dir: &Path,
    gc_config: &GcConfig,
    keep_extra: &[usize],
) -> Result<(), CleanupGenerationsError> {
    if !profile_dir.exists() {
        return Ok(());
    }

    let entries = std::fs::read_dir(profile_dir).map_err(CleanupGenerationsError::Cleanup)?;

    let mut generations: Vec<usize> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(CleanupGenerationsError::Cleanup)?;
        let name = entry.file_name();
        if let Some(num) = crate::profile::parse_generation_link_name(&name.to_string_lossy()) {
            generations.push(num);
        }
    }

    if generations.is_empty() {
        return Ok(());
    }

    generations.sort_unstable();

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

        for &gen_num in &generations {
            let gen_dir = profile_dir.join(crate::profile::generation_link_name(gen_num));
            let metadata = std::fs::metadata(&gen_dir).map_err(CleanupGenerationsError::Cleanup)?;
            let mtime = metadata
                .modified()
                .map_err(CleanupGenerationsError::Cleanup)?;
            if mtime > cutoff {
                keep.insert(gen_num);
            }
        }
    }

    // Remove generations not in keep set. Track failures so the caller can
    // detect partial cleanup; do not abort on the first failure (a
    // permissions glitch on one generation must not leave later
    // generations un-attempted).
    let mut attempted: usize = 0;
    let mut first_failure: Option<(usize, std::io::Error)> = None;
    let mut failed: usize = 0;
    for &gen_num in &generations {
        if keep.contains(&gen_num) {
            continue;
        }

        attempted += 1;
        let gen_dir = profile_dir.join(crate::profile::generation_link_name(gen_num));
        info!(generation = gen_num, path = %gen_dir.display(), "removing old generation");
        if let Err(e) = std::fs::remove_dir_all(&gen_dir) {
            warn!(
                generation = gen_num,
                path = %gen_dir.display(),
                "failed to remove generation: {e}"
            );
            failed += 1;
            if first_failure.is_none() {
                first_failure = Some((gen_num, e));
            }
        }
    }

    if let Some((first_gen, first_error)) = first_failure {
        return Err(CleanupGenerationsError::RemovalFailures {
            attempted,
            failed,
            first_gen,
            first_error,
        });
    }

    Ok(())
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
    fn cleanup_aborts_when_keep_days_stat_fails() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        for n in 1..=2 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 2);
        // A dangling generation symlink: the keep_days pass stats every listed
        // generation, and stat on this entry fails with NotFound. That error
        // must propagate rather than being silently swallowed.
        std::os::unix::fs::symlink("missing", profile_dir.join("4-link"))
            .expect("BUG: symlink dangling gen");

        let gc_config = GcConfig {
            keep_generations: 0,
            keep_days: Some(30),
            protected_generations: vec![],
        };
        let err = cleanup_generations(&profile_dir, &gc_config, &[])
            .expect_err("BUG: a stat failure in the keep_days pass must abort GC");
        assert!(
            matches!(err, CleanupGenerationsError::Cleanup(_)),
            "got {err:?}"
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
        let old = SystemTime::now() - std::time::Duration::from_secs(60 * 24 * 60 * 60);
        let times = std::fs::FileTimes::new().set_modified(old);
        let gen2 = std::fs::File::open(profile_dir.join("2-link")).expect("BUG: open gen 2");
        gen2.set_times(times).expect("BUG: set mtime");

        let gc_config = GcConfig {
            keep_generations: 0,
            keep_days: Some(30),
            protected_generations: vec![],
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
    fn cleanup_missing_profile_dir_succeeds() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("nonexistent");

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: None,
            protected_generations: vec![],
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
}
