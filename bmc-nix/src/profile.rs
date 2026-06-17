// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::HashMap;
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::io::AsRawFd as _;
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::types::{ProfileGeneration, ResolvedPackage};

/// Errors that can occur when building or managing profiles.
#[derive(Debug, thiserror::Error)]
pub enum BuildProfileError {
    #[error("symlink conflict at '{path}': provided by '{pkg_a}' and '{pkg_b}'")]
    Conflict {
        path: String,
        pkg_a: String,
        pkg_b: String,
    },
    #[error("failed to create directory '{path}': {source}")]
    CreateDir {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to remove directory '{path}': {source}")]
    RemoveDir {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to create symlink '{path}': {source}")]
    CreateSymlink {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to read store directory '{path}': {source}")]
    ReadStorePath {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to stat store entry '{path}': {source}")]
    StatStorePath {
        path: String,
        source: std::io::Error,
    },
    #[error("symlink cycle detected at '{path}'")]
    SymlinkCycle { path: String },
    #[error("hook execution failed: {0}")]
    Hooks(#[from] crate::hooks::RunHooksError),
    #[error("manifest error: {0}")]
    Manifest(#[from] crate::manifest::WriteManifestError),
    #[error("failed to rename generation: {source}")]
    Rename { source: std::io::Error },
    #[error("failed to read profile directory: {source}")]
    ReadDir { source: std::io::Error },
    #[error("activation failed: {0}")]
    Activation(#[from] crate::activation::ActivationError),
    #[error("failed to acquire profile lock: {source}")]
    Lock { source: std::io::Error },
}

/// Name of the symlink in a profile directory that points to the
/// active generation.
pub(crate) const CURRENT_LINK_NAME: &str = "current";

/// Name of the generation subdirectory for generation `n`.
#[must_use]
pub(crate) fn generation_link_name(n: usize) -> String {
    format!("{n}-link")
}

/// Parse a generation number out of a directory name matching `<N>-link`.
///
/// `pub(crate)` so `upgrade::resolve_current_generation` can reuse it
/// instead of duplicating the strip/parse dance inline.
#[must_use]
pub(crate) fn parse_generation_link_name(name: &str) -> Option<usize> {
    name.strip_suffix("-link")?.parse::<usize>().ok()
}

/// RAII guard that holds an exclusive `flock` on a profile directory.
///
/// The lock is explicitly released via `LOCK_UN` on drop, then the file
/// descriptor is closed. This avoids a race where `close()` alone may
/// not release the lock atomically with respect to concurrent openers.
#[derive(Debug)]
pub struct ProfileLock {
    file: std::fs::File,
}

impl Drop for ProfileLock {
    fn drop(&mut self) {
        // Explicitly unlock before close — ignore errors since we're in Drop
        let _ = flock(&self.file, libc::LOCK_UN);
    }
}

/// Open and prepare the lock file, returning the file handle.
fn open_lock_file(profile_dir: &Path) -> Result<std::fs::File, BuildProfileError> {
    std::fs::create_dir_all(profile_dir).map_err(|source| BuildProfileError::CreateDir {
        path: profile_dir.display().to_string(),
        source,
    })?;

    let lock_path = profile_dir.join(".lock");
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| BuildProfileError::Lock { source })
}

/// Call `flock(2)` with the given flags.
///
/// # Safety
///
/// `file` must be an open file with a valid file descriptor.
fn flock(file: &std::fs::File, flags: libc::c_int) -> Result<(), std::io::Error> {
    // SAFETY: `file.as_raw_fd()` returns a valid, open file descriptor owned
    // by `file`. The fd remains valid for the duration of the call because
    // `file` is borrowed, preventing it from being dropped.
    let ret = unsafe { libc::flock(file.as_raw_fd(), flags) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Acquire an exclusive lock on a profile directory.
///
/// Creates `<profile_dir>/.lock` and holds an exclusive `flock(2)` on it.
/// The blocking `flock` call is offloaded to a blocking thread via
/// [`tokio::task::spawn_blocking`] so it does not stall the async runtime.
/// The lock is released when the returned [`ProfileLock`] is dropped.
pub async fn lock_profile(profile_dir: &Path) -> Result<ProfileLock, BuildProfileError> {
    let file = open_lock_file(profile_dir)?;

    tokio::task::spawn_blocking(move || {
        flock(&file, libc::LOCK_EX).map_err(|source| BuildProfileError::Lock { source })?;
        Ok(ProfileLock { file })
    })
    .await
    .expect("BUG: lock task should not panic")
}

/// Try to acquire an exclusive lock on a profile directory without blocking.
///
/// Returns `Ok(Some(lock))` if acquired, `Ok(None)` if another process holds it.
pub fn try_lock_profile(profile_dir: &Path) -> Result<Option<ProfileLock>, BuildProfileError> {
    let file = open_lock_file(profile_dir)?;

    match flock(&file, libc::LOCK_EX | libc::LOCK_NB) {
        Ok(()) => Ok(Some(ProfileLock { file })),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(source) => Err(BuildProfileError::Lock { source }),
    }
}

/// Try to acquire an exclusive lock on a profile directory within a timeout.
///
/// Returns `Ok(Some(lock))` if acquired within `timeout`, `Ok(None)` if the
/// timeout elapsed while another process held the lock.
///
/// Uses a blocking `flock(LOCK_EX)` on a [`tokio::task::spawn_blocking`]
/// thread and races it against [`tokio::time::timeout`]. On timeout, the
/// blocking thread remains parked inside `flock` until the other holder
/// releases; timing out this function does not cancel the in-flight
/// `flock(LOCK_EX)` call. Once the other holder releases, the background task
/// acquires the lock, constructs the resulting [`ProfileLock`], and
/// immediately drops it, releasing our fd again.
///
/// This keeps the implementation free of polling, but a timed-out waiter may
/// still briefly acquire and release the lock later.
pub async fn lock_profile_with_timeout(
    profile_dir: &Path,
    timeout: std::time::Duration,
) -> Result<Option<ProfileLock>, BuildProfileError> {
    let file = open_lock_file(profile_dir)?;

    let handle = tokio::task::spawn_blocking(move || {
        flock(&file, libc::LOCK_EX).map_err(|source| BuildProfileError::Lock { source })?;
        Ok::<ProfileLock, BuildProfileError>(ProfileLock { file })
    });

    match tokio::time::timeout(timeout, handle).await {
        Ok(join_result) => {
            let lock = join_result.expect("BUG: lock task should not panic")?;
            Ok(Some(lock))
        }
        Err(_elapsed) => Ok(None),
    }
}

/// Read all entries in `dir` and return them sorted lexicographically by name.
fn sorted_dir_entries(dir: &Path) -> Result<Vec<std::fs::DirEntry>, std::io::Error> {
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    Ok(entries)
}

/// Recursive descent worker for `build_symlink_tree`.
///
/// `src_dir` is the directory being descended in the store. `dst_dir` is the
/// corresponding real directory already created in the output tree.
/// `ancestors` tracks `(dev, ino)` of directories on the current call stack
/// to detect cycles introduced by directory symlinks.
fn descend_dir(
    src_dir: &Path,
    dst_dir: &Path,
    pkg_name: &str,
    rel_prefix: &Path,
    ownership: &mut HashMap<PathBuf, String>,
    ancestors: &mut HashSet<(u64, u64)>,
) -> Result<(), BuildProfileError> {
    let entries =
        sorted_dir_entries(src_dir).map_err(|source| BuildProfileError::ReadStorePath {
            path: src_dir.display().to_string(),
            source,
        })?;

    for entry in entries {
        let entry_name = entry.file_name();
        let src_path = src_dir.join(&entry_name);
        let dst_path = dst_dir.join(&entry_name);
        let rel_path = rel_prefix.join(&entry_name);

        let lstat = std::fs::symlink_metadata(&src_path).map_err(|source| {
            BuildProfileError::StatStorePath {
                path: src_path.display().to_string(),
                source,
            }
        })?;

        // For a symlink-to-dir, follow it once to get the resolved metadata
        // (used both to determine is_dir and for the cycle-identity dev/ino).
        // For a plain dir, lstat already carries the identity.
        let (is_dir, resolved_meta) = if lstat.is_symlink() {
            match std::fs::metadata(&src_path) {
                Ok(m) => {
                    let is_d = m.is_dir();
                    (is_d, Some(m))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Dangling symlink — treat as leaf
                    (false, None)
                }
                Err(source) => {
                    return Err(BuildProfileError::StatStorePath {
                        path: src_path.display().to_string(),
                        source,
                    });
                }
            }
        } else {
            (lstat.is_dir(), None)
        };

        if is_dir {
            let stat = resolved_meta.as_ref().unwrap_or(&lstat);
            let identity = (stat.dev(), stat.ino());
            if ancestors.contains(&identity) {
                return Err(BuildProfileError::SymlinkCycle {
                    path: src_path.display().to_string(),
                });
            }

            std::fs::create_dir_all(&dst_path).map_err(|source| BuildProfileError::CreateDir {
                path: dst_path.display().to_string(),
                source,
            })?;

            ancestors.insert(identity);
            descend_dir(
                &src_path, &dst_path, pkg_name, &rel_path, ownership, ancestors,
            )?;
            ancestors.remove(&identity);
        } else {
            if let Some(existing_pkg) = ownership.get(&rel_path) {
                warn!(
                    path = %rel_path.display(),
                    pkg_a = %existing_pkg,
                    pkg_b = %pkg_name,
                    "symlink conflict detected"
                );
                return Err(BuildProfileError::Conflict {
                    path: rel_path.display().to_string(),
                    pkg_a: existing_pkg.clone(),
                    pkg_b: pkg_name.to_owned(),
                });
            }
            ownership.insert(rel_path, pkg_name.to_owned());

            std::os::unix::fs::symlink(&src_path, &dst_path).map_err(|source| {
                BuildProfileError::CreateSymlink {
                    path: dst_path.display().to_string(),
                    source,
                }
            })?;

            debug!(
                pkg = %pkg_name,
                src = %src_path.display(),
                dst = %dst_path.display(),
                "created symlink"
            );
        }
    }

    Ok(())
}

/// Build a unified symlink tree from a set of resolved packages.
///
/// Descends each package's `store_path` recursively. Every directory
/// (including those reached through directory symlinks) is created as a real
/// directory inside `tmp_path`. Every leaf — regular file, file symlink, or
/// dangling symlink — is represented as a symlink pointing into the store.
///
/// Returns a [`BuildProfileError::Conflict`] if two packages provide the
/// same relative file path.
#[expect(
    clippy::unused_async,
    reason = "async interface kept for future I/O offloading"
)]
pub async fn build_symlink_tree(
    tmp_path: &Path,
    packages: &[ResolvedPackage],
) -> Result<(), BuildProfileError> {
    let mut ownership: HashMap<PathBuf, String> = HashMap::new();

    for pkg in packages {
        let store_path = Path::new(&pkg.store_path);
        // Ancestor identity set is per-package-root, not shared across packages,
        // so a diamond (two packages sharing a store directory) is handled correctly.
        let mut ancestors: HashSet<(u64, u64)> = HashSet::new();

        descend_dir(
            store_path,
            tmp_path,
            &pkg.name,
            Path::new(""),
            &mut ownership,
            &mut ancestors,
        )?;
    }

    Ok(())
}

/// Build a new profile generation.
///
/// Creates a generation directory under `profile_dir` containing a merged
/// symlink tree of all packages, runs hooks, and writes the manifest.
///
/// The generation directory is named `{generation}-link`.
///
/// When `hooks_override_path` is `Some`, hooks are executed from that path
/// instead of from inside the profile. This is needed for cross-compilation
/// bootstrap where the profile contains ARM hooks but we run on x86_64.
///
/// Returns the [`ProfileGeneration`] metadata for the new generation.
pub async fn build_profile(
    profile_dir: &Path,
    generation: usize,
    packages: &[ResolvedPackage],
    hooks_dir_name: &str,
    hooks_override_path: Option<&Path>,
) -> Result<ProfileGeneration, BuildProfileError> {
    let gen_name = generation_link_name(generation);
    let tmp_path = profile_dir.join(format!("{gen_name}.tmp"));
    let gen_path = profile_dir.join(&gen_name);

    // Ensure profile directory exists
    std::fs::create_dir_all(profile_dir).map_err(|source| BuildProfileError::CreateDir {
        path: profile_dir.display().to_string(),
        source,
    })?;

    // Clean up any leftover tmp directory from a previous failed build
    if tmp_path.exists() {
        std::fs::remove_dir_all(&tmp_path).map_err(|source| BuildProfileError::RemoveDir {
            path: tmp_path.display().to_string(),
            source,
        })?;
    }

    std::fs::create_dir_all(&tmp_path).map_err(|source| BuildProfileError::CreateDir {
        path: tmp_path.display().to_string(),
        source,
    })?;

    info!(%gen_name, "building symlink tree");

    // Step 1: Build symlink tree
    build_symlink_tree(&tmp_path, packages).await?;

    // Step 2: Run hooks
    crate::hooks::run_hooks(&tmp_path, hooks_dir_name, hooks_override_path).await?;

    // Step 3: Write manifest
    let manifest = crate::manifest::build_manifest(packages);
    crate::manifest::write_manifest(&tmp_path, &manifest)?;

    // Step 4: Rename tmp to final generation path
    std::fs::rename(&tmp_path, &gen_path).map_err(|source| BuildProfileError::Rename { source })?;

    info!(%gen_name, "profile generation built successfully");

    Ok(ProfileGeneration {
        number: generation,
        path: gen_path,
        manifest,
    })
}

/// Highest existing generation number in `profile_dir`, or `None`
/// when the directory is absent or contains no `<N>-link` entries.
///
/// Scans `profile_dir` for entries named `<N>-link` and returns `max(N)`.
/// Propagates I/O errors (permission denied, ENOTDIR, mid-scan
/// `DirEntry` failures); callers that want "+1" math or a default
/// apply it at the call site.
pub fn max_generation(profile_dir: &Path) -> Result<Option<usize>, BuildProfileError> {
    if !profile_dir.exists() {
        return Ok(None);
    }
    let entries =
        std::fs::read_dir(profile_dir).map_err(|source| BuildProfileError::ReadDir { source })?;
    let mut max_gen: Option<usize> = None;
    for entry in entries {
        let entry = entry.map_err(|source| BuildProfileError::ReadDir { source })?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some(num) = parse_generation_link_name(&name_str) {
            max_gen = Some(max_gen.map_or(num, |m| m.max(num)));
        }
    }
    Ok(max_gen)
}

/// Resolve `profile_dir/current` into its (absolute) target path.
///
/// Returns `Ok(None)` when the `current` symlink does not exist yet
/// (fresh profile). Propagates all other I/O errors — permission
/// denied, ENOTDIR on a non-symlink `current`, etc. Relative targets
/// are rebased onto `profile_dir`.
pub fn current_generation_link(profile_dir: &Path) -> Result<Option<PathBuf>, std::io::Error> {
    let current = profile_dir.join(CURRENT_LINK_NAME);
    match std::fs::read_link(&current) {
        Ok(target) => {
            let resolved = if target.is_absolute() {
                target
            } else {
                profile_dir.join(target)
            };
            Ok(Some(resolved))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Activate a profile generation by executing its activation entrypoint.
///
/// Runs `core/activation/entrypoint` inside the generation directory. This
/// entrypoint is generated by `hook_activation_resolver` and calls individual
/// activation scripts in the correct order.
///
/// The entrypoint receives `PROFILE_NEW_GENERATION` and
/// `PROFILE_OLD_GENERATION` environment variables.
///
/// When `profile_lock` is [`Some`], the entrypoint also receives
/// `ACTIVATION_HAS_PROFILE_LOCK=1`, signaling that the caller already holds the
/// profile lock and the entrypoint must not acquire it again.
pub async fn activate_profile(
    profile_dir: &Path,
    generation_number: usize,
    generation_path: &Path,
    profile_lock: Option<&ProfileLock>,
) -> Result<(), BuildProfileError> {
    let entrypoint = generation_path.join("core/activation/entrypoint");

    if !entrypoint.exists() {
        return Err(crate::activation::ActivationError::EntrypointNotFound {
            path: entrypoint.display().to_string(),
        }
        .into());
    }

    // Deliberately swallow I/O errors: this feeds PROFILE_OLD_GENERATION,
    // where an empty path is the sentinel for "no previous generation".
    // A transient read_link failure should not block activation of the
    // new generation.
    let old_gen = current_generation_link(profile_dir)
        .ok()
        .flatten()
        .unwrap_or_default();

    info!(
        entrypoint = %entrypoint.display(),
        "executing activation entrypoint"
    );

    let mut command = tokio::process::Command::new(&entrypoint);
    command
        .env("PROFILE_NEW_GENERATION", generation_path)
        .env("PROFILE_OLD_GENERATION", &old_gen);
    if profile_lock.is_some() {
        command.env("ACTIVATION_HAS_PROFILE_LOCK", "1");
    }

    let output = command.output().await.map_err(|source| {
        crate::activation::ActivationError::EntrypointExecute {
            path: entrypoint.display().to_string(),
            source,
        }
    })?;

    if !output.status.success() {
        match output.status.code() {
            Some(exit_code) => {
                return Err(crate::activation::ActivationError::EntrypointFailed {
                    path: entrypoint.display().to_string(),
                    exit_code,
                }
                .into());
            }
            None => {
                return Err(crate::activation::ActivationError::EntrypointSignaled {
                    path: entrypoint.display().to_string(),
                }
                .into());
            }
        }
    }

    info!(
        generation = generation_number,
        path = %generation_path.display(),
        "activated profile generation"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use serial_test::serial;

    use super::*;
    use crate::types::{InstalledBy, Manifest, PinStrategy};

    fn test_resolved_package(name: &str, store_path: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            store_path: store_path.into(),
            cache_url: Some("https://cache.example.com".into()),
            cache_name: "local".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        }
    }

    /// Create a fake store path with the given files.
    /// `files` is a list of relative paths (e.g., "bin/hello", "lib/libfoo.so").
    fn create_fake_store(base: &Path, files: &[&str]) {
        for file_path in files {
            let full_path = base.join(file_path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent).expect("BUG: should create parent dirs");
            }
            std::fs::write(&full_path, format!("content of {file_path}"))
                .expect("BUG: should write fake file");
        }
    }

    #[tokio::test]
    async fn build_symlink_tree_merges_packages() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        // Create two fake store paths with different files
        let store_a = tmp.path().join("store-a");
        create_fake_store(&store_a, &["bin/hello", "lib/liba.so"]);

        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["bin/world", "lib/libb.so"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: build_symlink_tree should succeed");

        // Verify all symlinks exist and point to correct targets
        let hello_link = output_dir.join("bin/hello");
        assert!(hello_link.is_symlink(), "bin/hello should be a symlink");
        assert_eq!(
            std::fs::read_link(&hello_link).expect("BUG: should read symlink"),
            store_a.join("bin/hello")
        );

        let world_link = output_dir.join("bin/world");
        assert!(world_link.is_symlink(), "bin/world should be a symlink");
        assert_eq!(
            std::fs::read_link(&world_link).expect("BUG: should read symlink"),
            store_b.join("bin/world")
        );

        let liba_link = output_dir.join("lib/liba.so");
        assert!(liba_link.is_symlink(), "lib/liba.so should be a symlink");

        let libb_link = output_dir.join("lib/libb.so");
        assert!(libb_link.is_symlink(), "lib/libb.so should be a symlink");

        // Verify directories were created (not symlinks)
        assert!(
            output_dir.join("bin").is_dir(),
            "bin/ should be a directory"
        );
        assert!(
            output_dir.join("lib").is_dir(),
            "lib/ should be a directory"
        );
    }

    #[tokio::test]
    async fn symlink_conflict_returns_error() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        // Two store paths with the same file
        let store_a = tmp.path().join("store-a");
        create_fake_store(&store_a, &["bin/conflict"]);

        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["bin/conflict"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        let result = build_symlink_tree(&output_dir, &packages).await;
        assert!(result.is_err(), "should return error on conflict");

        let err = result.expect_err("BUG: already checked is_err");
        match err {
            BuildProfileError::Conflict { path, pkg_a, pkg_b } => {
                assert_eq!(path, "bin/conflict");
                assert_eq!(pkg_a, "pkg-a");
                assert_eq!(pkg_b, "pkg-b");
            }
            other @ (BuildProfileError::CreateDir { .. }
            | BuildProfileError::RemoveDir { .. }
            | BuildProfileError::CreateSymlink { .. }
            | BuildProfileError::ReadStorePath { .. }
            | BuildProfileError::StatStorePath { .. }
            | BuildProfileError::SymlinkCycle { .. }
            | BuildProfileError::Hooks(_)
            | BuildProfileError::Manifest(_)
            | BuildProfileError::Rename { .. }
            | BuildProfileError::ReadDir { .. }
            | BuildProfileError::Activation(_)
            | BuildProfileError::Lock { .. }) => {
                panic!("expected Conflict error, got: {other}")
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn build_profile_creates_generation() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store_a = tmp.path().join("store-a");
        create_fake_store(&store_a, &["bin/hello"]);

        let packages = vec![test_resolved_package(
            "pkg-a",
            store_a.to_str().expect("BUG: valid UTF-8"),
        )];

        let profile_dir = tmp.path().join("bmc");

        let generation = build_profile(&profile_dir, 1, &packages, "hooks", None)
            .await
            .expect("BUG: build_profile should succeed");

        assert_eq!(generation.number, 1);
        assert!(
            generation.path.exists(),
            "generation directory should exist"
        );
        assert_eq!(
            generation.path.file_name().and_then(|n| n.to_str()),
            Some("1-link")
        );

        // Verify manifest was written
        let manifest_path = generation.path.join("manifest");
        assert!(manifest_path.exists(), "manifest file should exist");

        let manifest_content =
            std::fs::read_to_string(&manifest_path).expect("BUG: should read manifest");
        let manifest: Manifest =
            serde_json::from_str(&manifest_content).expect("BUG: manifest should be valid JSON");
        assert!(
            manifest.packages.contains_key("pkg-a"),
            "manifest should contain pkg-a"
        );

        // Verify symlink was created
        let hello_link = generation.path.join("bin/hello");
        assert!(hello_link.is_symlink(), "bin/hello should be a symlink");
    }

    #[test]
    fn max_generation_returns_none_for_nonexistent_dir() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("nonexistent");
        let result = max_generation(&profile_dir).expect("BUG: scan should succeed");
        assert_eq!(result, None);
    }

    #[test]
    fn max_generation_returns_none_for_empty_dir() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");
        let result = max_generation(&profile_dir).expect("BUG: scan should succeed");
        assert_eq!(result, None);
    }

    #[test]
    fn max_generation_returns_highest_existing() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(profile_dir.join("1-link")).expect("BUG: mk 1-link");
        std::fs::create_dir_all(profile_dir.join("3-link")).expect("BUG: mk 3-link");
        std::fs::create_dir_all(profile_dir.join("2-link")).expect("BUG: mk 2-link");
        let result = max_generation(&profile_dir).expect("BUG: scan should succeed");
        assert_eq!(result, Some(3));
    }

    #[test]
    fn max_generation_ignores_non_link_entries() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(profile_dir.join("1-link")).expect("BUG: mk 1-link");
        std::fs::create_dir_all(profile_dir.join("not-a-gen")).expect("BUG: mk junk");
        std::fs::write(profile_dir.join(".lock"), "").expect("BUG: write .lock");
        std::os::unix::fs::symlink("1-link", profile_dir.join("current"))
            .expect("BUG: symlink current");
        let result = max_generation(&profile_dir).expect("BUG: scan should succeed");
        assert_eq!(result, Some(1));
    }

    #[test]
    fn current_generation_link_missing() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");

        let result =
            current_generation_link(&profile_dir).expect("BUG: missing symlink should not error");
        assert_eq!(result, None);
    }

    #[test]
    fn current_generation_link_relative_target() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");
        std::os::unix::fs::symlink("1-link", profile_dir.join("current"))
            .expect("BUG: symlink current");

        let result = current_generation_link(&profile_dir).expect("BUG: should read symlink");
        assert_eq!(result, Some(profile_dir.join("1-link")));
    }

    #[test]
    fn current_generation_link_absolute_target() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");
        let abs_target = tmp.path().join("elsewhere/5-link");
        std::os::unix::fs::symlink(&abs_target, profile_dir.join("current"))
            .expect("BUG: symlink current");

        let result = current_generation_link(&profile_dir).expect("BUG: should read symlink");
        assert_eq!(result, Some(abs_target));
    }

    #[test]
    fn max_generation_propagates_read_dir_error() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let profile_dir = tmp.path().join("bmc");
        std::fs::create_dir_all(&profile_dir).expect("BUG: mkdir");
        // Revoke read+execute perms so `read_dir` fails with EACCES.
        let mut perms = std::fs::metadata(&profile_dir)
            .expect("BUG: stat")
            .permissions();
        perms.set_mode(0o000);
        std::fs::set_permissions(&profile_dir, perms).expect("BUG: chmod");

        let result = max_generation(&profile_dir);

        // Restore perms so tempdir cleanup works.
        let mut restore = std::fs::metadata(&profile_dir)
            .expect("BUG: stat")
            .permissions();
        restore.set_mode(0o755);
        std::fs::set_permissions(&profile_dir, restore).expect("BUG: chmod");

        assert!(
            matches!(result, Err(BuildProfileError::ReadDir { .. })),
            "expected ReadDir error, got {result:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn empty_packages_succeeds() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("bmc");

        let generation = build_profile(&profile_dir, 1, &[], "hooks", None)
            .await
            .expect("BUG: build_profile with empty packages should succeed");

        assert_eq!(generation.number, 1);
        assert!(
            generation.path.exists(),
            "generation directory should exist"
        );

        // Verify manifest was written with empty packages
        let manifest_path = generation.path.join("manifest");
        assert!(manifest_path.exists(), "manifest file should exist");

        let manifest_content =
            std::fs::read_to_string(&manifest_path).expect("BUG: should read manifest");
        let manifest: Manifest =
            serde_json::from_str(&manifest_content).expect("BUG: manifest should be valid JSON");
        assert!(
            manifest.packages.is_empty(),
            "manifest should have no packages"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_runs_entrypoint() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("bmc");

        let store_a = tmp.path().join("store-a");
        // Create a fake activation entrypoint in the store package
        let activation_dir_in_store = store_a.join("core/activation");
        std::fs::create_dir_all(&activation_dir_in_store)
            .expect("BUG: should create activation dir");

        let log_file = tmp.path().join("activation.log");

        let entrypoint_content = format!(
            "#!/bin/sh\necho \"activated $PROFILE_NEW_GENERATION\" >> {}\n",
            log_file.display()
        );
        let entrypoint_path = activation_dir_in_store.join("entrypoint");
        std::fs::write(&entrypoint_path, &entrypoint_content)
            .expect("BUG: should write entrypoint");
        std::fs::set_permissions(&entrypoint_path, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: should set permissions");

        let packages = vec![test_resolved_package(
            "pkg-a",
            store_a.to_str().expect("BUG: valid UTF-8"),
        )];

        let generation = build_profile(&profile_dir, 1, &packages, "hooks", None)
            .await
            .expect("BUG: build_profile should succeed");

        activate_profile(&profile_dir, generation.number, &generation.path, None)
            .await
            .expect("BUG: activate_profile should succeed");

        let log_content = std::fs::read_to_string(&log_file).expect("BUG: should read log file");
        assert!(
            log_content.contains("activated"),
            "activation entrypoint should have run"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_does_not_set_activation_has_profile_lock_without_lock() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("bmc");

        let store_a = tmp.path().join("store-a");
        let activation_dir_in_store = store_a.join("core/activation");
        std::fs::create_dir_all(&activation_dir_in_store)
            .expect("BUG: should create activation dir");

        let log_file = tmp.path().join("activation-env.log");

        let entrypoint_content = format!(
            "#!/bin/sh\nprintf 'ACTIVATION_HAS_PROFILE_LOCK=%s\\n' \"${{ACTIVATION_HAS_PROFILE_LOCK-}}\" > {}\n",
            log_file.display()
        );
        let entrypoint_path = activation_dir_in_store.join("entrypoint");
        std::fs::write(&entrypoint_path, &entrypoint_content)
            .expect("BUG: should write entrypoint");
        std::fs::set_permissions(&entrypoint_path, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: should set permissions");

        let packages = vec![test_resolved_package(
            "pkg-a",
            store_a.to_str().expect("BUG: valid UTF-8"),
        )];

        let generation = build_profile(&profile_dir, 1, &packages, "hooks", None)
            .await
            .expect("BUG: build_profile should succeed");

        activate_profile(&profile_dir, generation.number, &generation.path, None)
            .await
            .expect("BUG: activate_profile should succeed");

        let log_content = std::fs::read_to_string(&log_file).expect("BUG: should read log file");
        assert_eq!(
            log_content, "ACTIVATION_HAS_PROFILE_LOCK=\n",
            "activation entrypoint should not see the pre-held lock marker without a lock witness"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_sets_activation_has_profile_lock_when_lock_is_passed() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("bmc");

        let store_a = tmp.path().join("store-a");
        let activation_dir_in_store = store_a.join("core/activation");
        std::fs::create_dir_all(&activation_dir_in_store)
            .expect("BUG: should create activation dir");

        let log_file = tmp.path().join("activation-env.log");

        let entrypoint_content = format!(
            "#!/bin/sh\nprintf 'ACTIVATION_HAS_PROFILE_LOCK=%s\\n' \"${{ACTIVATION_HAS_PROFILE_LOCK-}}\" > {}\n",
            log_file.display()
        );
        let entrypoint_path = activation_dir_in_store.join("entrypoint");
        std::fs::write(&entrypoint_path, &entrypoint_content)
            .expect("BUG: should write entrypoint");
        std::fs::set_permissions(&entrypoint_path, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: should set permissions");

        let packages = vec![test_resolved_package(
            "pkg-a",
            store_a.to_str().expect("BUG: valid UTF-8"),
        )];

        let generation = build_profile(&profile_dir, 1, &packages, "hooks", None)
            .await
            .expect("BUG: build_profile should succeed");
        let lock = lock_profile(&profile_dir)
            .await
            .expect("BUG: lock_profile should succeed");

        activate_profile(
            &profile_dir,
            generation.number,
            &generation.path,
            Some(&lock),
        )
        .await
        .expect("BUG: activate_profile should succeed");

        let log_content = std::fs::read_to_string(&log_file).expect("BUG: should read log file");
        assert_eq!(
            log_content, "ACTIVATION_HAS_PROFILE_LOCK=1\n",
            "activation entrypoint should see the pre-held lock marker when a lock witness is passed"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_fails_without_entrypoint() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("bmc");

        let store_a = tmp.path().join("store-a");
        create_fake_store(&store_a, &["bin/hello"]);

        let packages = vec![test_resolved_package(
            "pkg-a",
            store_a.to_str().expect("BUG: valid UTF-8"),
        )];

        let generation = build_profile(&profile_dir, 1, &packages, "hooks", None)
            .await
            .expect("BUG: build_profile should succeed");

        let result =
            activate_profile(&profile_dir, generation.number, &generation.path, None).await;
        assert!(
            result.is_err(),
            "activate_profile should fail when entrypoint is missing"
        );
    }

    #[test]
    fn parse_generation_link_name_valid() {
        assert_eq!(parse_generation_link_name("1-link"), Some(1));
        assert_eq!(parse_generation_link_name("42-link"), Some(42));
        assert_eq!(parse_generation_link_name("100-link"), Some(100));
    }

    #[test]
    fn parse_generation_link_name_invalid() {
        assert_eq!(parse_generation_link_name("not-a-gen"), None);
        assert_eq!(parse_generation_link_name("bmc-1-link"), None);
        assert_eq!(parse_generation_link_name("abc-link"), None);
        assert_eq!(parse_generation_link_name(""), None);
        assert_eq!(parse_generation_link_name("current"), None);
    }

    #[tokio::test]
    async fn build_symlink_tree_preserves_nested_directories() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store = tmp.path().join("store");
        create_fake_store(&store, &["a/b/c/deep_file"]);

        let packages = vec![test_resolved_package(
            "deep-pkg",
            store.to_str().expect("BUG: valid UTF-8"),
        )];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: should handle nested directories");

        assert!(
            output_dir.join("a/b/c").is_dir(),
            "nested directories should be created"
        );
        assert!(
            output_dir.join("a/b/c/deep_file").is_symlink(),
            "deep file should be a symlink"
        );
    }

    // Test 1: the bug itself — package A exposes `share` as a symlink to a
    // directory; package B contributes a real file under `share/`.  The build
    // must succeed and `share` in the output must be a real directory.
    #[tokio::test]
    async fn dir_symlink_unrolled_merges_correctly() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        // pkg-a: share → real_share (a directory symlink inside the store)
        let store_a = tmp.path().join("store-a");
        std::fs::create_dir_all(&store_a).expect("BUG: create store-a");
        let real_share = tmp.path().join("real_share");
        std::fs::create_dir_all(&real_share).expect("BUG: create real_share");
        std::fs::write(real_share.join("python3.11_foo"), "content")
            .expect("BUG: write python3.11_foo");
        std::os::unix::fs::symlink(&real_share, store_a.join("share"))
            .expect("BUG: create share symlink");

        // pkg-b: real share/applications/bar
        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["share/applications/bar"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: build should succeed with dir symlink unrolling");

        let share = output_dir.join("share");
        assert!(
            share
                .symlink_metadata()
                .expect("BUG: stat share")
                .file_type()
                .is_dir(),
            "share must be a real directory, not a symlink"
        );
        assert!(
            !share
                .symlink_metadata()
                .expect("BUG: stat share")
                .file_type()
                .is_symlink(),
            "share must not be a symlink"
        );
        assert!(
            output_dir.join("share/python3.11_foo").is_symlink(),
            "share/python3.11_foo should be a symlink"
        );
        assert!(
            output_dir.join("share/applications/bar").is_symlink(),
            "share/applications/bar should be a symlink"
        );
    }

    // Same merge as above, but with the real directory package first.  This
    // covers the original failure where `share/` already existed in the output
    // before the symlinked `share` package was processed.
    #[tokio::test]
    async fn dir_symlink_unrolled_merges_after_real_dir() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        // pkg-a: share → real_share (a directory symlink inside the store)
        let store_a = tmp.path().join("store-a");
        std::fs::create_dir_all(&store_a).expect("BUG: create store-a");
        let real_share = tmp.path().join("real_share");
        std::fs::create_dir_all(&real_share).expect("BUG: create real_share");
        std::fs::write(real_share.join("python3.11_foo"), "content")
            .expect("BUG: write python3.11_foo");
        std::os::unix::fs::symlink(&real_share, store_a.join("share"))
            .expect("BUG: create share symlink");

        // pkg-b: real share/applications/bar
        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["share/applications/bar"]);

        let packages = vec![
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: build should succeed with real dir before dir symlink");

        let share = output_dir.join("share");
        assert!(
            share
                .symlink_metadata()
                .expect("BUG: stat share")
                .file_type()
                .is_dir(),
            "share must be a real directory, not a symlink"
        );
        assert!(
            !share
                .symlink_metadata()
                .expect("BUG: stat share")
                .file_type()
                .is_symlink(),
            "share must not be a symlink"
        );
        assert!(
            output_dir.join("share/python3.11_foo").is_symlink(),
            "share/python3.11_foo should be a symlink"
        );
        assert!(
            output_dir.join("share/applications/bar").is_symlink(),
            "share/applications/bar should be a symlink"
        );
    }

    // Test 2: a file symlink inside a store path is preserved as a symlink in
    // the output (not resolved or copied).
    #[tokio::test]
    async fn file_symlink_preserved_as_symlink() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).expect("BUG: create store");
        let target = store.join("real_file");
        std::fs::write(&target, "data").expect("BUG: write real_file");
        std::os::unix::fs::symlink(&target, store.join("file_link"))
            .expect("BUG: create file_link");

        let packages = vec![test_resolved_package(
            "pkg",
            store.to_str().expect("BUG: valid UTF-8"),
        )];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: build should succeed");

        let out_link = output_dir.join("file_link");
        assert!(
            out_link
                .symlink_metadata()
                .expect("BUG: stat file_link")
                .file_type()
                .is_symlink(),
            "file_link in output must be a symlink"
        );
        assert_eq!(
            std::fs::read_link(&out_link).expect("BUG: read file_link symlink"),
            store.join("file_link"),
            "file_link in output must point at the store symlink, not the resolved target"
        );
    }

    // Test 3: when two packages provide the same file under a directory that
    // was previously a symlink-dir, the conflict is reported correctly.
    #[tokio::test]
    async fn real_conflict_under_unrolled_dir() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        // pkg-a: share → real_share containing foo
        let store_a = tmp.path().join("store-a");
        std::fs::create_dir_all(&store_a).expect("BUG: create store-a");
        let real_share = tmp.path().join("real_share");
        std::fs::create_dir_all(&real_share).expect("BUG: create real_share");
        std::fs::write(real_share.join("foo"), "content-a").expect("BUG: write foo");
        std::os::unix::fs::symlink(&real_share, store_a.join("share"))
            .expect("BUG: create share symlink");

        // pkg-b: real share/foo
        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["share/foo"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        let result = build_symlink_tree(&output_dir, &packages).await;
        assert!(
            matches!(
                result,
                Err(BuildProfileError::Conflict { ref path, .. }) if path == "share/foo"
            ),
            "expected Conflict at share/foo, got: {result:?}"
        );
    }

    // Test 4: a directory symlink cycle (`self -> .`) must cause SymlinkCycle,
    // not an infinite recursion.
    #[tokio::test]
    async fn dir_symlink_cycle_returns_error() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).expect("BUG: create store");
        // `self` → `.` forms a cycle: resolves to `store/`, which is an ancestor.
        std::os::unix::fs::symlink(".", store.join("self")).expect("BUG: create self symlink");

        let packages = vec![test_resolved_package(
            "pkg",
            store.to_str().expect("BUG: valid UTF-8"),
        )];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        let result = build_symlink_tree(&output_dir, &packages).await;
        assert!(
            matches!(result, Err(BuildProfileError::SymlinkCycle { .. })),
            "expected SymlinkCycle error, got: {result:?}"
        );
    }

    // Test 5: a mutual file-symlink loop (`a -> b`, `b -> a`) causes stat to
    // fail with ELOOP, which must be propagated as StatStorePath.
    #[tokio::test]
    async fn eloop_file_symlink_propagated_as_stat_error() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).expect("BUG: create store");
        std::os::unix::fs::symlink(store.join("b"), store.join("a")).expect("BUG: create a -> b");
        std::os::unix::fs::symlink(store.join("a"), store.join("b")).expect("BUG: create b -> a");

        let packages = vec![test_resolved_package(
            "pkg",
            store.to_str().expect("BUG: valid UTF-8"),
        )];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        let result = build_symlink_tree(&output_dir, &packages).await;
        assert!(
            matches!(result, Err(BuildProfileError::StatStorePath { .. })),
            "expected StatStorePath error for ELOOP, got: {result:?}"
        );
    }

    // Test 6: two directory symlinks pointing at the same real directory
    // ("diamond") must both be unrolled successfully.  The ancestor set is
    // scoped to the current call stack, so revisiting `shared/` via a
    // different symlink is not a cycle.  A regression to a global visited
    // set would incorrectly fail on the second arm.
    #[tokio::test]
    async fn build_symlink_tree_unrolls_diamond_without_false_cycle() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).expect("BUG: create store");

        // Create the shared real directory with one file inside it.
        let shared = store.join("shared");
        std::fs::create_dir_all(&shared).expect("BUG: create shared");
        std::fs::write(shared.join("file.txt"), "shared content")
            .expect("BUG: write shared/file.txt");

        // x -> shared  and  y -> shared  (both are directory symlinks)
        std::os::unix::fs::symlink(&shared, store.join("x")).expect("BUG: create x -> shared");
        std::os::unix::fs::symlink(&shared, store.join("y")).expect("BUG: create y -> shared");

        let packages = vec![test_resolved_package(
            "pkg",
            store.to_str().expect("BUG: valid UTF-8"),
        )];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: diamond should not be mistaken for a cycle");

        let x_file = output_dir.join("x/file.txt");
        assert!(
            x_file
                .symlink_metadata()
                .expect("BUG: stat x/file.txt")
                .file_type()
                .is_symlink(),
            "x/file.txt must be a symlink"
        );

        let y_file = output_dir.join("y/file.txt");
        assert!(
            y_file
                .symlink_metadata()
                .expect("BUG: stat y/file.txt")
                .file_type()
                .is_symlink(),
            "y/file.txt must be a symlink"
        );
    }

    #[tokio::test]
    async fn lock_profile_creates_lock_file() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("bmc");

        let _lock = lock_profile(&profile_dir)
            .await
            .expect("BUG: lock_profile should succeed");

        assert!(
            profile_dir.join(".lock").exists(),
            ".lock file should exist"
        );
    }

    #[tokio::test]
    async fn lock_profile_is_exclusive() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("bmc");

        let lock = lock_profile(&profile_dir)
            .await
            .expect("BUG: first lock should succeed");

        // Second try_lock on the same directory should fail (held by us)
        let second = try_lock_profile(&profile_dir).expect("BUG: try_lock should not error");
        assert!(
            second.is_none(),
            "try_lock should return None while lock is held"
        );

        // Drop the first lock, now try_lock should succeed.
        drop(lock);
        let third = lock_profile_with_timeout(&profile_dir, std::time::Duration::from_millis(50))
            .await
            .expect("BUG: timed lock should not error");
        assert!(
            third.is_some(),
            "timed lock should succeed after lock is released"
        );
    }

    #[tokio::test]
    async fn lock_profile_creates_directory_if_missing() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("nonexistent/nested/bmc");

        let _lock = lock_profile(&profile_dir)
            .await
            .expect("BUG: lock_profile should create dirs");

        assert!(profile_dir.exists(), "profile directory should be created");
        assert!(
            profile_dir.join(".lock").exists(),
            ".lock file should exist"
        );
    }

    #[tokio::test]
    async fn lock_profile_waits_until_release_within_timeout() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("bmc");
        let held_lock = lock_profile(&profile_dir)
            .await
            .expect("BUG: first lock should succeed");

        let release_task = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            drop(held_lock);
        });

        let second = lock_profile_with_timeout(&profile_dir, std::time::Duration::from_millis(250))
            .await
            .expect("BUG: timed lock should not error");

        release_task
            .await
            .expect("BUG: release task should not panic");
        assert!(
            second.is_some(),
            "timed lock should succeed after the first lock is released"
        );
    }

    #[tokio::test]
    async fn lock_profile_times_out_while_lock_is_held() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("bmc");
        let _held_lock = lock_profile(&profile_dir)
            .await
            .expect("BUG: first lock should succeed");

        let second = lock_profile_with_timeout(&profile_dir, std::time::Duration::from_millis(50))
            .await
            .expect("BUG: timed lock should not error");

        assert!(
            second.is_none(),
            "timed lock should time out while another lock is held"
        );
    }
}
