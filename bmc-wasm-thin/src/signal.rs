// Copyright (C) 2026  Braiins Systems s.r.o.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::Result;

static WRITE_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn handler(_sig: libc::c_int) {
    let fd = WRITE_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        unsafe {
            let byte: [u8; 1] = [1];
            let _ = libc::write(fd, byte.as_ptr().cast::<libc::c_void>(), 1);
        }
    }
}

pub struct SignalPipe {
    read: OwnedFd,
    write: OwnedFd,
    old_sigterm: libc::sigaction,
    old_sigint: libc::sigaction,
}

impl std::fmt::Debug for SignalPipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalPipe")
            .field("read_fd", &self.read.as_raw_fd())
            .field("write_fd", &self.write.as_raw_fd())
            .finish_non_exhaustive()
    }
}

impl SignalPipe {
    pub fn new() -> Result<Self> {
        let mut fds = [0; 2];
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if rc != 0 {
            return Err(io::Error::last_os_error().into());
        }
        // Publish the write fd before installing handlers so a signal cannot fire
        // with WRITE_FD still set to -1.
        WRITE_FD.store(fds[1], Ordering::Relaxed);
        unsafe {
            let old_sigterm = match install(libc::SIGTERM) {
                Ok(old) => old,
                Err(e) => {
                    WRITE_FD.store(-1, Ordering::Relaxed);
                    libc::close(fds[0]);
                    libc::close(fds[1]);
                    return Err(e);
                }
            };
            let old_sigint = match install(libc::SIGINT) {
                Ok(old) => old,
                Err(e) => {
                    WRITE_FD.store(-1, Ordering::Relaxed);
                    let _ = libc::sigaction(
                        libc::SIGTERM,
                        &raw const old_sigterm,
                        std::ptr::null_mut(),
                    );
                    libc::close(fds[0]);
                    libc::close(fds[1]);
                    return Err(e);
                }
            };
            Ok(Self {
                read: OwnedFd::from_raw_fd(fds[0]),
                write: OwnedFd::from_raw_fd(fds[1]),
                old_sigterm,
                old_sigint,
            })
        }
    }

    #[must_use]
    pub fn read_fd(&self) -> i32 {
        self.read.as_raw_fd()
    }

    pub fn drain(&self) {
        let mut buf = [0_u8; 64];
        loop {
            let rc = unsafe {
                libc::read(
                    self.read.as_raw_fd(),
                    buf.as_mut_ptr().cast::<libc::c_void>(),
                    buf.len(),
                )
            };
            if rc <= 0 {
                break;
            }
        }
    }
}

impl Drop for SignalPipe {
    fn drop(&mut self) {
        // Clear WRITE_FD before OwnedFd closes the write fd so a signal racing
        // with drop cannot write to a stale fd that may already be reused.
        WRITE_FD.store(-1, Ordering::Relaxed);
        unsafe {
            let _ = libc::sigaction(
                libc::SIGTERM,
                &raw const self.old_sigterm,
                std::ptr::null_mut(),
            );
            let _ = libc::sigaction(
                libc::SIGINT,
                &raw const self.old_sigint,
                std::ptr::null_mut(),
            );
        }
    }
}

unsafe fn install(sig: libc::c_int) -> Result<libc::sigaction> {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
    let handler_ptr = handler as *const () as usize;
    action.sa_sigaction = handler_ptr;
    unsafe { libc::sigemptyset(&raw mut action.sa_mask) };
    action.sa_flags = 0;
    if unsafe { libc::sigaction(sig, &raw const action, &raw mut old) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(old)
}
