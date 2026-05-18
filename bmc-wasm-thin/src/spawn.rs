// Copyright (C) 2026  Braiins Systems s.r.o.

use std::ffi::CString;
use std::fs::{DirBuilder, File, OpenOptions, Permissions};
use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use rustix::fs::{FlockOperation, Mode, OFlags, flock};

use crate::args::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectFailure {
    Spawnable,
    Fatal,
}

#[must_use]
pub fn classify_connect_error(err: &io::Error) -> ConnectFailure {
    match err.raw_os_error() {
        Some(libc::ENOENT | libc::ECONNREFUSED) => ConnectFailure::Spawnable,
        _ => ConnectFailure::Fatal,
    }
}

pub fn ensure_socket_parent(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(parent)?;
        std::fs::set_permissions(parent, Permissions::from_mode(0o755))?;
    }
    Ok(())
}

// Open without O_CLOEXEC: the fd is inherited across exec into the spawned
// bmc-wasm-host as the release-lock-fd handoff.
fn open_owner_lock(path: &Path) -> io::Result<File> {
    let fd = rustix::fs::open(
        path,
        OFlags::CREATE | OFlags::RDWR,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(io::Error::from)?;
    Ok(File::from(fd))
}

fn open_shared_wait_lock(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)
}

pub fn wait_for_readiness(lockfile: &Path, host_wait: Duration) -> Result<()> {
    let lock = open_shared_wait_lock(lockfile).context("open readiness lockfile")?;
    let start = Instant::now();
    let deadline = Instant::now() + host_wait;
    loop {
        match flock(&lock, FlockOperation::NonBlockingLockShared) {
            Ok(()) => return Ok(()),
            Err(errno) => {
                let err = io::Error::from(errno);
                match err.raw_os_error() {
                    Some(libc::EWOULDBLOCK | libc::EINTR) => {}
                    _ => return Err(err).context("take shared readiness lock"),
                }
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for bmc-wasm-host readiness lock");
        }
        let sleep = if start.elapsed() < Duration::from_millis(250) {
            Duration::from_millis(25)
        } else {
            Duration::from_millis(100)
        };
        std::thread::sleep(sleep);
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for bmc-wasm-host readiness lock");
        }
    }
}

pub trait HostLauncher {
    fn before_spawn_owner_reconnect(&self, _config: &Config) -> Result<()> {
        Ok(())
    }

    fn spawn_host(&self, config: &Config, release_lock_fd: i32) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct DaemonHostLauncher;

impl HostLauncher for DaemonHostLauncher {
    fn spawn_host(&self, config: &Config, release_lock_fd: i32) -> Result<()> {
        spawn_daemon(config, release_lock_fd)
    }
}

pub fn connect_or_spawn(config: &Config) -> Result<UnixStream> {
    connect_or_spawn_with_launcher(config, &DaemonHostLauncher)
}

pub fn connect_or_spawn_with_launcher<L: HostLauncher>(
    config: &Config,
    launcher: &L,
) -> Result<UnixStream> {
    match UnixStream::connect(&config.host_socket) {
        Ok(stream) => {
            tracing::info!(
                host_socket = %config.host_socket.display(),
                "connected to existing bmc-wasm-host"
            );
            return Ok(stream);
        }
        Err(e) if classify_connect_error(&e) == ConnectFailure::Spawnable => {}
        Err(e) => {
            return Err(e).with_context(|| format!("connect {}", config.host_socket.display()));
        }
    }

    ensure_socket_parent(&config.host_socket).context("ensure host socket parent")?;
    ensure_socket_parent(&config.lockfile).context("ensure lockfile parent")?;
    let owner_lock = open_owner_lock(&config.lockfile).context("open owner lock")?;
    if let Err(errno) = flock(&owner_lock, FlockOperation::NonBlockingLockExclusive) {
        let err = io::Error::from(errno);
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            tracing::info!(
                lockfile = %config.lockfile.display(),
                "another thin owns host startup; waiting for readiness"
            );
            drop(owner_lock);
            wait_for_readiness(&config.lockfile, config.host_wait)?;
            return final_connect(config);
        }
        return Err(err).context("take exclusive spawn lock");
    }

    launcher.before_spawn_owner_reconnect(config)?;
    match UnixStream::connect(&config.host_socket) {
        Ok(stream) => {
            tracing::info!(
                host_socket = %config.host_socket.display(),
                "host appeared before this thin spawned one"
            );
            drop(owner_lock);
            return Ok(stream);
        }
        Err(e) if classify_connect_error(&e) == ConnectFailure::Spawnable => {}
        Err(e) => {
            return Err(e).with_context(|| format!("reconnect {}", config.host_socket.display()));
        }
    }

    tracing::info!(
        host_bin = %config.host_bin.display(),
        host_socket = %config.host_socket.display(),
        release_lock_fd = owner_lock.as_raw_fd(),
        "spawning bmc-wasm-host"
    );
    launcher.spawn_host(config, owner_lock.as_raw_fd())?;
    drop(owner_lock);
    wait_for_readiness(&config.lockfile, config.host_wait)?;
    final_connect(config)
}

fn final_connect(config: &Config) -> Result<UnixStream> {
    match UnixStream::connect(&config.host_socket) {
        Ok(stream) => {
            tracing::info!(
                host_socket = %config.host_socket.display(),
                "connected to ready bmc-wasm-host"
            );
            Ok(stream)
        }
        Err(e) if classify_connect_error(&e) == ConnectFailure::Spawnable => {
            anyhow::bail!(
                "bmc-wasm-host released readiness lock but {} is not accepting connections: {e}",
                config.host_socket.display(),
            );
        }
        Err(e) => Err(e).with_context(|| format!("connect {}", config.host_socket.display())),
    }
}

pub fn spawn_daemon(config: &Config, release_lock_fd: i32) -> Result<()> {
    spawn_daemon_with_env(config, release_lock_fd, &[])
}

pub fn spawn_daemon_with_env(
    config: &Config,
    release_lock_fd: i32,
    extra_env: &[(&str, String)],
) -> Result<()> {
    let mut pipe = [0_i32; 2];
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error()).context("create daemon setup pipe");
    }

    let first = unsafe { libc::fork() };
    if first < 0 {
        unsafe {
            libc::close(pipe[0]);
            libc::close(pipe[1]);
        }
        return Err(io::Error::last_os_error()).context("first fork for bmc-wasm-host");
    }
    if first == 0 {
        unsafe {
            libc::close(pipe[0]);
        }
        daemon_intermediate_child(config, release_lock_fd, pipe[1], extra_env);
    }

    unsafe {
        libc::close(pipe[1]);
    }
    let mut status: libc::c_int = 0;
    if unsafe { libc::waitpid(first, &raw mut status, 0) } < 0 {
        unsafe {
            libc::close(pipe[0]);
        }
        return Err(io::Error::last_os_error()).context("wait for intermediate daemon child");
    }
    let mut errno_buf = [0_u8; std::mem::size_of::<i32>()];
    let n = unsafe {
        libc::read(
            pipe[0],
            errno_buf.as_mut_ptr().cast::<libc::c_void>(),
            errno_buf.len(),
        )
    };
    unsafe {
        libc::close(pipe[0]);
    }
    if n > 0 {
        let errno = i32::from_ne_bytes(errno_buf);
        return Err(io::Error::from_raw_os_error(errno)).context("daemon setup failed");
    }
    if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
        Ok(())
    } else {
        anyhow::bail!("intermediate daemon child exited unexpectedly: status={status}");
    }
}

fn write_errno_and_exit(pipe_w: i32, errno: i32) -> ! {
    let bytes = errno.to_ne_bytes();
    unsafe {
        let _ = libc::write(pipe_w, bytes.as_ptr().cast::<libc::c_void>(), bytes.len());
        libc::close(pipe_w);
        libc::_exit(127);
    }
}

fn daemon_intermediate_child(
    config: &Config,
    release_lock_fd: i32,
    pipe_w: i32,
    extra_env: &[(&str, String)],
) -> ! {
    let sid = unsafe { libc::setsid() };
    if sid < 0 {
        let errno = io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EIO);
        write_errno_and_exit(pipe_w, errno);
    }
    let grand = unsafe { libc::fork() };
    if grand < 0 {
        let errno = io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EIO);
        write_errno_and_exit(pipe_w, errno);
    }
    if grand > 0 {
        unsafe {
            libc::close(pipe_w);
            libc::_exit(0);
        }
    }
    unsafe {
        libc::close(pipe_w);
    }
    daemon_grandchild(config, release_lock_fd, extra_env);
}

fn daemon_grandchild(config: &Config, release_lock_fd: i32, extra_env: &[(&str, String)]) -> ! {
    close_fds_except(release_lock_fd);
    let devnull = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDWR) };
    if devnull >= 0 {
        let r0 = unsafe { libc::dup2(devnull, 0) };
        let r1 = unsafe { libc::dup2(devnull, 1) };
        let r2 = unsafe { libc::dup2(devnull, 2) };
        if devnull > 2 && devnull != release_lock_fd {
            unsafe {
                libc::close(devnull);
            }
        }
        if r0 < 0 || r1 < 0 || r2 < 0 {
            unsafe { libc::_exit(127) };
        }
    }
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&raw mut sa.sa_mask);
        sa.sa_flags = 0;
        libc::sigaction(libc::SIGTERM, &raw const sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &raw const sa, std::ptr::null_mut());
    }
    for (k, v) in extra_env {
        let ck = CString::new(*k).expect("BUG: env key contains NUL");
        let cv = CString::new(v.as_bytes()).expect("BUG: env value contains NUL");
        unsafe {
            libc::setenv(ck.as_ptr(), cv.as_ptr(), 1);
        }
    }

    let prog =
        CString::new(config.host_bin.as_os_str().as_bytes()).expect("BUG: host_bin contains NUL");
    let arg_host_socket = CString::new("--host-socket").expect("BUG: literal CString");
    let host_socket = CString::new(config.host_socket.as_os_str().as_bytes())
        .expect("BUG: host_socket contains NUL");
    let arg_release_lock_fd = CString::new("--release-lock-fd").expect("BUG: literal CString");
    let release_lock_fd_str =
        CString::new(release_lock_fd.to_string()).expect("BUG: release_lock_fd string");
    let argv: [*const libc::c_char; 6] = [
        prog.as_ptr(),
        arg_host_socket.as_ptr(),
        host_socket.as_ptr(),
        arg_release_lock_fd.as_ptr(),
        release_lock_fd_str.as_ptr(),
        std::ptr::null(),
    ];

    let use_execv =
        config.host_bin.is_absolute() || config.host_bin.as_os_str().as_bytes().contains(&b'/');
    unsafe {
        if use_execv {
            libc::execv(prog.as_ptr(), argv.as_ptr());
        } else {
            libc::execvp(prog.as_ptr(), argv.as_ptr());
        }
        libc::_exit(127);
    }
}

fn close_fds_except(keep_fd: i32) {
    let max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    let max = if max > 0 { max } else { 1024 };
    for fd in 3..max {
        let fd_i32 = i32::try_from(fd).expect("BUG: fd range fits i32");
        if fd_i32 != keep_fd {
            unsafe {
                libc::close(fd_i32);
            }
        }
    }
}
