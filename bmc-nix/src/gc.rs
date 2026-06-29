// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;

use tracing::{info, warn};

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

/// Parse a generation number from a directory name matching `N-link`.
fn parse_generation_number(name: &str) -> Option<usize> {
    let stripped = name.strip_suffix("-link")?;
    stripped.parse::<usize>().ok()
}

/// Read the `current` symlink in `profile_dir` and return the generation
/// number it points to.
fn current_generation_number(profile_dir: &Path) -> Option<usize> {
    let current_link = profile_dir.join("current");
    let target = std::fs::read_link(&current_link).ok()?;
    let name = target.file_name()?.to_str()?;
    parse_generation_number(name)
}

/// Remove old profile generations according to GC policy.
///
/// Keeps:
/// - the current generation (pointed to by the `current` symlink);
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
///
/// `gc_config.min_free_space` is intentionally not consulted here; it drives
/// store-GC pressure decisions outside generation-directory cleanup.
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
        if let Some(num) = parse_generation_number(&name.to_string_lossy()) {
            generations.push(num);
        }
    }

    if generations.is_empty() {
        return Ok(());
    }

    generations.sort_unstable();

    let mut keep: HashSet<usize> = HashSet::new();

    if let Some(current) = current_generation_number(profile_dir) {
        keep.insert(current);
    }

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
            let gen_dir = profile_dir.join(format!("{gen_num}-link"));
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
        let gen_dir = profile_dir.join(format!("{gen_num}-link"));
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
pub async fn collect_garbage() -> Result<(), CollectGarbageError> {
    let output = tokio::process::Command::new("nix-collect-garbage")
        .output()
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

    fn generation_exists(profile_dir: &Path, number: usize) -> bool {
        profile_dir.join(format!("{number}-link")).exists()
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
            min_free_space: "0".into(),
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
            min_free_space: "0".into(),
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
            min_free_space: "0".into(),
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
            min_free_space: "0".into(),
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
            min_free_space: "0".into(),
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
            min_free_space: "0".into(),
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
            min_free_space: "0".into(),
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
            min_free_space: "0".into(),
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
            min_free_space: "0".into(),
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
}
