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

use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
    #[error("failed to timestamp invalid activation marker '{path}': {source}")]
    QuarantineTimestamp {
        path: String,
        #[source]
        source: std::time::SystemTimeError,
    },
    #[error("failed to quarantine invalid activation marker '{path}' as '{quarantine}': {source}")]
    Quarantine {
        path: String,
        quarantine: String,
        #[source]
        source: io::Error,
    },
    #[error("generation activated, but consuming its marker '{path}' failed: {source}")]
    ConsumeMarker {
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
    /// `<profile-dir>/current`, quarantining a non-symlink obstruction and
    /// falling back to `find_latest_link` when no valid link remains.
    Current,
    /// The largest `<N>-link` in the profile directory.
    Latest,
    /// A specific `<N>-link` (positive integer).
    Number(usize),
    /// The staged `<profile-dir>/next.<bos-version>` symlink for the
    /// running firmware, consumed on success; falls back to `Current`
    /// when absent.
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

/// Name of the deferred-activation marker staged for `bos_version`.
///
/// The version is part of the file name, so an activator only ever
/// finds the marker staged for the firmware it runs on. Packages staged
/// for another firmware — e.g. by a sysupgrade that failed before its
/// reboot — are invisible to it and swept as stale instead.
#[must_use]
pub fn next_marker_name(bos_version: &str) -> String {
    format!("next.{bos_version}")
}

/// Whether `name` is a deferred-activation marker: `next.<version>` or
/// a bare `next`.
pub(crate) fn is_next_marker_name(name: &str) -> bool {
    name == "next" || name.starts_with("next.")
}

/// Remove every deferred-activation marker in `profile_dir` except the
/// one named `keep`; a missing directory or entry is treated as done.
pub(crate) fn sweep_next_markers(profile_dir: &Path, keep: Option<&str>) -> io::Result<()> {
    let entries = match std::fs::read_dir(profile_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if !is_next_marker_name(name_str) || Some(name_str) == keep {
            continue;
        }
        // Only symlinks are markers (mirrors the shell activator's
        // `[ -L ]` guard): other entries must not fail boot activation.
        if !entry.file_type()?.is_symlink() {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// Remove the marker staged for `bos_version`; `NotFound` is treated as
/// success.
///
/// Distinct from [`crate::upgrade::remove_stale_next`], which invalidates
/// pre-existing markers before *building* a new profile on the upgrade
/// path. `remove_next` here consumes a marker after
/// `GenerationSelector::Next` activates it successfully.
/// The disappearance is fsynced so a crash after a successful boot activation
/// cannot resurrect a consumed marker.
pub fn remove_next(profile_dir: &Path, bos_version: &str) -> io::Result<()> {
    match std::fs::remove_file(profile_dir.join(next_marker_name(bos_version))) {
        Ok(()) => crate::fs_sync::fsync_dir(profile_dir),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Resolve `selector` to a generation and activate it via
/// [`crate::profile::activate_profile`] (which reverts to `current` on
/// failure).
///
/// `Current` quarantines a non-symlink marker and softly falls back to
/// `find_latest_link` (both for a missing symlink and for a
/// missing/non-executable entrypoint). It only returns
/// [`ActivationOutcome::Skipped`] when nothing resolves; `Latest` and
/// `Number(N)` never fall back. `Next` activates the
/// `next.<bos-version>` marker matching `bos_version` (the running
/// firmware's) and removes it on success; markers staged for other
/// versions are removed as stale. When no marker for this version
/// exists — including when `bos_version` is `None` — it behaves exactly
/// like `Current` (an unknown version leaves all markers untouched).
///
/// The profile lock is acquired up front and held across selector
/// resolution, activation, and marker removal, so a concurrent upgrade
/// cannot re-stage or observe a stale marker mid-sequence.
pub async fn activate(
    profile_dir: &Path,
    selector: GenerationSelector,
    bos_version: Option<&str>,
) -> Result<ActivationOutcome, ActivationError> {
    let lock = crate::profile::lock_profile(profile_dir)
        .await
        .map_err(|err| ActivationError::Lock(Box::new(err)))?;
    let next_marker = match (selector, bos_version) {
        (GenerationSelector::Next, Some(version)) => Some(next_marker_name(version)),
        // Without a known BOS version no marker can be matched: leave
        // all markers alone (staleness is undecidable) and boot current.
        (GenerationSelector::Next, None) => {
            tracing::warn!("BOS version unknown; ignoring any staged next generation");
            None
        }
        _ => None,
    };
    let effective = match selector {
        GenerationSelector::Next => match next_marker.as_deref() {
            None => GenerationSelector::Current,
            Some(marker) => {
                sweep_next_markers(profile_dir, Some(marker))
                    .map_err(io_to_activation(profile_dir))?;
                let next = profile_dir.join(marker);
                if matches!(
                    quarantine_invalid_marker(profile_dir, marker)?,
                    MarkerPresence::Symlink
                ) {
                    // `metadata` follows the symlink, so `NotFound` covers a
                    // concurrently removed or dangling marker. A dangling
                    // marker (partial GC, manual cleanup) must fall back to
                    // `current` instead of failing every boot — the good
                    // `current` generation still boots. Any other error
                    // (EACCES, ELOOP, …) says nothing about staleness and must
                    // surface rather than silently skip a genuinely staged
                    // generation.
                    match std::fs::metadata(&next) {
                        Ok(_) => GenerationSelector::Next,
                        Err(err) if err.kind() == io::ErrorKind::NotFound => {
                            if next.symlink_metadata().is_ok() {
                                tracing::warn!(
                                    "staged next generation {} is dangling; falling back to current",
                                    next.display()
                                );
                            }
                            GenerationSelector::Current
                        }
                        Err(err) => return Err(io_to_activation(profile_dir)(err)),
                    }
                } else {
                    GenerationSelector::Current
                }
            }
        },
        GenerationSelector::Current
        | GenerationSelector::Latest
        | GenerationSelector::Number(_) => selector,
    };
    let Some(target) = resolve_selector(profile_dir, effective, next_marker.as_deref())? else {
        return Ok(ActivationOutcome::Skipped);
    };
    let outcome = activate_resolved(profile_dir, effective, target, &lock).await?;
    if matches!(effective, GenerationSelector::Next) {
        let version = bos_version.expect("BUG: Next is only effective with a version");
        remove_next(profile_dir, version).map_err(|source| ActivationError::ConsumeMarker {
            path: profile_dir
                .join(next_marker_name(version))
                .display()
                .to_string(),
            source,
        })?;
    }
    Ok(outcome)
}

fn resolve_selector(
    profile_dir: &Path,
    selector: GenerationSelector,
    next_marker: Option<&str>,
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
            let next = profile_dir.join(next_marker.expect("BUG: Next resolved without a marker"));
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
    if matches!(
        quarantine_invalid_marker(profile_dir, "current")?,
        MarkerPresence::Missing
    ) {
        return Ok(None);
    }
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

enum MarkerPresence {
    Missing,
    Symlink,
}

fn quarantine_invalid_marker(
    profile_dir: &Path,
    marker_name: &str,
) -> Result<MarkerPresence, ActivationError> {
    let marker = profile_dir.join(marker_name);
    let metadata = match marker.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(MarkerPresence::Missing),
        Err(err) => return Err(io_to_activation(&marker)(err)),
    };
    if metadata.file_type().is_symlink() {
        return Ok(MarkerPresence::Symlink);
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| ActivationError::QuarantineTimestamp {
            path: marker.display().to_string(),
            source,
        })?
        .as_nanos();
    let quarantine_stem = format!("{marker_name}.invalid.{timestamp}");
    let quarantine = unique_quarantine_path(profile_dir, &quarantine_stem)?;
    std::fs::rename(&marker, &quarantine).map_err(|source| ActivationError::Quarantine {
        path: marker.display().to_string(),
        quarantine: quarantine.display().to_string(),
        source,
    })?;
    crate::fs_sync::fsync_dir(profile_dir).map_err(|source| ActivationError::Quarantine {
        path: marker.display().to_string(),
        quarantine: quarantine.display().to_string(),
        source,
    })?;
    tracing::warn!(
        marker = %marker.display(),
        quarantine = %quarantine.display(),
        "quarantined invalid activation marker"
    );
    Ok(MarkerPresence::Missing)
}

fn unique_quarantine_path(
    profile_dir: &Path,
    quarantine_stem: &str,
) -> Result<PathBuf, ActivationError> {
    for suffix in 0_usize.. {
        let file_name = if suffix == 0 {
            quarantine_stem.to_owned()
        } else {
            format!("{quarantine_stem}.{suffix}")
        };
        let candidate = profile_dir.join(file_name);
        match candidate.symlink_metadata() {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => {}
            Err(err) => return Err(io_to_activation(&candidate)(err)),
        }
    }
    unreachable!("usize quarantine suffix space cannot be exhausted")
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
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
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
    fn unique_quarantine_path_appends_suffix_after_collisions() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        std::fs::write(dir.path().join("current.invalid.123"), b"first")
            .expect("BUG: write first quarantine");
        std::fs::write(dir.path().join("current.invalid.123.1"), b"second")
            .expect("BUG: write second quarantine");

        assert_eq!(
            unique_quarantine_path(dir.path(), "current.invalid.123")
                .expect("BUG: choose quarantine path"),
            dir.path().join("current.invalid.123.2")
        );
    }

    #[test]
    fn remove_next_is_idempotent_when_absent() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        remove_next(dir.path(), "1.0").expect("BUG: remove_next on absent");
    }

    #[test]
    fn remove_next_deletes_symlink() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        touch_generation(dir.path(), 2);
        symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next");
        remove_next(dir.path(), "1.0").expect("BUG: remove_next");
        assert!(dir.path().join("next.1.0").symlink_metadata().is_err());
    }

    #[tokio::test]
    #[serial]
    async fn activate_current_no_current_and_no_generations_is_skipped() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let out = activate(dir.path(), GenerationSelector::Current, None)
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
        let out = activate(dir.path(), GenerationSelector::Current, None)
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

        let err = activate(dir.path(), GenerationSelector::Current, None)
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

        let out = activate(dir.path(), GenerationSelector::Current, None)
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

        let err = activate(dir.path(), GenerationSelector::Current, None)
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
        let err = activate(dir.path(), GenerationSelector::Latest, None)
            .await
            .expect_err("BUG: expected error");
        assert!(matches!(err, ActivationError::NoGeneration { .. }));
    }

    #[tokio::test]
    #[serial]
    async fn activate_number_missing_is_hard_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let err = activate(dir.path(), GenerationSelector::Number(7), None)
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

        let out = activate(dir.path(), GenerationSelector::Next, Some("1.0"))
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
        symlink("9-link", dir.path().join("next.1.0")).expect("BUG: next");

        let out = activate(dir.path(), GenerationSelector::Next, Some("1.0"))
            .await
            .expect("BUG: a dangling next must fall back to current, not error");
        assert!(
            matches!(out, ActivationOutcome::Activated { generation: 1, .. }),
            "got {out:?}"
        );
        // The dangling marker is left in place for later cleanup, not removed.
        assert!(dir.path().join("next.1.0").symlink_metadata().is_ok());
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
        symlink("weird", dir.path().join("next.1.0")).expect("BUG: next");

        let err = activate(dir.path(), GenerationSelector::Next, Some("1.0"))
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
        symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next");

        let out = activate(dir.path(), GenerationSelector::Next, Some("1.0"))
            .await
            .expect("BUG: activate next");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 2, .. }
        ));
        assert!(dir.path().join("next.1.0").symlink_metadata().is_err());
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_sweeps_markers_staged_for_other_versions() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g1 = touch_generation(dir.path(), 1);
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g1, ZERO_EXIT);
        write_entrypoint(&g2, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");
        symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next.1.0");
        // Markers left behind by a sysupgrade that never rebooted into
        // its firmware, and by pre-versioning staging.
        symlink("2-link", dir.path().join("next.2.0")).expect("BUG: next.2.0");
        symlink("2-link", dir.path().join("next")).expect("BUG: bare next");

        let out = activate(dir.path(), GenerationSelector::Next, Some("1.0"))
            .await
            .expect("BUG: activate next");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 2, .. }
        ));
        assert!(dir.path().join("next.2.0").symlink_metadata().is_err());
        assert!(dir.path().join("next").symlink_metadata().is_err());
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_sweep_skips_non_symlink_entries() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g1 = touch_generation(dir.path(), 1);
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g1, ZERO_EXIT);
        write_entrypoint(&g2, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");
        symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next.1.0");
        // A directory squatting on a marker name must not fail boot
        // activation; only symlinks are markers.
        std::fs::create_dir(dir.path().join("next.junk")).expect("BUG: mk next.junk");

        let out = activate(dir.path(), GenerationSelector::Next, Some("1.0"))
            .await
            .expect("BUG: junk in the profile dir must not fail activation");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 2, .. }
        ));
        assert!(dir.path().join("next.junk").is_dir());
    }

    #[tokio::test]
    #[serial]
    async fn activate_next_without_version_activates_current_and_keeps_markers() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g1 = touch_generation(dir.path(), 1);
        let g2 = touch_generation(dir.path(), 2);
        write_entrypoint(&g1, ZERO_EXIT);
        write_entrypoint(&g2, ZERO_EXIT);
        symlink("1-link", dir.path().join("current")).expect("BUG: current");
        symlink("2-link", dir.path().join("next.2.0")).expect("BUG: next.2.0");

        // Without a version, staleness is undecidable: no marker may be
        // consumed or swept, and activation falls back to current.
        let out = activate(dir.path(), GenerationSelector::Next, None)
            .await
            .expect("BUG: activate next without version");
        assert!(matches!(
            out,
            ActivationOutcome::Activated { generation: 1, .. }
        ));
        assert!(dir.path().join("next.2.0").symlink_metadata().is_ok());
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
        symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next");

        let err = activate(dir.path(), GenerationSelector::Next, Some("1.0"))
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
            dir.path().join("next.1.0").symlink_metadata().is_ok(),
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
        symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next");

        let err = activate(dir.path(), GenerationSelector::Next, Some("1.0"))
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
        symlink("2-link", dir.path().join("next.1.0")).expect("BUG: next");

        let err = activate(dir.path(), GenerationSelector::Next, Some("1.0"))
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

        let out = activate(dir.path(), GenerationSelector::Current, None)
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
