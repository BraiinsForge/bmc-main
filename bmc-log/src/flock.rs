// Copyright (C) 2026  Braiins Systems s.r.o.

//! Cross-process advisory locking via `flock(2)`.
//!
//! The rotated log files here and the Nix profile directory in `bmc-nix`
//! guard concurrent access with the same idiom: an exclusive lock on a
//! sidecar file, released explicitly on drop. This is the single shared
//! implementation so the semantics cannot drift between copies.

use std::io;
use std::os::fd::AsRawFd as _;

/// RAII guard holding an exclusive `flock(2)` on an open file.
///
/// The lock is explicitly released with `LOCK_UN` on drop before the file
/// descriptor is closed. This avoids a race where `close()` alone may not
/// release the lock atomically with respect to concurrent openers.
#[derive(Debug)]
pub struct FileLock {
    file: std::fs::File,
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = flock(&self.file, libc::LOCK_UN);
    }
}

/// Call `flock(2)` on `file` with the given flags.
fn flock(file: &std::fs::File, flags: libc::c_int) -> io::Result<()> {
    // SAFETY: `file.as_raw_fd()` returns a valid, open file descriptor owned
    // by `file`; the borrow keeps it alive for the duration of the call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), flags) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Take an exclusive lock on `file`, blocking until it is granted.
pub fn lock_file(file: std::fs::File) -> io::Result<FileLock> {
    flock(&file, libc::LOCK_EX)?;
    Ok(FileLock { file })
}

/// Try to take an exclusive lock on `file` without blocking.
///
/// Returns `Ok(None)` when another process already holds the lock.
pub fn try_lock_file(file: std::fs::File) -> io::Result<Option<FileLock>> {
    match flock(&file, libc::LOCK_EX | libc::LOCK_NB) {
        Ok(()) => Ok(Some(FileLock { file })),
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(err) => Err(err),
    }
}
