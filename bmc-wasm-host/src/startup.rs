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

//! Host startup contract: validate an inherited release-lock fd against the lockfile path
//! derived from the control socket and decide whether to run or defer to an already-running
//! host that won the spawn race.

#![cfg(target_os = "linux")]

use std::fs;
use std::io;
use std::os::fd::{AsFd as _, BorrowedFd, FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::control::ListenSocket;

#[derive(Debug)]
pub struct ReleaseLockFd {
    fd: OwnedFd,
}

impl ReleaseLockFd {
    pub fn validate(fd: RawFd, lockfile_path: &Path) -> Result<Self> {
        // The parent process inherited `fd` into us; we now take ownership so close is automatic.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        validate_release_lock_fd(owned.as_fd(), lockfile_path)?;
        set_cloexec(owned.as_fd()).context("set FD_CLOEXEC on release-lock-fd")?;
        Ok(Self { fd: owned })
    }

    pub fn release(self) -> Result<()> {
        drop(self.fd);
        Ok(())
    }
}

pub fn validate_release_lock_fd(fd: BorrowedFd<'_>, lockfile_path: &Path) -> Result<()> {
    rustix::io::fcntl_getfd(fd).context("validate release-lock-fd with F_GETFD")?;
    let fd_stat = fstat_fd(fd).context("fstat release-lock-fd")?;
    let path_meta = fs::metadata(lockfile_path)
        .with_context(|| format!("stat lockfile {}", lockfile_path.display()))?;
    if fd_stat.st_dev != path_meta.dev() || fd_stat.st_ino != path_meta.ino() {
        bail!(
            "release-lock-fd does not point at {}",
            lockfile_path.display()
        );
    }
    Ok(())
}

fn fstat_fd(fd: BorrowedFd<'_>) -> io::Result<rustix::fs::Stat> {
    rustix::fs::fstat(fd).map_err(Into::into)
}

fn set_cloexec(fd: BorrowedFd<'_>) -> io::Result<()> {
    let flags = rustix::io::fcntl_getfd(fd)?;
    rustix::io::fcntl_setfd(fd, flags | rustix::io::FdFlags::CLOEXEC)?;
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
