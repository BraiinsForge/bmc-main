// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};

use tracing::info;

use crate::activation::ActivationError;
use crate::types::{ProfileGeneration, ResolvedPackage};

/// RAII guard holding an exclusive `flock` on a profile directory's
/// `.lock` file, released on drop.
pub use bmc_log::flock::FileLock as ProfileLock;

mod collisions;
mod union;

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
    #[error("failed to sync '{path}': {source}")]
    Sync {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to read profile directory: {source}")]
    ReadDir { source: std::io::Error },
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
#[must_use]
pub fn parse_generation_link_name(name: &str) -> Option<usize> {
    name.strip_suffix("-link")?.parse::<usize>().ok()
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

/// Acquire an exclusive lock on a profile directory.
///
/// Creates `<profile_dir>/.lock` and holds an exclusive `flock(2)` on it.
/// The blocking `flock` call is offloaded to a blocking thread via
/// [`tokio::task::spawn_blocking`] so it does not stall the async runtime.
/// The lock is released when the returned [`ProfileLock`] is dropped.
pub async fn lock_profile(profile_dir: &Path) -> Result<ProfileLock, BuildProfileError> {
    let file = open_lock_file(profile_dir)?;

    tokio::task::spawn_blocking(move || {
        bmc_log::flock::lock_file(file).map_err(|source| BuildProfileError::Lock { source })
    })
    .await
    .expect("BUG: lock task should not panic")
}

/// Try to acquire an exclusive lock on a profile directory without blocking.
///
/// Returns `Ok(Some(lock))` if acquired, `Ok(None)` if another process holds it.
pub fn try_lock_profile(profile_dir: &Path) -> Result<Option<ProfileLock>, BuildProfileError> {
    let file = open_lock_file(profile_dir)?;

    bmc_log::flock::try_lock_file(file).map_err(|source| BuildProfileError::Lock { source })
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
        bmc_log::flock::lock_file(file).map_err(|source| BuildProfileError::Lock { source })
    });

    match tokio::time::timeout(timeout, handle).await {
        Ok(join_result) => {
            let lock = join_result.expect("BUG: lock task should not panic")?;
            Ok(Some(lock))
        }
        Err(_elapsed) => Ok(None),
    }
}

/// Build a unified symlink tree from a set of resolved packages.
///
/// Single-provider directories are linked directly into `tmp_path`.
/// Directories provided by multiple packages are materialized in `tmp_path`,
/// then their children are recursively resolved with the same rules.
/// Leaves — regular files, file symlinks, and dangling symlinks — are
/// represented as symlinks pointing into the store.
///
/// Returns a [`BuildProfileError::Conflict`] if two packages provide the
/// same relative file path.
///
/// The tree is a sequential walk of many blocking filesystem syscalls, so it
/// runs on a [`tokio::task::spawn_blocking`] thread and does not stall the
/// async runtime of callers such as `bmc-openwrt`.
pub async fn build_symlink_tree(
    tmp_path: &Path,
    packages: &[ResolvedPackage],
) -> Result<(), BuildProfileError> {
    let tmp_path = tmp_path.to_path_buf();
    let packages = packages.to_vec();

    tokio::task::spawn_blocking(move || union::build_symlink_tree(&tmp_path, &packages))
        .await
        .expect("BUG: symlink tree task should not panic")
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

    // Step 4: Make the generation contents (tree, hook outputs,
    // manifest) durable before publishing the name.
    crate::fs_sync::sync_filesystem_of_blocking(profile_dir)
        .await
        .map_err(|source| BuildProfileError::Sync {
            path: profile_dir.display().to_string(),
            source,
        })?;

    // Step 5: Rename tmp to final generation path
    std::fs::rename(&tmp_path, &gen_path).map_err(|source| BuildProfileError::Rename { source })?;

    // Step 6: Make the publication durable before reporting success.
    crate::fs_sync::fsync_dir(profile_dir).map_err(|source| BuildProfileError::Sync {
        path: profile_dir.display().to_string(),
        source,
    })?;

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

/// Activate a profile generation, reverting to `current` on failure.
///
/// Runs `core/activation/entrypoint` of `generation_path` under the
/// profile lock. When activating a generation other than the one
/// `current` points to, a failure triggers re-activation of the old
/// generation with `PROFILE_OLD_GENERATION` set to the failed target,
/// so diff-driven activation scripts can undo its side effects; the
/// outcome is reported as [`ActivationError::RevertedAfterFailure`]
/// (revert succeeded) or [`ActivationError::RevertFailed`] (revert
/// failed too). When the target already is `current` — or no valid
/// `current` exists — there is nothing to revert to and errors
/// propagate unchanged. bmc-nix never writes the `current` symlink;
/// only the generation's own activation scripts do.
///
/// When `profile_lock` is [`None`], the lock is acquired internally
/// (blocking) for the duration of the sequence. The entrypoint always
/// receives `ACTIVATION_HAS_PROFILE_LOCK=1`.
pub async fn activate_profile(
    profile_dir: &Path,
    generation_number: usize,
    generation_path: &Path,
    profile_lock: Option<&ProfileLock>,
) -> Result<(), ActivationError> {
    let _owned_lock = match profile_lock {
        Some(_) => None,
        None => Some(
            lock_profile(profile_dir)
                .await
                .map_err(|err| ActivationError::Lock(Box::new(err)))?,
        ),
    };

    let entrypoint = generation_path.join("core/activation/entrypoint");
    if !crate::activation::is_executable(&entrypoint) {
        return Err(ActivationError::EntrypointNotFound {
            path: entrypoint.display().to_string(),
        });
    }

    let current = crate::activation::resolve_current_link(profile_dir)?;
    let revert_target = current
        .clone()
        .filter(|cur| !crate::activation::canonicalize_pair(cur, generation_path));

    let attempt =
        run_activation_entrypoint(generation_number, generation_path, current.as_deref()).await;

    let Err(original) = attempt else {
        return Ok(());
    };
    let Some(revert_target) = revert_target else {
        return Err(original);
    };

    let Some(reverted_to) = crate::activation::generation_number_from_link(&revert_target) else {
        return Err(ActivationError::RevertFailed {
            original: Box::new(original),
            revert_error: Box::new(ActivationError::NoGeneration {
                profile_dir: revert_target.display().to_string(),
            }),
        });
    };
    match run_activation_entrypoint(reverted_to, &revert_target, Some(generation_path)).await {
        Ok(()) => Err(ActivationError::RevertedAfterFailure {
            original: Box::new(original),
            reverted_to,
        }),
        Err(revert_error) => Err(ActivationError::RevertFailed {
            original: Box::new(original),
            revert_error: Box::new(revert_error),
        }),
    }
}

/// Execute a generation's activation entrypoint.
///
/// The entrypoint receives `PROFILE_NEW_GENERATION`,
/// `PROFILE_OLD_GENERATION` (empty when `old_generation` is [`None`],
/// the entrypoint's sentinel for "no previous generation"), and
/// `ACTIVATION_HAS_PROFILE_LOCK=1` — [`activate_profile`] guarantees
/// the profile lock is held for the whole call.
async fn run_activation_entrypoint(
    generation_number: usize,
    generation_path: &Path,
    old_generation: Option<&Path>,
) -> Result<(), ActivationError> {
    let entrypoint = generation_path.join("core/activation/entrypoint");

    if !entrypoint.exists() {
        return Err(ActivationError::EntrypointNotFound {
            path: entrypoint.display().to_string(),
        });
    }

    info!(
        entrypoint = %entrypoint.display(),
        "executing activation entrypoint"
    );

    let mut command = tokio::process::Command::new(&entrypoint);
    command
        .env("PROFILE_NEW_GENERATION", generation_path)
        .env(
            "PROFILE_OLD_GENERATION",
            old_generation.map(Path::as_os_str).unwrap_or_default(),
        )
        .env("ACTIVATION_HAS_PROFILE_LOCK", "1");

    let output = crate::store::output_bounded(command)
        .await
        .map_err(|source| ActivationError::EntrypointExecute {
            path: entrypoint.display().to_string(),
            source,
        })?;

    let snippet = if output.stderr.is_empty() {
        crate::store::stderr_snippet(&output.stdout)
    } else {
        crate::store::stderr_snippet(&output.stderr)
    };

    if !output.status.success() {
        tracing::warn!(
            entrypoint = %entrypoint.display(),
            status = ?output.status,
            output = %snippet,
            "activation entrypoint failed"
        );
        match output.status.code() {
            Some(exit_code) => {
                return Err(ActivationError::EntrypointFailed {
                    path: entrypoint.display().to_string(),
                    exit_code,
                    output: snippet,
                });
            }
            None => {
                return Err(ActivationError::EntrypointSignaled {
                    path: entrypoint.display().to_string(),
                    output: snippet,
                });
            }
        }
    }

    tracing::debug!(
        entrypoint = %entrypoint.display(),
        output = %snippet,
        "activation entrypoint completed successfully"
    );

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
    use crate::types::{InstalledBy, Manifest};

    fn test_resolved_package(name: &str, store_path: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            store_path: store_path.into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: None,
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
            | BuildProfileError::Sync { .. }
            | BuildProfileError::ReadDir { .. }
            | BuildProfileError::Lock { .. }) => {
                panic!("expected Conflict error, got: {other}")
            }
        }
    }

    #[tokio::test]
    async fn identical_symlink_target_collision_is_allowed() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        // A shared target both packages link to.
        let shared_target = tmp.path().join("shared-target");
        std::fs::write(&shared_target, "shared content").expect("BUG: write shared target");

        // Two store paths each providing bin/tool as a symlink to shared_target.
        let store_a = tmp.path().join("store-a");
        std::fs::create_dir_all(store_a.join("bin")).expect("BUG: create store-a/bin");
        std::os::unix::fs::symlink(&shared_target, store_a.join("bin/tool"))
            .expect("BUG: symlink store-a/bin/tool");

        let store_b = tmp.path().join("store-b");
        std::fs::create_dir_all(store_b.join("bin")).expect("BUG: create store-b/bin");
        std::os::unix::fs::symlink(&shared_target, store_b.join("bin/tool"))
            .expect("BUG: symlink store-b/bin/tool");

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: identical symlink targets should not conflict");

        let tool = output_dir.join("bin/tool");
        assert!(tool.is_symlink(), "bin/tool should be a symlink");
        assert_eq!(
            std::fs::canonicalize(&tool).expect("BUG: canonicalize tool"),
            std::fs::canonicalize(&shared_target).expect("BUG: canonicalize target"),
            "bin/tool should resolve to the shared target",
        );
    }

    #[tokio::test]
    async fn identical_relative_symlink_target_collision_is_error() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        // Two store paths each providing bin/tool as a symlink to the same
        // relative target name. Equal relative targets resolve differently per
        // store, so this is a real conflict rather than an allowed collision.
        let store_a = tmp.path().join("store-a");
        std::fs::create_dir_all(store_a.join("bin")).expect("BUG: create store-a/bin");
        std::os::unix::fs::symlink("busybox", store_a.join("bin/tool"))
            .expect("BUG: symlink store-a/bin/tool");

        let store_b = tmp.path().join("store-b");
        std::fs::create_dir_all(store_b.join("bin")).expect("BUG: create store-b/bin");
        std::os::unix::fs::symlink("busybox", store_b.join("bin/tool"))
            .expect("BUG: symlink store-b/bin/tool");

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        let result = build_symlink_tree(&output_dir, &packages).await;
        assert!(
            result.is_err(),
            "identical relative symlink targets should conflict"
        );

        let err = result.expect_err("BUG: already checked is_err");
        match err {
            BuildProfileError::Conflict { path, pkg_a, pkg_b } => {
                assert_eq!(path, "bin/tool");
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
            | BuildProfileError::Sync { .. }
            | BuildProfileError::ReadDir { .. }
            | BuildProfileError::Lock { .. }) => {
                panic!("expected Conflict error, got: {other}")
            }
        }
    }

    #[tokio::test]
    async fn build_symlink_tree_allows_dir_path_collision() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store_a = tmp.path().join("store-a");
        create_fake_store(&store_a, &["share/doc/README"]);
        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["share/doc/README"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: allowlisted collision should not error");

        let readme = output_dir.join("share/doc/README");
        assert!(readme.is_symlink(), "share/doc/README should be a symlink");
        assert_eq!(
            std::fs::read_link(&readme).expect("BUG: should read symlink"),
            store_a.join("share/doc/README"),
            "first package's symlink should win",
        );
    }

    #[tokio::test]
    async fn build_symlink_tree_allows_cargo_timings_collision() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        // Every crate built with `cargo --timings` ships the shared workspace-deps chart
        // under the same relative path, with per-build content.
        let store_a = tmp.path().join("store-a");
        create_fake_store(
            &store_a,
            &["cargo-timings/cargo-timing-workspace-deps-check.html"],
        );
        let store_b = tmp.path().join("store-b");
        create_fake_store(
            &store_b,
            &["cargo-timings/cargo-timing-workspace-deps-check.html"],
        );

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: allowlisted collision should not error");

        let chart = output_dir.join("cargo-timings/cargo-timing-workspace-deps-check.html");
        assert!(chart.is_symlink(), "timing chart should be a symlink");
        assert_eq!(
            std::fs::read_link(&chart).expect("BUG: should read symlink"),
            store_a.join("cargo-timings/cargo-timing-workspace-deps-check.html"),
            "first package's symlink should win",
        );
    }

    #[tokio::test]
    async fn build_symlink_tree_allows_dir_name_collision() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store_a = tmp.path().join("store-a");
        create_fake_store(&store_a, &["lib/foo/__pycache__/m.pyc"]);
        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["lib/foo/__pycache__/m.pyc"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: allowlisted collision should not error");

        let target = output_dir.join("lib/foo/__pycache__/m.pyc");
        assert!(target.is_symlink(), "m.pyc should be a symlink");
        assert_eq!(
            std::fs::read_link(&target).expect("BUG: should read symlink"),
            store_a.join("lib/foo/__pycache__/m.pyc"),
            "first package's symlink should win",
        );
    }

    #[tokio::test]
    async fn build_symlink_tree_allows_file_path_collision() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store_a = tmp.path().join("store-a");
        create_fake_store(&store_a, &["share/applications/mimeinfo.cache"]);
        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["share/applications/mimeinfo.cache"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: allowlisted collision should not error");

        let target = output_dir.join("share/applications/mimeinfo.cache");
        assert!(target.is_symlink(), "mimeinfo.cache should be a symlink");
        assert_eq!(
            std::fs::read_link(&target).expect("BUG: should read symlink"),
            store_a.join("share/applications/mimeinfo.cache"),
            "first package's symlink should win",
        );
    }

    #[tokio::test]
    async fn build_symlink_tree_allows_file_name_collision() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store_a = tmp.path().join("store-a");
        create_fake_store(&store_a, &["share/icons/hicolor/icon-theme.cache"]);
        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["share/icons/hicolor/icon-theme.cache"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: allowlisted collision should not error");

        let cache = output_dir.join("share/icons/hicolor/icon-theme.cache");
        assert!(cache.is_symlink(), "cache file should be a symlink");
        assert_eq!(
            std::fs::read_link(&cache).expect("BUG: should read symlink"),
            store_a.join("share/icons/hicolor/icon-theme.cache"),
            "first package's symlink should win",
        );
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

        let bin_link = generation.path.join("bin");
        assert!(
            bin_link
                .symlink_metadata()
                .expect("BUG: stat bin")
                .file_type()
                .is_symlink(),
            "single-provider bin should be a symlink"
        );
        assert_eq!(
            std::fs::read_link(&bin_link).expect("BUG: read bin symlink"),
            store_a.join("bin")
        );
    }

    #[tokio::test]
    #[serial]
    async fn build_profile_replaces_package_manifest_leaf_without_mutating_store() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).expect("BUG: create store");
        std::fs::write(store.join("manifest"), "package manifest")
            .expect("BUG: write manifest leaf");

        let packages = vec![test_resolved_package(
            "pkg-with-manifest",
            store.to_str().expect("BUG: valid UTF-8"),
        )];
        let profile_dir = tmp.path().join("bmc");

        let generation = build_profile(&profile_dir, 1, &packages, "hooks", None)
            .await
            .expect("BUG: build_profile should succeed");

        let manifest_path = generation.path.join("manifest");
        let meta = manifest_path
            .symlink_metadata()
            .expect("BUG: stat generated manifest");
        assert!(
            meta.is_file(),
            "generated manifest should be a regular file"
        );
        assert!(
            !meta.file_type().is_symlink(),
            "generated manifest should not be a symlink into the store"
        );
        assert_eq!(
            std::fs::read_to_string(store.join("manifest")).expect("BUG: read store manifest"),
            "package manifest",
            "profile manifest writing must not mutate the package store path"
        );

        let manifest_content =
            std::fs::read_to_string(&manifest_path).expect("BUG: read generated manifest");
        let manifest: Manifest = serde_json::from_str(&manifest_content)
            .expect("BUG: generated manifest should be JSON");
        assert!(manifest.packages.contains_key("pkg-with-manifest"));
    }

    #[tokio::test]
    #[serial]
    async fn build_profile_errors_when_manifest_target_is_directory() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");
        let store = tmp.path().join("store");
        std::fs::create_dir_all(store.join("manifest")).expect("BUG: create manifest directory");

        let packages = vec![test_resolved_package(
            "pkg-with-manifest-dir",
            store.to_str().expect("BUG: valid UTF-8"),
        )];
        let profile_dir = tmp.path().join("bmc");

        let result = build_profile(&profile_dir, 1, &packages, "hooks", None).await;

        assert!(
            matches!(result, Err(BuildProfileError::Manifest(_))),
            "expected manifest write error, got {result:?}"
        );
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
    async fn activate_profile_sets_activation_has_profile_lock_without_lock() {
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
            log_content, "ACTIVATION_HAS_PROFILE_LOCK=1\n",
            "activation entrypoint should see the pre-held lock marker without an external lock witness"
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

        let err = activate_profile(&profile_dir, generation.number, &generation.path, None)
            .await
            .expect_err("BUG: activate_profile should fail when entrypoint is missing");
        assert!(
            matches!(
                err,
                crate::activation::ActivationError::EntrypointNotFound { .. }
            ),
            "got {err:?}"
        );
    }

    fn revert_test_gen(profile_dir: &Path, n: usize, exit_code: i32) -> PathBuf {
        let gen_path = profile_dir.join(format!("{n}-link"));
        let dir = gen_path.join("core/activation");
        std::fs::create_dir_all(&dir).expect("BUG: mk core/activation");
        let entrypoint = dir.join("entrypoint");
        let script = format!(
            "#!/bin/sh\ndir=\"$(dirname \"$PROFILE_NEW_GENERATION\")\"\nprintf '%s\\n' {n} >> \"$dir/activation.log\"\nprintf 'new=%s old=%s\\n' \"$(basename \"$PROFILE_NEW_GENERATION\")\" \"$(basename \"$PROFILE_OLD_GENERATION\")\" >> \"$dir/env.log\"\nexit {exit_code}\n"
        );
        std::fs::write(&entrypoint, script).expect("BUG: write entrypoint");
        std::fs::set_permissions(&entrypoint, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: chmod entrypoint");
        gen_path
    }

    fn activation_log(profile_dir: &Path) -> Vec<usize> {
        match std::fs::read_to_string(profile_dir.join("activation.log")) {
            Ok(s) => s.lines().filter_map(|l| l.trim().parse().ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn env_log(profile_dir: &Path) -> Vec<String> {
        match std::fs::read_to_string(profile_dir.join("env.log")) {
            Ok(s) => s.lines().map(str::to_owned).collect(),
            Err(_) => Vec::new(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_failure_reverts_to_current() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        revert_test_gen(dir.path(), 1, 0);
        let g2 = revert_test_gen(dir.path(), 2, 42);
        std::os::unix::fs::symlink("1-link", dir.path().join("current")).expect("BUG: current");

        let err = activate_profile(dir.path(), 2, &g2, None)
            .await
            .expect_err("BUG: expected revert error");
        assert!(
            matches!(
                err,
                crate::activation::ActivationError::RevertedAfterFailure { reverted_to: 1, .. }
            ),
            "got {err:?}"
        );
        // Failed target ran first, then the reverted-to generation re-ran.
        assert_eq!(activation_log(dir.path()), vec![2, 1]);
        let current = std::fs::read_link(dir.path().join("current")).expect("BUG: current");
        assert_eq!(
            current,
            PathBuf::from("1-link"),
            "bmc-nix never writes current; the fake entrypoints leave it untouched"
        );
        assert!(
            dir.path().join("previous").symlink_metadata().is_err(),
            "bmc-nix must never write a previous symlink"
        );
        assert_eq!(
            env_log(dir.path()).last(),
            Some(&"new=1-link old=2-link".to_owned()),
            "the revert run must activate the old generation with PROFILE_OLD_GENERATION set to the failed target"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_double_failure_returns_revert_failed() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        revert_test_gen(dir.path(), 1, 7);
        let g2 = revert_test_gen(dir.path(), 2, 42);
        std::os::unix::fs::symlink("1-link", dir.path().join("current")).expect("BUG: current");

        let err = activate_profile(dir.path(), 2, &g2, None)
            .await
            .expect_err("BUG: expected revert error");
        assert!(
            matches!(err, crate::activation::ActivationError::RevertFailed { .. }),
            "got {err:?}"
        );
        assert_eq!(activation_log(dir.path()), vec![2, 1]);
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_revert_target_without_generation_is_revert_failed() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        // `current` resolves to a real directory whose name is not `<N>-link`,
        // so no generation number can be parsed from the revert target.
        let bogus = dir.path().join("bogus");
        std::fs::create_dir_all(&bogus).expect("BUG: mk bogus current dir");
        std::os::unix::fs::symlink("bogus", dir.path().join("current")).expect("BUG: current");
        let g2 = revert_test_gen(dir.path(), 2, 42);

        let err = activate_profile(dir.path(), 2, &g2, None)
            .await
            .expect_err("BUG: expected revert failure");
        assert!(
            matches!(err, crate::activation::ActivationError::RevertFailed { .. }),
            "got {err:?}"
        );
        // The unidentifiable target's entrypoint must never run.
        assert_eq!(activation_log(dir.path()), vec![2]);
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_target_is_current_propagates_raw_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g1 = revert_test_gen(dir.path(), 1, 42);
        std::os::unix::fs::symlink("1-link", dir.path().join("current")).expect("BUG: current");

        let err = activate_profile(dir.path(), 1, &g1, None)
            .await
            .expect_err("BUG: expected raw error");
        assert!(
            matches!(
                err,
                crate::activation::ActivationError::EntrypointFailed { exit_code: 42, .. }
            ),
            "got {err:?}"
        );
        assert!(
            dir.path().join("previous").symlink_metadata().is_err(),
            "bmc-nix must never write a previous symlink"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_no_current_propagates_raw_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g1 = revert_test_gen(dir.path(), 1, 42);

        let err = activate_profile(dir.path(), 1, &g1, None)
            .await
            .expect_err("BUG: expected raw error");
        assert!(
            matches!(
                err,
                crate::activation::ActivationError::EntrypointFailed { exit_code: 42, .. }
            ),
            "got {err:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_dangling_current_is_not_a_revert_target() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let g2 = revert_test_gen(dir.path(), 2, 42);
        std::os::unix::fs::symlink("gone-link", dir.path().join("current")).expect("BUG: current");

        let err = activate_profile(dir.path(), 2, &g2, None)
            .await
            .expect_err("BUG: expected raw error");
        assert!(
            matches!(
                err,
                crate::activation::ActivationError::EntrypointFailed { exit_code: 42, .. }
            ),
            "got {err:?}"
        );
        assert!(
            dir.path().join("previous").symlink_metadata().is_err(),
            "bmc-nix must never write a previous symlink"
        );
        let current = std::fs::read_link(dir.path().join("current")).expect("BUG: current");
        assert_eq!(current, PathBuf::from("gone-link"));
    }

    #[tokio::test]
    #[serial]
    async fn activate_profile_invalid_entrypoint_fails_without_revert() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        revert_test_gen(dir.path(), 1, 0);
        let g2 = dir.path().join("2-link");
        std::fs::create_dir_all(&g2).expect("BUG: mk gen dir");
        std::os::unix::fs::symlink("1-link", dir.path().join("current")).expect("BUG: current");

        let err = activate_profile(dir.path(), 2, &g2, None)
            .await
            .expect_err("BUG: expected entrypoint error");
        assert!(
            matches!(
                err,
                crate::activation::ActivationError::EntrypointNotFound { .. }
            ),
            "got {err:?}"
        );
        assert!(
            dir.path().join("previous").symlink_metadata().is_err(),
            "nothing ran, so nothing must be staged"
        );
        assert_eq!(
            activation_log(dir.path()),
            Vec::<usize>::new(),
            "current's entrypoint must not re-run"
        );
    }

    #[tokio::test]
    #[serial]
    async fn entrypoint_failure_carries_stderr_snippet() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let gen_dir = tmp.path().join("1-link");
        let dir = gen_dir.join("core/activation");
        std::fs::create_dir_all(&dir).expect("BUG: mk core/activation");
        std::fs::write(
            dir.join("entrypoint"),
            "#!/bin/sh\necho boundary check failed >&2\nexit 7\n",
        )
        .expect("BUG: write entrypoint");
        std::fs::set_permissions(
            dir.join("entrypoint"),
            std::fs::Permissions::from_mode(0o755),
        )
        .expect("BUG: chmod entrypoint");

        let err = activate_profile(tmp.path(), 1, &gen_dir, None)
            .await
            .expect_err("BUG: failing entrypoint must error");
        match err {
            crate::activation::ActivationError::EntrypointFailed {
                output, exit_code, ..
            } => {
                assert_eq!(exit_code, 7);
                assert!(output.contains("boundary check failed"), "got: {output:?}");
            }
            other @ (crate::activation::ActivationError::ReadDir { .. }
            | crate::activation::ActivationError::EntrypointSignaled { .. }
            | crate::activation::ActivationError::EntrypointExecute { .. }
            | crate::activation::ActivationError::EntrypointNotFound { .. }
            | crate::activation::ActivationError::NoGeneration { .. }
            | crate::activation::ActivationError::ResolveIo { .. }
            | crate::activation::ActivationError::RevertedAfterFailure { .. }
            | crate::activation::ActivationError::RevertFailed { .. }
            | crate::activation::ActivationError::ConsumeMarker { .. }
            | crate::activation::ActivationError::Lock(_)) => {
                panic!("expected EntrypointFailed, got: {other}")
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn entrypoint_signal_termination_is_captured() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let gen_dir = tmp.path().join("1-link");
        let dir = gen_dir.join("core/activation");
        std::fs::create_dir_all(&dir).expect("BUG: mk core/activation");
        std::fs::write(
            dir.join("entrypoint"),
            "#!/bin/sh\necho dying >&2\nkill -9 $$\n",
        )
        .expect("BUG: write entrypoint");
        std::fs::set_permissions(
            dir.join("entrypoint"),
            std::fs::Permissions::from_mode(0o755),
        )
        .expect("BUG: chmod entrypoint");

        let err = activate_profile(tmp.path(), 1, &gen_dir, None)
            .await
            .expect_err("BUG: signaled entrypoint must error");
        match err {
            crate::activation::ActivationError::EntrypointSignaled { output, .. } => {
                assert!(output.contains("dying"), "got: {output:?}");
            }
            other @ (crate::activation::ActivationError::ReadDir { .. }
            | crate::activation::ActivationError::EntrypointFailed { .. }
            | crate::activation::ActivationError::EntrypointExecute { .. }
            | crate::activation::ActivationError::EntrypointNotFound { .. }
            | crate::activation::ActivationError::NoGeneration { .. }
            | crate::activation::ActivationError::ResolveIo { .. }
            | crate::activation::ActivationError::RevertedAfterFailure { .. }
            | crate::activation::ActivationError::RevertFailed { .. }
            | crate::activation::ActivationError::ConsumeMarker { .. }
            | crate::activation::ActivationError::Lock(_)) => {
                panic!("expected EntrypointSignaled, got: {other}")
            }
        }
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
    async fn single_provider_nested_directory_links_highest_directory() {
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

        let a = output_dir.join("a");
        let meta = a.symlink_metadata().expect("BUG: stat a");
        assert!(
            meta.file_type().is_symlink(),
            "single-provider top-level directory should be a symlink"
        );
        assert_eq!(
            std::fs::read_link(&a).expect("BUG: read a symlink"),
            store.join("a")
        );
    }

    #[tokio::test]
    async fn directory_and_leaf_at_same_path_conflict() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store_a = tmp.path().join("store-a");
        create_fake_store(&store_a, &["a"]);

        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["a/child"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: create output");

        let result = build_symlink_tree(&output_dir, &packages).await;

        assert!(
            matches!(
                result,
                Err(BuildProfileError::Conflict { ref path, ref pkg_a, ref pkg_b })
                    if path == "a" && pkg_a == "pkg-a" && pkg_b == "pkg-b"
            ),
            "expected Conflict at a with package names, got {result:?}"
        );
    }

    #[tokio::test]
    async fn later_directory_provider_upgrades_linkable_directory_to_merge() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store_a = tmp.path().join("store-a");
        create_fake_store(&store_a, &["share/foo"]);

        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["share/bar"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: create output");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: shared directory should merge");

        let share = output_dir.join("share");
        let meta = share.symlink_metadata().expect("BUG: stat share");
        assert!(meta.is_dir(), "shared directory should be materialized");
        assert!(
            !meta.file_type().is_symlink(),
            "shared directory should be a real generation directory"
        );
        assert!(output_dir.join("share/foo").is_symlink());
        assert!(output_dir.join("share/bar").is_symlink());
    }

    // Package A exposes `share` as a symlink to a directory.
    // Package B contributes a real file under `share/`. The build
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
        let applications_link = output_dir.join("share/applications");
        assert!(
            applications_link
                .symlink_metadata()
                .expect("BUG: stat share/applications")
                .file_type()
                .is_symlink(),
            "share/applications should stay linked when it has a single provider"
        );
        assert_eq!(
            std::fs::read_link(&applications_link).expect("BUG: read share/applications symlink"),
            store_b.join("share/applications")
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
        let applications_link = output_dir.join("share/applications");
        assert!(
            applications_link
                .symlink_metadata()
                .expect("BUG: stat share/applications")
                .file_type()
                .is_symlink(),
            "share/applications should stay linked when it has a single provider"
        );
        assert_eq!(
            std::fs::read_link(&applications_link).expect("BUG: read share/applications symlink"),
            store_b.join("share/applications")
        );
    }

    // A file symlink inside a store path is preserved as a symlink in the
    // output, not resolved or copied.
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

    // When two packages provide the same file under a directory that was
    // previously a symlink-dir, report the conflict correctly.
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

    #[tokio::test]
    async fn single_provider_dir_symlink_cycle_is_linked_without_descent() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).expect("BUG: create store");
        std::os::unix::fs::symlink(".", store.join("self")).expect("BUG: create self symlink");

        let packages = vec![test_resolved_package(
            "pkg",
            store.to_str().expect("BUG: valid UTF-8"),
        )];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: single-provider directory cycle should be linked");

        let self_link = output_dir.join("self");
        assert!(
            self_link
                .symlink_metadata()
                .expect("BUG: stat self")
                .file_type()
                .is_symlink(),
            "single-provider cyclic directory should be emitted as a symlink"
        );
        assert_eq!(
            std::fs::read_link(&self_link).expect("BUG: read self symlink"),
            store.join("self")
        );
    }

    #[tokio::test]
    async fn merged_dir_symlink_cycle_returns_error() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store_a = tmp.path().join("store-a");
        std::fs::create_dir_all(store_a.join("share")).expect("BUG: create store-a share");
        std::os::unix::fs::symlink(".", store_a.join("share/loop"))
            .expect("BUG: create loop symlink");

        let store_b = tmp.path().join("store-b");
        create_fake_store(&store_b, &["share/loop/leaf"]);

        let packages = vec![
            test_resolved_package("pkg-a", store_a.to_str().expect("BUG: valid UTF-8")),
            test_resolved_package("pkg-b", store_b.to_str().expect("BUG: valid UTF-8")),
        ];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        let result = build_symlink_tree(&output_dir, &packages).await;

        assert!(
            matches!(result, Err(BuildProfileError::SymlinkCycle { .. })),
            "expected SymlinkCycle during merged-directory recursion, got {result:?}"
        );
    }

    // A mutual file-symlink loop (`a -> b`, `b -> a`) causes stat to fail with
    // ELOOP, which must be propagated as StatStorePath.
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

    // Two directory symlinks pointing at the same real directory are both kept
    // as generation-root links when the package is their only provider.
    #[tokio::test]
    async fn single_provider_diamond_dir_symlinks_stay_linked() {
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
            .expect("BUG: single-provider diamond links should build");

        for name in ["x", "y"] {
            let link = output_dir.join(name);
            assert!(
                link.symlink_metadata()
                    .expect("BUG: stat diamond symlink")
                    .file_type()
                    .is_symlink(),
                "{name} should stay linked when it has a single provider"
            );
            assert_eq!(
                std::fs::read_link(&link).expect("BUG: read diamond symlink"),
                store.join(name)
            );
        }
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

    // Test: store path has `outer -> dirA`, inside dirA there is `inner -> dirB`,
    // and dirB contains `leaf`. The build must keep the single-provider
    // `outer` entry linked instead of materializing it in the output tree.
    #[tokio::test]
    async fn single_provider_nested_dir_symlink_subtree_stays_linked() {
        let tmp = tempfile::tempdir().expect("BUG: should create tempdir");

        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).expect("BUG: create store");

        // dirB is a plain real directory with one file
        let dir_b = tmp.path().join("dirB");
        std::fs::create_dir_all(&dir_b).expect("BUG: create dirB");
        std::fs::write(dir_b.join("leaf"), "data").expect("BUG: write leaf");

        // dirA is a plain real directory whose `inner` entry is a symlink to dirB
        let dir_a = tmp.path().join("dirA");
        std::fs::create_dir_all(&dir_a).expect("BUG: create dirA");
        std::os::unix::fs::symlink(&dir_b, dir_a.join("inner")).expect("BUG: create inner -> dirB");

        // store/outer is a symlink to dirA
        std::os::unix::fs::symlink(&dir_a, store.join("outer")).expect("BUG: create outer -> dirA");

        let packages = vec![test_resolved_package(
            "pkg",
            store.to_str().expect("BUG: valid UTF-8"),
        )];

        let output_dir = tmp.path().join("output");
        std::fs::create_dir_all(&output_dir).expect("BUG: should create output dir");

        build_symlink_tree(&output_dir, &packages)
            .await
            .expect("BUG: single-provider nested dir symlink should stay linked");

        let outer = output_dir.join("outer");
        assert!(
            outer
                .symlink_metadata()
                .expect("BUG: stat outer")
                .file_type()
                .is_symlink(),
            "outer should stay linked when only one package provides it"
        );
        assert_eq!(
            std::fs::read_link(&outer).expect("BUG: read outer symlink"),
            store.join("outer")
        );
    }
}
