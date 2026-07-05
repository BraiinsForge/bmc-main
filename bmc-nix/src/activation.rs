// Copyright (C) 2025  Braiins Systems s.r.o.

use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::profile::parse_generation_link_name;

/// Errors that can occur during activation.
#[derive(Debug, thiserror::Error)]
pub enum ActivationError {
    #[error("failed to read activation directory '{path}': {source}")]
    ReadDir {
        path: String,
        source: std::io::Error,
    },
    #[error("activation entrypoint '{path}' failed with exit code {exit_code}: {output}")]
    EntrypointFailed {
        path: String,
        exit_code: i32,
        output: String,
    },
    #[error("activation entrypoint '{path}' was terminated by signal: {output}")]
    EntrypointSignaled { path: String, output: String },
    #[error("failed to execute activation entrypoint '{path}': {source}")]
    EntrypointExecute {
        path: String,
        source: std::io::Error,
    },
    #[error("activation entrypoint not found at '{path}'")]
    EntrypointNotFound { path: String },
    #[error("no profile generation available at '{profile_dir}'")]
    NoGeneration { profile_dir: String },
    #[error("filesystem error while resolving generation '{path}': {source}")]
    ResolveIo {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("activation failed, reverted to generation {reverted_to}: {original}")]
    RevertedAfterFailure {
        original: Box<ActivationError>,
        reverted_to: usize,
    },
    #[error(
        "activation failed ({original}); revert to previous generation also failed: {revert_error}"
    )]
    RevertFailed {
        original: Box<ActivationError>,
        revert_error: Box<ActivationError>,
    },
    #[error("failed to acquire profile lock: {0}")]
    Lock(#[source] Box<crate::profile::BuildProfileError>),
}

/// Which profile generation to activate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationSelector {
    /// `<profile-dir>/current`, with a soft fallback to `find_latest_link`.
    Current,
    /// The largest `<N>-link` in the profile directory.
    Latest,
    /// A specific `<N>-link` (positive integer).
    Number(usize),
    /// The staged `<profile-dir>/next` symlink, consumed on success;
    /// falls back to `Current` when absent.
    Next,
}

/// Error returned by [`GenerationSelector::from_str`].
#[derive(Debug, thiserror::Error)]
#[error(
    "invalid generation selector '{0}': expected 'current', 'latest', 'next', or a positive integer"
)]
pub struct ParseSelectorError(String);

impl std::str::FromStr for GenerationSelector {
    type Err = ParseSelectorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "current" => Ok(Self::Current),
            "latest" => Ok(Self::Latest),
            "next" => Ok(Self::Next),
            other => other
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .map(Self::Number)
                .ok_or_else(|| ParseSelectorError(other.to_owned())),
        }
    }
}

/// Outcome of a successful [`activate`] call.
#[derive(Debug)]
pub enum ActivationOutcome {
    Activated { generation: usize, path: PathBuf },
    Skipped,
}

/// Return the `<profile-dir>/<N>-link` path with the largest `N >= 1`
/// whose target resolves to an existing directory.
///
/// Filters out non-numeric names, generation number 0, and dangling /
/// non-directory targets — matching the shell activator's guards.
pub fn find_latest_link(profile_dir: &Path) -> io::Result<Option<PathBuf>> {
    let entries = match std::fs::read_dir(profile_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };

    let mut best: Option<(usize, PathBuf)> = None;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let Some(n) = parse_generation_link_name(name_str) else {
            continue;
        };
        if n == 0 {
            continue;
        }
        let link_path = entry.path();
        match std::fs::metadata(&link_path) {
            Ok(meta) if meta.is_dir() => {}
            _ => continue,
        }
        match &best {
            Some((current_best, _)) if *current_best >= n => {}
            _ => best = Some((n, link_path)),
        }
    }
    Ok(best.map(|(_, p)| p))
}

/// Remove the `next` symlink; `NotFound` is treated as success.
///
/// Distinct from [`crate::upgrade::remove_stale_next`], which invalidates
/// a pre-existing `next` before *building* a new profile on the upgrade
/// path. `remove_next` here consumes a `next` after
/// `GenerationSelector::Next` activates it successfully.
pub fn remove_next(profile_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(profile_dir.join("next")) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Resolve `selector` to a generation and activate it via
/// [`crate::profile::activate_profile`] (which reverts to `current` on
/// failure).
///
/// `Current` softly falls back to `find_latest_link` (both for a
/// missing symlink and for a missing/non-executable entrypoint) and only
/// returns [`ActivationOutcome::Skipped`] when nothing resolves; `Latest`
/// and `Number(N)` never fall back. `Next` activates the staged `next`
/// symlink and removes it on success; when `next` is absent it behaves
/// exactly like `Current`.
///
/// The profile lock is acquired up front and held across selector
/// resolution, activation, and `next` removal, so a concurrent upgrade
/// cannot re-stage or observe a stale `next` mid-sequence.
pub async fn activate(
    profile_dir: &Path,
    selector: GenerationSelector,
) -> Result<ActivationOutcome, ActivationError> {
    let lock = crate::profile::lock_profile(profile_dir)
        .await
        .map_err(|err| ActivationError::Lock(Box::new(err)))?;
    let effective = match selector {
        GenerationSelector::Next => {
            let next = profile_dir.join("next");
            // `metadata` follows the symlink, so it fails when `next` is
            // absent or dangling; only a `next` that resolves to a real
            // generation is honored. A dangling marker (partial GC, manual
            // cleanup) must fall back to `current` instead of failing every
            // boot — the good `current` generation still boots.
            if std::fs::metadata(&next).is_ok() {
                GenerationSelector::Next
            } else {
                if next.symlink_metadata().is_ok() {
                    tracing::warn!(
                        "staged next generation {} is dangling; falling back to current",
                        next.display()
                    );
                }
                GenerationSelector::Current
            }
        }
        GenerationSelector::Current
        | GenerationSelector::Latest
        | GenerationSelector::Number(_) => selector,
    };
    let Some(target) = resolve_selector(profile_dir, effective)? else {
        return Ok(ActivationOutcome::Skipped);
    };
    let outcome = activate_resolved(profile_dir, effective, target, &lock).await?;
    if matches!(effective, GenerationSelector::Next) {
        remove_next(profile_dir).map_err(io_to_activation(profile_dir))?;
    }
    Ok(outcome)
}

fn resolve_selector(
    profile_dir: &Path,
    selector: GenerationSelector,
) -> Result<Option<PathBuf>, ActivationError> {
    match selector {
        GenerationSelector::Current => match resolve_current_link(profile_dir)? {
            Some(path) => Ok(Some(path)),
            None => find_latest_link(profile_dir).map_err(io_to_activation(profile_dir)),
        },
        GenerationSelector::Latest => {
            match find_latest_link(profile_dir).map_err(io_to_activation(profile_dir))? {
                Some(path) => Ok(Some(path)),
                None => Err(ActivationError::NoGeneration {
                    profile_dir: profile_dir.display().to_string(),
                }),
            }
        }
        GenerationSelector::Number(n) => {
            let path = profile_dir.join(format!("{n}-link"));
            match std::fs::metadata(&path) {
                Ok(meta) if meta.is_dir() => Ok(Some(path)),
                _ => Err(ActivationError::NoGeneration {
                    profile_dir: profile_dir.display().to_string(),
                }),
            }
        }
        GenerationSelector::Next => {
            let next = profile_dir.join("next");
            let target = std::fs::read_link(&next).map_err(io_to_activation(profile_dir))?;
            let absolute = if target.is_absolute() {
                target
            } else {
                profile_dir.join(target)
            };
            Ok(Some(absolute))
        }
    }
}

pub(crate) fn resolve_current_link(profile_dir: &Path) -> Result<Option<PathBuf>, ActivationError> {
    let current = profile_dir.join("current");
    match std::fs::read_link(&current) {
        Ok(target) => {
            let absolute = if target.is_absolute() {
                target
            } else {
                profile_dir.join(target)
            };
            match std::fs::metadata(&absolute) {
                Ok(meta) if meta.is_dir() => Ok(Some(absolute)),
                _ => Ok(None),
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(io_to_activation(profile_dir)(err)),
    }
}

pub(crate) fn io_to_activation(profile_dir: &Path) -> impl Fn(io::Error) -> ActivationError + '_ {
    move |source| ActivationError::ResolveIo {
        path: profile_dir.display().to_string(),
        source,
    }
}

async fn activate_resolved(
    profile_dir: &Path,
    selector: GenerationSelector,
    target: PathBuf,
    lock: &crate::profile::ProfileLock,
) -> Result<ActivationOutcome, ActivationError> {
    let generation =
        generation_number_from_link(&target).ok_or_else(|| ActivationError::NoGeneration {
            profile_dir: profile_dir.display().to_string(),
        })?;

    let entrypoint = target.join("core/activation/entrypoint");
    if !is_executable(&entrypoint) {
        if matches!(selector, GenerationSelector::Current) {
            return current_fallback_to_latest(profile_dir, &target, lock).await;
        }
        return Err(ActivationError::EntrypointNotFound {
            path: entrypoint.display().to_string(),
        });
    }

    crate::profile::activate_profile(profile_dir, generation, &target, Some(lock)).await?;

    Ok(ActivationOutcome::Activated {
        generation,
        path: target,
    })
}

async fn current_fallback_to_latest(
    profile_dir: &Path,
    failed_target: &Path,
    lock: &crate::profile::ProfileLock,
) -> Result<ActivationOutcome, ActivationError> {
    let Some(latest) = find_latest_link(profile_dir).map_err(io_to_activation(profile_dir))? else {
        return Err(ActivationError::EntrypointNotFound {
            path: failed_target
                .join("core/activation/entrypoint")
                .display()
                .to_string(),
        });
    };

    if canonicalize_pair(failed_target, &latest) {
        return Err(ActivationError::EntrypointNotFound {
            path: latest
                .join("core/activation/entrypoint")
                .display()
                .to_string(),
        });
    }

    let generation =
        generation_number_from_link(&latest).ok_or_else(|| ActivationError::NoGeneration {
            profile_dir: profile_dir.display().to_string(),
        })?;
    let entrypoint = latest.join("core/activation/entrypoint");
    if !is_executable(&entrypoint) {
        return Err(ActivationError::EntrypointNotFound {
            path: entrypoint.display().to_string(),
        });
    }
    crate::profile::activate_profile(profile_dir, generation, &latest, Some(lock)).await?;
    Ok(ActivationOutcome::Activated {
        generation,
        path: latest,
    })
}

pub(crate) fn canonicalize_pair(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ap), Ok(bp)) => ap == bp,
        _ => a == b,
    }
}

pub(crate) fn generation_number_from_link(link: &Path) -> Option<usize> {
    let name = link.file_name()?.to_str()?;
    parse_generation_link_name(name)
}

pub(crate) fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    use std::os::unix::fs::symlink;

    use serial_test::serial;

    fn touch_generation(profile_dir: &Path, n: usize) -> PathBuf {
        let path = profile_dir.join(format!("{n}-link"));
        std::fs::create_dir_all(&path).expect("BUG: mk gen dir");
        path
    }

    fn write_entrypoint(gen_path: &Path, script: &str) {
        let dir = gen_path.join("core/activation");
        std::fs::create_dir_all(&dir).expect("BUG: mk core/activation");
        let entrypoint = dir.join("entrypoint");
        std::fs::write(&entrypoint, script).expect("BUG: write entrypoint");
        std::fs::set_permissions(&entrypoint, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: chmod entrypoint");
    }

    const ZERO_EXIT: &str = "#!/bin/sh\nexit 0\n";
    const NONZERO_EXIT: &str = "#!/bin/sh\nexit 42\n";

    #[test]
    fn selector_from_str_current() {
        assert!(matches!(
            "current".parse::<GenerationSelector>(),
            Ok(GenerationSelector::Current)
        ));
    }

    #[test]
    fn selector_from_str_latest() {
        assert!(matches!(
            "latest".parse::<GenerationSelector>(),
            Ok(GenerationSelector::Latest)
        ));
    }

    #[test]
    fn selector_from_str_positive_number() {
        assert!(matches!(
            "5".parse::<GenerationSelector>(),
            Ok(GenerationSelector::Number(5))
        ));
    }

    #[test]
    fn selector_from_str_zero_is_rejected() {
        assert!("0".parse::<GenerationSelector>().is_err());
    }

    #[test]
    fn selector_from_str_negative_is_rejected() {
        assert!("-1".parse::<GenerationSelector>().is_err());
    }

    #[test]
    fn selector_from_str_garbage_is_rejected() {
        assert!("foo".parse::<GenerationSelector>().is_err());
    }

    #[test]
    fn find_latest_link_empty_dir() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        assert!(find_latest_link(dir.path()).expect("BUG: find").is_none());
    }

    #[test]
    fn find_latest_link_returns_largest_valid() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        touch_generation(dir.path(), 1);
        touch_generation(dir.path(), 5);
        touch_generation(dir.path(), 3);
        let latest = find_latest_link(dir.path())
            .expect("BUG: find")
            .expect("BUG: newest generation link must be present");
        assert_eq!(latest, dir.path().join("5-link"));
    }

    #[test]
    fn find_latest_link_skips_dangling() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        touch_generation(dir.path(), 2);
        symlink("does-not-exist", dir.path().join("9-link")).expect("BUG: symlink");
        let latest = find_latest_link(dir.path())
            .expect("BUG: find")
            .expect("BUG: newest generation link must be present");
        assert_eq!(latest, dir.path().join("2-link"));
    }

    #[test]
    fn find_latest_link_skips_non_numeric_and_non_dir() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        touch_generation(dir.path(), 1);
        std::fs::write(dir.path().join("latest-link"), b"garbage").expect("BUG: write");
        std::fs::write(dir.path().join("2-link"), b"not a dir").expect("BUG: write");
        let latest = find_latest_link(dir.path())
            .expect("BUG: find")
            .expect("BUG: newest generation link must be present");
        assert_eq!(latest, dir.path().join("1-link"));
    }

    #[test]
    fn remove_next_is_idempotent_when_absent() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        remove_next(dir.path()).expect("BUG: remove_next on absent");
    }

    #[test]
    fn remove_next_deletes_symlink() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        touch_generation(dir.path(), 2);
        symlink("2-link", dir.path().join("next")).expect("BUG: next");
        remove_next(dir.path()).expect("BUG: remove_next");
        assert!(dir.path().join("next").symlink_metadata().is_err());
    }

    #[tokio::test]
    #[serial]
    async fn activate_current_no_current_and_no_generations_is_skipped() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let out = activate(dir.path(), GenerationSelector::Current)
            .await
            .expect("BUG: soft skip");
        assert!(matches!(out, ActivationOutcome::Skipped));
    }

    #[tokio::test]
    #[serial]
    async fn activate_current_missing_current_falls_back_to_latest() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g = touch_generation(dir.path(), 4);
        write_entrypoint(&g, ZERO_EXIT);
        let out = activate(dir.path(), GenerationSelector::Current)
            .await
            .expect("BUG: activate");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 4, .. }
        ));
    }

    #[tokio::test]
    #[serial]
    async fn activate_current_entrypoint_exit_nonzero_is_hard_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g = touch_generation(dir.path(), 1);
        write_entrypoint(&g, NONZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");

        let err = activate(dir.path(), GenerationSelector::Current)
            .await
            .expect_err("BUG: expected error");
        assert!(
            matches!(err, ActivationError::EntrypointFailed { .. }),
            "got {err:?}"
        );
        assert!(dir.path().join("current").symlink_metadata().is_ok());
    }

    #[tokio::test]
    #[serial]
    async fn activate_current_missing_entrypoint_falls_back_to_latest() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        touch_generation(dir.path(), 1);
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g2, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");

        let out = activate(dir.path(), GenerationSelector::Current)
            .await
            .expect("BUG: activate");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 2, .. }
        ));
    }

    #[tokio::test]
    #[serial]
    async fn activate_current_latest_is_same_broken_profile_returns_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        touch_generation(dir.path(), 1);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");

        let err = activate(dir.path(), GenerationSelector::Current)
            .await
            .expect_err("BUG: expected error");
        assert!(
            matches!(err, ActivationError::EntrypointNotFound { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_latest_missing_is_hard_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let err = activate(dir.path(), GenerationSelector::Latest)
            .await
            .expect_err("BUG: expected error");
        assert!(matches!(err, ActivationError::NoGeneration { .. }));
    }

    #[tokio::test]
    #[serial]
    async fn activate_number_missing_is_hard_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let err = activate(dir.path(), GenerationSelector::Number(7))
            .await
            .expect_err("BUG: expected error");
        assert!(matches!(err, ActivationError::NoGeneration { .. }));
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_selector_no_next_delegates_to_current() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g = touch_generation(dir.path(), 1);
        write_entrypoint(&g, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");

        let out = activate(dir.path(), GenerationSelector::Next)
            .await
            .expect("BUG: activate next");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 1, .. }
        ));
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_selector_dangling_next_falls_back_to_current() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g = touch_generation(dir.path(), 1);
        write_entrypoint(&g, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");
        // `next` points at a generation directory that does not exist.
        symlink("9-link", dir.path().join("next")).expect("BUG: next");

        let out = activate(dir.path(), GenerationSelector::Next)
            .await
            .expect("BUG: a dangling next must fall back to current, not error");
        assert!(
            matches!(out, ActivationOutcome::Activated { generation: 1, .. }),
            "got {out:?}"
        );
        // The dangling marker is left in place for later cleanup, not removed.
        assert!(dir.path().join("next").symlink_metadata().is_ok());
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_non_generation_target_is_hard_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        // `next` resolves to a real directory whose name is not `<N>-link`,
        // so no generation number can be parsed from it.
        let weird = dir.path().join("weird");
        std::fs::create_dir_all(&weird).expect("BUG: mk weird dir");
        write_entrypoint(&weird, ZERO_EXIT);
        symlink("weird", dir.path().join("next")).expect("BUG: next");

        let err = activate(dir.path(), GenerationSelector::Next)
            .await
            .expect_err("BUG: a next target that is not <N>-link must be a hard error");
        assert!(
            matches!(err, ActivationError::NoGeneration { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_selector_success_removes_next() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g1 = touch_generation(dir.path(), 1);
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g1, ZERO_EXIT);
        write_entrypoint(&g2, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");
        symlink("2-link", dir.path().join("next")).expect("BUG: next");

        let out = activate(dir.path(), GenerationSelector::Next)
            .await
            .expect("BUG: activate next");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 2, .. }
        ));
        assert!(dir.path().join("next").symlink_metadata().is_err());
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_selector_failure_reverts_and_keeps_next() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g1 = touch_generation(dir.path(), 1);
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g1, ZERO_EXIT);
        write_entrypoint(&g2, NONZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");
        symlink("2-link", dir.path().join("next")).expect("BUG: next");

        let err = activate(dir.path(), GenerationSelector::Next)
            .await
            .expect_err("BUG: expected revert error");
        assert!(
            matches!(
                err,
                ActivationError::RevertedAfterFailure { reverted_to: 1, .. }
            ),
            "got {err:?}"
        );
        assert!(
            dir.path().join("next").symlink_metadata().is_ok(),
            "failed next should stay put for inspection"
        );
        assert!(dir.path().join("previous").symlink_metadata().is_err());
        let current = std::fs::read_link(dir.path().join("current")).expect("BUG: current");
        assert_eq!(current, PathBuf::from("1-link"));
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_selector_failure_no_current_propagates_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g2, NONZERO_EXIT);
        symlink("2-link", dir.path().join("next")).expect("BUG: next");

        let err = activate(dir.path(), GenerationSelector::Next)
            .await
            .expect_err("BUG: expected error");
        assert!(
            matches!(err, ActivationError::EntrypointFailed { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_selector_revert_onto_gc_entrypoint_is_revert_failed() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        // 1-link has no entrypoint (simulates GC); 2-link fails.
        touch_generation(dir.path(), 1);
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g2, NONZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");
        symlink("2-link", dir.path().join("next")).expect("BUG: next");

        let err = activate(dir.path(), GenerationSelector::Next)
            .await
            .expect_err("BUG: expected revert failure");
        assert!(
            matches!(err, ActivationError::RevertFailed { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_holds_profile_lock_and_leaves_lock_file() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g = touch_generation(dir.path(), 1);
        write_entrypoint(&g, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");

        let out = activate(dir.path(), GenerationSelector::Current)
            .await
            .expect("BUG: activate");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 1, .. }
        ));
        assert!(
            dir.path().join(".lock").exists(),
            "activate should create the profile lock file"
        );
    }

    #[test]
    fn selector_from_str_next() {
        assert!(matches!(
            "next".parse::<GenerationSelector>(),
            Ok(GenerationSelector::Next)
        ));
    }
}
