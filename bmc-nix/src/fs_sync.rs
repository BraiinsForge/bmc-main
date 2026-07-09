// Copyright (C) 2026  Braiins Systems s.r.o.

//! Durability helpers for rename-published names.
//!
//! The corruption-safety contract: make contents durable
//! (`syncfs(2)` on the containing filesystem) *before* a publishing
//! rename, and fsync the parent directory *after* it so the rename
//! itself is durable before success is reported.

use std::path::Path;

/// Flush the whole filesystem containing `path` to stable storage.
///
/// `syncfs(2)` is used instead of walking the tree with per-file
/// `fsync`: store trees contain thousands of entries, symlinks cannot
/// be fsynced individually, and the data partition is dedicated, so
/// flushing it wholesale is exactly what is wanted. Blocking — async
/// callers must use [`sync_filesystem_of_blocking`].
#[cfg(target_os = "linux")]
pub fn sync_filesystem_of(path: &Path) -> Result<(), std::io::Error> {
    use std::os::fd::AsRawFd;

    let dir = std::fs::File::open(path)?;
    // SAFETY: `dir.as_raw_fd()` returns a valid, open file descriptor
    // owned by `dir`, which is borrowed for the duration of the call.
    let ret = unsafe { libc::syncfs(dir.as_raw_fd()) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// `syncfs(2)` exists only on Linux, so this stub stands in on other
/// targets. It is never reached in practice — no `bmc-nix` binary is
/// built off Linux — but keeps the crate compilable there, matching the
/// `decode_dev_t` rationale that this Linux-only code is still
/// type-checked as part of the workspace.
#[cfg(not(target_os = "linux"))]
pub fn sync_filesystem_of(_path: &Path) -> Result<(), std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "syncfs is only available on Linux",
    ))
}

/// Run [`sync_filesystem_of`] on a blocking thread, for async callers.
///
/// `syncfs` over a fresh store can take seconds, so it must not run on a
/// runtime worker. This is the only supported way to call it from an
/// async context.
pub async fn sync_filesystem_of_blocking(path: &Path) -> Result<(), std::io::Error> {
    let path = path.to_path_buf();

    tokio::task::spawn_blocking(move || sync_filesystem_of(&path))
        .await
        .expect("BUG: sync task should not panic")
}

/// Fsync a directory so a rename (or unlink) inside it is durable.
pub fn fsync_dir(path: &Path) -> Result<(), std::io::Error> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn sync_filesystem_of_succeeds_on_existing_dir() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        sync_filesystem_of(tmp.path()).expect("BUG: syncfs on a real directory must succeed");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sync_filesystem_of_propagates_missing_path() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let err = sync_filesystem_of(&tmp.path().join("missing"))
            .expect_err("BUG: syncfs on a missing path must fail");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn fsync_dir_succeeds_on_existing_dir() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        fsync_dir(tmp.path()).expect("BUG: fsync on a real directory must succeed");
    }

    #[test]
    fn fsync_dir_propagates_missing_path() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let err = fsync_dir(&tmp.path().join("missing"))
            .expect_err("BUG: fsync on a missing path must fail");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
