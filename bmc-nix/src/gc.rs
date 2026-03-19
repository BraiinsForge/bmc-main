// Copyright (C) 2025  Braiins Systems s.r.o.

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
/// - The current generation (pointed to by `current` symlink)
/// - Protected generations from `gc_config.protected_generations`
/// - The most recent `gc_config.keep_generations` generations
/// - Generations newer than `gc_config.keep_days` days (using directory mtime)
///
/// Removes everything else by deleting the generation directory.
///
/// **Note on `keep_days`:** Generation directories do not store creation
/// timestamps in the manifest. Directory `mtime` is used to determine age.
/// This is fragile but matches the approach used by Nix's own GC.
pub fn cleanup_generations(
    profile_dir: &Path,
    gc_config: &GcConfig,
) -> Result<(), CleanupGenerationsError> {
    if !profile_dir.exists() {
        return Ok(());
    }

    let entries = std::fs::read_dir(profile_dir).map_err(CleanupGenerationsError::Cleanup)?;

    // Collect all generation numbers
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

    // Build keep set
    let mut keep: HashSet<usize> = HashSet::new();

    // Keep current generation
    if let Some(current) = current_generation_number(profile_dir) {
        keep.insert(current);
    }

    // Keep protected generations
    for &gen_num in &gc_config.protected_generations {
        keep.insert(gen_num);
    }

    // Keep most recent N generations
    let keep_count = gc_config.keep_generations;
    for &gen_num in generations.iter().rev().take(keep_count) {
        keep.insert(gen_num);
    }

    // Keep generations newer than keep_days
    if gc_config.keep_days > 0 {
        let cutoff = SystemTime::now()
            - std::time::Duration::from_secs((gc_config.keep_days as u64) * 24 * 60 * 60);

        for &gen_num in &generations {
            let gen_dir = profile_dir.join(format!("{gen_num}-link"));
            if let Ok(metadata) = std::fs::metadata(&gen_dir) {
                if let Ok(mtime) = metadata.modified() {
                    if mtime > cutoff {
                        keep.insert(gen_num);
                    }
                }
            }
        }
    }

    // Remove generations not in keep set
    for &gen_num in &generations {
        if keep.contains(&gen_num) {
            continue;
        }

        let gen_dir = profile_dir.join(format!("{gen_num}-link"));
        info!(generation = gen_num, path = %gen_dir.display(), "removing old generation");
        if let Err(e) = std::fs::remove_dir_all(&gen_dir) {
            warn!(
                generation = gen_num,
                path = %gen_dir.display(),
                "failed to remove generation: {e}"
            );
        }
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
        return Err(CollectGarbageError::NixCommand(std::io::Error::new(
            std::io::ErrorKind::Other,
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
        // Remove existing symlink if present
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
            keep_days: 0,
            min_free_space: "0".into(),
            protected_generations: vec![1],
        };
        cleanup_generations(&profile_dir, &gc_config).expect("BUG: cleanup failed");

        // Generation 1 protected, 3 is current -> 2 removed
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
            keep_days: 0,
            min_free_space: "0".into(),
            protected_generations: vec![],
        };
        cleanup_generations(&profile_dir, &gc_config).expect("BUG: cleanup failed");

        // keep_generations=2 keeps 4 and 5, current=5 also kept
        assert!(!generation_exists(&profile_dir, 1));
        assert!(!generation_exists(&profile_dir, 2));
        assert!(!generation_exists(&profile_dir, 3));
        assert!(generation_exists(&profile_dir, 4));
        assert!(generation_exists(&profile_dir, 5));
    }

    #[test]
    fn cleanup_with_no_removable() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        create_generation(&profile_dir, 1);
        set_current(&profile_dir, 1);

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: 0,
            min_free_space: "0".into(),
            protected_generations: vec![1],
        };
        cleanup_generations(&profile_dir, &gc_config).expect("BUG: cleanup failed");

        assert!(generation_exists(&profile_dir, 1));
    }

    #[test]
    fn cleanup_empty_profile() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: 0,
            min_free_space: "0".into(),
            protected_generations: vec![],
        };
        // Should not error on empty profile
        cleanup_generations(&profile_dir, &gc_config).expect("BUG: cleanup failed");
    }

    #[test]
    fn cleanup_nonexistent_profile() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("nonexistent");

        let gc_config = GcConfig {
            keep_generations: 1,
            keep_days: 0,
            min_free_space: "0".into(),
            protected_generations: vec![],
        };
        cleanup_generations(&profile_dir, &gc_config).expect("BUG: cleanup failed");
    }

    #[test]
    fn cleanup_keeps_recent_by_days() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        // All generations are fresh (just created) -> all kept by keep_days
        for n in 1..=3 {
            create_generation(&profile_dir, n);
        }
        set_current(&profile_dir, 3);

        let gc_config = GcConfig {
            keep_generations: 0,
            keep_days: 30,
            min_free_space: "0".into(),
            protected_generations: vec![],
        };
        cleanup_generations(&profile_dir, &gc_config).expect("BUG: cleanup failed");

        // All should be kept because they're newer than 30 days
        assert!(generation_exists(&profile_dir, 1));
        assert!(generation_exists(&profile_dir, 2));
        assert!(generation_exists(&profile_dir, 3));
    }
}
