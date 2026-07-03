// Copyright (C) 2025  Braiins Systems s.r.o.

use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::profile::parse_generation_link_name;

/// An activation script discovered from `core/activation/scripts/`.
///
/// Scripts are executed in alphanumerical order by filename.
#[derive(Debug, Clone)]
pub struct ActivationScript {
    /// Name of the script (filename without path).
    pub name: String,
    /// Full path to the executable.
    pub path: PathBuf,
}

/// Errors that can occur during activation.
#[derive(Debug, thiserror::Error)]
pub enum ActivationError {
    #[error("failed to read activation directory '{path}': {source}")]
    ReadDir {
        path: String,
        source: std::io::Error,
    },
    #[error("activation entrypoint '{path}' failed with exit code {exit_code}")]
    EntrypointFailed { path: String, exit_code: i32 },
    #[error("activation entrypoint '{path}' was terminated by signal")]
    EntrypointSignaled { path: String },
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
    #[error("profile activation failed: {0}")]
    BuildProfile(#[source] Box<crate::profile::BuildProfileError>),
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
}

/// Error returned by [`GenerationSelector::from_str`].
#[derive(Debug, thiserror::Error)]
#[error("invalid generation selector '{0}': expected 'current', 'latest', or a positive integer")]
pub struct ParseSelectorError(String);

impl std::str::FromStr for GenerationSelector {
    type Err = ParseSelectorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "current" => Ok(Self::Current),
            "latest" => Ok(Self::Latest),
            other => other
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .map(Self::Number)
                .ok_or_else(|| ParseSelectorError(other.to_owned())),
        }
    }
}

/// Outcome of a successful [`activate`] or [`activate_next`] call.
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

/// Back up `current` as `previous` using a one-hop `readlink`.
///
/// Mirrors the shell activator's `backup_current_profile`: `previous`
/// ends up pointing at the same `<N>-link` name `current` had, so a
/// later restore reconstructs the same symlink form.
pub fn backup_current_to_previous(profile_dir: &Path) -> io::Result<()> {
    let current = profile_dir.join("current");
    let previous = profile_dir.join("previous");

    let target = std::fs::read_link(&current)?;

    match std::fs::remove_file(&previous) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    std::os::unix::fs::symlink(target, &previous)
}

/// Move `previous` back on top of `current`.
///
/// A single `rename(2)`: it atomically replaces an existing `current`,
/// so there is no window in which the profile has no `current` at all.
pub fn restore_previous_as_current(profile_dir: &Path) -> io::Result<()> {
    std::fs::rename(profile_dir.join("previous"), profile_dir.join("current"))
}

/// Remove the `next` symlink; `NotFound` is treated as success.
///
/// Distinct from [`crate::upgrade::remove_stale_next`], which invalidates
/// a pre-existing `next` before *building* a new profile on the upgrade
/// path. `remove_next` here consumes a `next` after a successful
/// boot-time activation.
pub fn remove_next(profile_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(profile_dir.join("next")) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Resolve `selector` to a generation and run its activation entrypoint.
///
/// Implements the "Default and explicit selectors" section of the design
/// spec: `Current` softly falls back to `find_latest_link` (both for a
/// missing symlink and for a missing/non-executable entrypoint) and only
/// returns [`ActivationOutcome::Skipped`] when nothing resolves; `Latest`
/// and `Number(N)` never fall back.
pub async fn activate(
    profile_dir: &Path,
    selector: GenerationSelector,
) -> Result<ActivationOutcome, ActivationError> {
    let Some(target) = resolve_selector(profile_dir, selector)? else {
        return Ok(ActivationOutcome::Skipped);
    };
    activate_resolved(profile_dir, selector, target).await
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
) -> Result<ActivationOutcome, ActivationError> {
    let generation = generation_number_from_link(&target).unwrap_or(0);

    let entrypoint = target.join("core/activation/entrypoint");
    if !is_executable(&entrypoint) {
        if matches!(selector, GenerationSelector::Current) {
            return current_fallback_to_latest(profile_dir, &target).await;
        }
        return Err(ActivationError::EntrypointNotFound {
            path: entrypoint.display().to_string(),
        });
    }

    crate::profile::activate_profile(profile_dir, generation, &target, None)
        .await
        .map_err(|err| ActivationError::BuildProfile(Box::new(err)))?;

    Ok(ActivationOutcome::Activated {
        generation,
        path: target,
    })
}

async fn current_fallback_to_latest(
    profile_dir: &Path,
    failed_target: &Path,
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

    let generation = generation_number_from_link(&latest).unwrap_or(0);
    let entrypoint = latest.join("core/activation/entrypoint");
    if !is_executable(&entrypoint) {
        return Err(ActivationError::EntrypointNotFound {
            path: entrypoint.display().to_string(),
        });
    }
    crate::profile::activate_profile(profile_dir, generation, &latest, None)
        .await
        .map_err(|err| ActivationError::BuildProfile(Box::new(err)))?;
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

/// Boot-time consume of a staged `next` profile.
///
/// If `next` is absent, delegates to [`activate`] with `Current`.
/// Otherwise: back up `current` as `previous` (if any), try `next`, on
/// success remove `next`, on failure restore `previous` (when we staged
/// one) and delegate to `Current` — which itself may fall back to the
/// latest generation.
pub async fn activate_next(profile_dir: &Path) -> Result<ActivationOutcome, ActivationError> {
    let next_path = profile_dir.join("next");
    if next_path.symlink_metadata().is_err() {
        return activate(profile_dir, GenerationSelector::Current).await;
    }

    let staged_previous = if profile_dir.join("current").symlink_metadata().is_ok() {
        backup_current_to_previous(profile_dir).map_err(io_to_activation(profile_dir))?;
        true
    } else {
        false
    };

    let next_target = std::fs::read_link(&next_path)
        .map(|t| {
            if t.is_absolute() {
                t
            } else {
                profile_dir.join(t)
            }
        })
        .map_err(io_to_activation(profile_dir))?;

    let generation = generation_number_from_link(&next_target).unwrap_or(0);
    let entrypoint = next_target.join("core/activation/entrypoint");
    let attempted = if is_executable(&entrypoint) {
        crate::profile::activate_profile(profile_dir, generation, &next_target, None)
            .await
            .map(|()| ActivationOutcome::Activated {
                generation,
                path: next_target.clone(),
            })
            .map_err(|err| ActivationError::BuildProfile(Box::new(err)))
    } else {
        Err(ActivationError::EntrypointNotFound {
            path: entrypoint.display().to_string(),
        })
    };

    match attempted {
        Ok(outcome) => {
            remove_next(profile_dir).map_err(io_to_activation(profile_dir))?;
            Ok(outcome)
        }
        Err(err) => {
            if staged_previous {
                restore_previous_as_current(profile_dir).map_err(io_to_activation(profile_dir))?;
                activate(profile_dir, GenerationSelector::Current).await
            } else {
                Err(err)
            }
        }
    }
}

/// Discover activation scripts from a generation directory.
///
/// Scans `gen_path/core/activation/scripts/` for executable files and
/// returns them sorted in alphanumerical order by filename.
///
/// Returns an empty vec (not an error) when the directory does not exist.
pub fn discover_activation_scripts(
    gen_path: &Path,
) -> Result<Vec<ActivationScript>, ActivationError> {
    let scripts_dir = gen_path.join("core/activation/scripts");

    if !scripts_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = std::fs::read_dir(&scripts_dir).map_err(|source| ActivationError::ReadDir {
        path: scripts_dir.display().to_string(),
        source,
    })?;

    let mut scripts = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|source| ActivationError::ReadDir {
            path: scripts_dir.display().to_string(),
            source,
        })?;

        let metadata = entry
            .metadata()
            .map_err(|source| ActivationError::ReadDir {
                path: scripts_dir.display().to_string(),
                source,
            })?;

        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            continue;
        }

        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| ActivationError::ReadDir {
                path: format!(
                    "non-UTF-8 filename in {}: {}",
                    scripts_dir.display(),
                    entry.file_name().display()
                ),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "filename is not valid UTF-8",
                ),
            })?
            .to_owned();

        scripts.push(ActivationScript {
            name,
            path: entry.path(),
        });
    }

    // Sort alphanumerically by name
    scripts.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(scripts)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn discover_activation_scripts_sorted() {
        let dir = tempfile::tempdir().expect("BUG: should create temp dir");
        let scripts_dir = dir.path().join("core/activation/scripts");
        std::fs::create_dir_all(&scripts_dir).expect("BUG: should create scripts dir");

        // Create scripts in non-alphabetical order
        for name in &["zzz-link-current", "50-write-boundary", "60-bmc-service"] {
            let script_path = scripts_dir.join(name);
            std::fs::write(&script_path, "#!/bin/sh\necho hello\n")
                .expect("BUG: should write script");
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .expect("BUG: should set permissions");
        }

        let scripts =
            discover_activation_scripts(dir.path()).expect("BUG: discovery should succeed");

        assert_eq!(scripts.len(), 3);
        let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["50-write-boundary", "60-bmc-service", "zzz-link-current"]
        );
    }

    #[test]
    fn discover_returns_empty_when_no_activation_dir() {
        let dir = tempfile::tempdir().expect("BUG: should create temp dir");

        let scripts = discover_activation_scripts(dir.path())
            .expect("BUG: missing dir should not be an error");

        assert!(
            scripts.is_empty(),
            "should return empty vec when core/activation/scripts/ does not exist"
        );
    }

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
    fn backup_current_uses_one_hop_readlink() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        touch_generation(dir.path(), 3);
        symlink("3-link", dir.path().join("current")).expect("BUG: current");

        backup_current_to_previous(dir.path()).expect("BUG: backup");

        let previous = std::fs::read_link(dir.path().join("previous")).expect("BUG: readlink");
        assert_eq!(previous, PathBuf::from("3-link"));
    }

    #[test]
    fn backup_current_replaces_stale_previous() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        touch_generation(dir.path(), 3);
        symlink("3-link", dir.path().join("current")).expect("BUG: current");
        symlink("stale", dir.path().join("previous")).expect("BUG: stale previous");

        backup_current_to_previous(dir.path()).expect("BUG: backup");
        let previous = std::fs::read_link(dir.path().join("previous")).expect("BUG: readlink");
        assert_eq!(previous, PathBuf::from("3-link"));
    }

    #[test]
    fn restore_previous_as_current_moves_symlink() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        touch_generation(dir.path(), 3);
        symlink("3-link", dir.path().join("previous")).expect("BUG: previous");
        symlink("failed", dir.path().join("current")).expect("BUG: current");

        restore_previous_as_current(dir.path()).expect("BUG: restore");

        assert!(dir.path().join("previous").symlink_metadata().is_err());
        let current = std::fs::read_link(dir.path().join("current")).expect("BUG: readlink");
        assert_eq!(current, PathBuf::from("3-link"));
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
            matches!(err, ActivationError::BuildProfile(_)),
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
            matches!(
                err,
                ActivationError::BuildProfile(_) | ActivationError::EntrypointNotFound { .. }
            ),
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
    async fn activate_next_no_next_delegates_to_current() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g = touch_generation(dir.path(), 1);
        write_entrypoint(&g, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");

        let out = activate_next(dir.path()).await.expect("BUG: activate_next");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 1, .. }
        ));
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_success_removes_next_and_backs_up_previous() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g1 = touch_generation(dir.path(), 1);
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g1, ZERO_EXIT);
        write_entrypoint(&g2, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");
        symlink("2-link", dir.path().join("next")).expect("BUG: next");

        let out = activate_next(dir.path()).await.expect("BUG: activate_next");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 2, .. }
        ));
        assert!(dir.path().join("next").symlink_metadata().is_err());
        let previous = std::fs::read_link(dir.path().join("previous")).expect("BUG: previous");
        assert_eq!(previous, PathBuf::from("1-link"));
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_failure_restores_previous_and_delegates_to_current() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g1 = touch_generation(dir.path(), 1);
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g1, ZERO_EXIT);
        write_entrypoint(&g2, NONZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");
        symlink("2-link", dir.path().join("next")).expect("BUG: next");

        let out = activate_next(dir.path()).await.expect("BUG: activate_next");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 1, .. }
        ));
        assert!(dir.path().join("next").symlink_metadata().is_ok());
        assert!(dir.path().join("previous").symlink_metadata().is_err());
        let current = std::fs::read_link(dir.path().join("current")).expect("BUG: current");
        assert_eq!(current, PathBuf::from("1-link"));
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_failure_no_previous_propagates_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g2, NONZERO_EXIT);
        symlink("2-link", dir.path().join("next")).expect("BUG: next");

        let err = activate_next(dir.path())
            .await
            .expect_err("BUG: expected error");
        assert!(
            matches!(err, ActivationError::BuildProfile(_)),
            "got {err:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_restored_previous_with_gc_entrypoint_cascades_to_latest() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        // 1-link has no entrypoint (simulates GC); 2-link fails; 3-link healthy.
        touch_generation(dir.path(), 1);
        let g2 = touch_generation(dir.path(), 2);
        let g3 = touch_generation(dir.path(), 3);
        write_entrypoint(&g2, NONZERO_EXIT);
        write_entrypoint(&g3, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");
        symlink("2-link", dir.path().join("next")).expect("BUG: next");

        let out = activate_next(dir.path()).await.expect("BUG: activate_next");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 3, .. }
        ));
    }
}
