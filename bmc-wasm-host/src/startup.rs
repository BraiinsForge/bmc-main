// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host startup contract: validate an inherited release-lock fd against the lockfile path
//! derived from the control socket and decide whether to run or defer to an already-running
//! host that won the spawn race.

#![cfg(target_os = "linux")]

use std::fs;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::RawFd;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::control::ListenSocket;

#[derive(Debug)]
pub struct ReleaseLockFd {
    fd: RawFd,
}

impl ReleaseLockFd {
    pub fn validate(fd: RawFd, lockfile_path: &Path) -> Result<Self> {
        validate_release_lock_fd(fd, lockfile_path)?;
        set_cloexec(fd).context("set FD_CLOEXEC on release-lock-fd")?;
        Ok(Self { fd })
    }

    pub fn release(self) -> Result<()> {
        let fd = self.fd;
        std::mem::forget(self);
        let rc = unsafe { libc::close(fd) };
        if rc != 0 {
            return Err(io::Error::last_os_error()).context("close release-lock-fd");
        }
        Ok(())
    }
}

impl Drop for ReleaseLockFd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

pub fn validate_release_lock_fd(fd: RawFd, lockfile_path: &Path) -> Result<()> {
    if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
        return Err(io::Error::last_os_error()).context("validate release-lock-fd with F_GETFD");
    }
    let fd_stat = fstat_fd(fd).context("fstat release-lock-fd")?;
    let path_meta = fs::metadata(lockfile_path)
        .with_context(|| format!("stat lockfile {}", lockfile_path.display()))?;
    // libc::stat fields differ in width across targets: dev_t/ino_t are
    // u64 on x86_64 glibc and u32 on armv7 glibc. Widen both portably.
    #[expect(
        clippy::useless_conversion,
        reason = "st_dev/st_ino are u32 on armv7 glibc; identity on x86_64"
    )]
    let fd_dev = u64::from(fd_stat.st_dev);
    #[expect(
        clippy::useless_conversion,
        reason = "st_dev/st_ino are u32 on armv7 glibc; identity on x86_64"
    )]
    let fd_ino = u64::from(fd_stat.st_ino);
    if fd_dev != path_meta.dev() || fd_ino != path_meta.ino() {
        bail!(
            "release-lock-fd does not point at {}",
            lockfile_path.display()
        );
    }
    Ok(())
}

fn fstat_fd(fd: RawFd) -> io::Result<libc::stat> {
    let mut stat = MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { stat.assume_init() })
}

fn set_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[derive(Debug)]
pub enum StartupDecision {
    Run {
        listener: ListenSocket,
        release_lock: Option<ReleaseLockFd>,
    },
    AnotherHostAlive,
}

pub fn prepare_listener(
    socket_path: &Path,
    release_lock_fd: Option<RawFd>,
) -> Result<StartupDecision> {
    let lockfile_path = bmc_wasm_thin_protocol::derive_lockfile_path(socket_path);
    let release_lock = release_lock_fd
        .map(|fd| ReleaseLockFd::validate(fd, &lockfile_path))
        .transpose()?;
    match ListenSocket::bind(socket_path) {
        Ok(listener) => Ok(StartupDecision::Run {
            listener,
            release_lock,
        }),
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            drop(release_lock);
            Ok(StartupDecision::AnotherHostAlive)
        }
        Err(e) => Err(e).context("bind host control socket"),
    }
}
