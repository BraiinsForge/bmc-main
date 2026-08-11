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
use crate::ownership::{
    CompositorIdentity, RecordStatus, commit_record, current_compositor_identity,
    parse_proc_stat_starttime, proc_stat_is_zombie, read_record_status, remove_record,
};

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

#[derive(Debug, Clone, Copy)]
struct LifecycleDeadline {
    at: Instant,
}

impl LifecycleDeadline {
    fn new(wait: Duration) -> Result<Self> {
        let at = Instant::now()
            .checked_add(wait)
            .context("host-wait duration exceeds the monotonic clock range")?;
        Ok(Self { at })
    }

    fn remaining(self, phase: &str) -> Result<Duration> {
        self.at
            .checked_duration_since(Instant::now())
            .with_context(|| format!("timed out during bmc-wasm-host {phase}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnershipVerdict {
    Reuse,
    Replace,
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
    let deadline = LifecycleDeadline::new(host_wait)?;
    wait_for_readiness_until(lockfile, deadline)
}

fn wait_for_readiness_until(lockfile: &Path, deadline: LifecycleDeadline) -> Result<()> {
    let lock = open_shared_wait_lock(lockfile).context("open readiness lockfile")?;
    let start = Instant::now();
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
        let remaining = deadline.remaining("readiness wait")?;
        let sleep = if start.elapsed() < Duration::from_millis(250) {
            Duration::from_millis(25)
        } else {
            Duration::from_millis(100)
        }
        .min(remaining);
        std::thread::sleep(sleep);
    }
}

fn take_exclusive_spawn_lock(lock: &File, deadline: LifecycleDeadline) -> Result<()> {
    loop {
        match flock(lock, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => return Ok(()),
            Err(errno) => {
                let error = io::Error::from(errno);
                match error.raw_os_error() {
                    Some(libc::EWOULDBLOCK | libc::EINTR) => {}
                    _ => return Err(error).context("take exclusive spawn lock"),
                }
            }
        }
        let remaining = deadline.remaining("spawn-lock wait")?;
        std::thread::sleep(Duration::from_millis(25).min(remaining));
    }
}

pub trait HostLauncher {
    fn spawn_host(&self, config: &Config, release_lock_fd: i32) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct DaemonHostLauncher;

impl HostLauncher for DaemonHostLauncher {
    fn spawn_host(&self, config: &Config, release_lock_fd: i32) -> Result<()> {
        spawn_daemon(config, release_lock_fd)
    }
}

pub trait ForeignHostTerminator {
    fn terminate(
        &self,
        stream: UnixStream,
        socket: &Path,
        lifecycle_deadline: Instant,
        term_wait: Duration,
        kill_wait: Duration,
    ) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct SignalForeignHostTerminator;

impl ForeignHostTerminator for SignalForeignHostTerminator {
    fn terminate(
        &self,
        stream: UnixStream,
        socket: &Path,
        lifecycle_deadline: Instant,
        term_wait: Duration,
        kill_wait: Duration,
    ) -> Result<()> {
        terminate_foreign_host(&stream, socket, lifecycle_deadline, term_wait, kill_wait)
    }
}

#[must_use]
pub fn foreign_host_signal_waits(host_wait: Duration) -> (Duration, Duration) {
    const TERM_WAIT_DIVISOR: u32 = 2;
    const KILL_WAIT_DIVISOR: u32 = 4;

    (host_wait / TERM_WAIT_DIVISOR, host_wait / KILL_WAIT_DIVISOR)
}

pub fn connect_or_spawn(config: &Config) -> Result<UnixStream> {
    connect_or_spawn_with_launcher(config, &DaemonHostLauncher)
}

pub fn connect_or_spawn_with_launcher<L: HostLauncher>(
    config: &Config,
    launcher: &L,
) -> Result<UnixStream> {
    connect_or_spawn_with_launcher_and_terminator(config, launcher, &SignalForeignHostTerminator)
}

pub fn connect_or_spawn_with_launcher_and_terminator<L: HostLauncher, T: ForeignHostTerminator>(
    config: &Config,
    launcher: &L,
    terminator: &T,
) -> Result<UnixStream> {
    let deadline = LifecycleDeadline::new(config.host_wait)?;
    ensure_socket_parent(&config.host_socket).context("ensure host socket parent")?;
    ensure_socket_parent(&config.lockfile).context("ensure lockfile parent")?;
    ensure_socket_parent(&config.owner_record).context("ensure ownership record parent")?;
    let owner_lock = open_owner_lock(&config.lockfile).context("open owner lock")?;
    take_exclusive_spawn_lock(&owner_lock, deadline)?;
    let current = current_compositor_identity().context("read compositor ownership identity")?;
    match UnixStream::connect(&config.host_socket) {
        Ok(stream) => {
            if ownership_verdict(&config.owner_record, &current)? == OwnershipVerdict::Reuse {
                tracing::info!(
                    host_socket = %config.host_socket.display(),
                    "connected to compositor-owned bmc-wasm-host"
                );
                drop(owner_lock);
                return Ok(stream);
            }
            let (term_wait, kill_wait) = foreign_host_signal_waits(config.host_wait);
            terminator.terminate(
                stream,
                &config.host_socket,
                deadline.at,
                term_wait,
                kill_wait,
            )?;
        }
        Err(e) if classify_connect_error(&e) == ConnectFailure::Spawnable => {}
        Err(e) => {
            return Err(e).with_context(|| format!("connect {}", config.host_socket.display()));
        }
    }

    commit_record(&config.owner_record, &current).with_context(|| {
        format!(
            "commit bmc-wasm-host ownership record {}",
            config.owner_record.display()
        )
    })?;
    tracing::info!(
        host_bin = %config.host_bin.display(),
        host_socket = %config.host_socket.display(),
        release_lock_fd = owner_lock.as_raw_fd(),
        "spawning bmc-wasm-host"
    );
    if let Err(error) = launcher.spawn_host(config, owner_lock.as_raw_fd()) {
        if let Err(cleanup_error) = remove_record(&config.owner_record) {
            tracing::warn!(
                owner_record = %config.owner_record.display(),
                %cleanup_error,
                "failed to remove ownership record after host spawn failure"
            );
        }
        return Err(error).context("spawn bmc-wasm-host after committing ownership record");
    }
    let readiness_deadline = LifecycleDeadline::new(config.host_wait)?;
    drop(owner_lock);
    wait_for_readiness_until(&config.lockfile, readiness_deadline)?;
    final_connect(config, readiness_deadline)
}

fn ownership_verdict(
    owner_record: &Path,
    current: &CompositorIdentity,
) -> Result<OwnershipVerdict> {
    match read_record_status(owner_record, current) {
        RecordStatus::Match => Ok(OwnershipVerdict::Reuse),
        RecordStatus::Missing => {
            tracing::info!(
                owner_record = %owner_record.display(),
                "bmc-wasm-host ownership record is missing"
            );
            Ok(OwnershipVerdict::Replace)
        }
        RecordStatus::Malformed { error } => {
            tracing::warn!(
                owner_record = %owner_record.display(),
                %error,
                "bmc-wasm-host ownership record is malformed"
            );
            Ok(OwnershipVerdict::Replace)
        }
        RecordStatus::Unreadable { error } => {
            tracing::warn!(
                owner_record = %owner_record.display(),
                %error,
                "bmc-wasm-host ownership record is unreadable"
            );
            Ok(OwnershipVerdict::Replace)
        }
        RecordStatus::Mismatch { recorded } => {
            if recorded.boot_id == current.boot_id
                && process_state(ProcessIdentity {
                    pid: recorded.pid,
                    starttime: recorded.starttime,
                })? == ProcessState::Running
            {
                tracing::info!(
                    owner_record = %owner_record.display(),
                    recorded_pid = recorded.pid,
                    recorded_starttime = recorded.starttime,
                    "connected host's recorded compositor remains alive"
                );
                return Ok(OwnershipVerdict::Reuse);
            }
            tracing::info!(
                owner_record = %owner_record.display(),
                recorded_boot_id = %recorded.boot_id,
                recorded_pid = recorded.pid,
                recorded_starttime = recorded.starttime,
                "bmc-wasm-host belongs to another compositor"
            );
            Ok(OwnershipVerdict::Replace)
        }
    }
}

fn final_connect(config: &Config, deadline: LifecycleDeadline) -> Result<UnixStream> {
    deadline.remaining("final connect")?;
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

fn terminate_foreign_host(
    stream: &UnixStream,
    socket: &Path,
    lifecycle_deadline: Instant,
    term_wait: Duration,
    kill_wait: Duration,
) -> Result<()> {
    use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};

    let peer_pid = getsockopt(&stream, PeerCredentials)
        .context("read bmc-wasm-host SO_PEERCRED")?
        .pid();
    if peer_pid <= 0 {
        anyhow::bail!(
            "bmc-wasm-host {} reported invalid peer pid {peer_pid}",
            socket.display()
        );
    }
    let Some(peer) = process_identity(peer_pid)? else {
        return Ok(());
    };
    tracing::warn!(
        host_socket = %socket.display(),
        peer_pid,
        "terminating bmc-wasm-host owned by another compositor"
    );
    let Some(term_deadline) = clipped_deadline(lifecycle_deadline, term_wait) else {
        anyhow::bail!(
            "bmc-wasm-host pid {peer_pid} on {} has no SIGTERM observation budget remaining",
            socket.display()
        );
    };
    if send_signal(peer_pid, libc::SIGTERM)? == SignalResult::AlreadyExited {
        return Ok(());
    }
    if wait_for_process_exit(peer, term_deadline)? == ProcessState::Exited {
        return Ok(());
    }
    let Some(kill_deadline) = clipped_deadline(lifecycle_deadline, kill_wait) else {
        anyhow::bail!(
            "bmc-wasm-host pid {peer_pid} on {} has no SIGKILL observation budget remaining",
            socket.display()
        );
    };
    if send_signal(peer_pid, libc::SIGKILL)? == SignalResult::AlreadyExited {
        return Ok(());
    }
    if wait_for_process_exit(peer, kill_deadline)? == ProcessState::Exited {
        return Ok(());
    }
    anyhow::bail!(
        "bmc-wasm-host pid {peer_pid} on {} survived SIGKILL observation",
        socket.display()
    )
}

fn clipped_deadline(lifecycle_deadline: Instant, phase_wait: Duration) -> Option<Instant> {
    let now = Instant::now();
    let remaining = lifecycle_deadline.checked_duration_since(now)?;
    let wait = phase_wait.min(remaining);
    if wait.is_zero() {
        return None;
    }
    now.checked_add(wait)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignalResult {
    Delivered,
    AlreadyExited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessIdentity {
    pid: libc::pid_t,
    starttime: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessState {
    Running,
    Exited,
}

fn send_signal(pid: libc::pid_t, signal: libc::c_int) -> Result<SignalResult> {
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(SignalResult::Delivered);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(SignalResult::AlreadyExited);
    }
    Err(error).with_context(|| format!("send signal {signal} to bmc-wasm-host pid {pid}"))
}

fn process_identity(pid: libc::pid_t) -> Result<Option<ProcessIdentity>> {
    let Some(stat) = process_stat(pid)? else {
        return Ok(None);
    };
    Ok(Some(ProcessIdentity {
        pid,
        starttime: parse_proc_stat_starttime(&stat)
            .with_context(|| format!("parse bmc-wasm-host process stat for pid {pid}"))?,
    }))
}

fn process_stat(pid: libc::pid_t) -> Result<Option<String>> {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => Ok(Some(stat)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read process stat for pid {pid}")),
    }
}

fn process_state(identity: ProcessIdentity) -> Result<ProcessState> {
    let Some(stat) = process_stat(identity.pid)? else {
        return Ok(ProcessState::Exited);
    };
    if proc_stat_is_zombie(&stat)
        .with_context(|| format!("parse process stat for pid {}", identity.pid))?
    {
        return Ok(ProcessState::Exited);
    }
    let current = ProcessIdentity {
        pid: identity.pid,
        starttime: parse_proc_stat_starttime(&stat)
            .with_context(|| format!("parse process stat for pid {}", identity.pid))?,
    };
    Ok(if current == identity {
        ProcessState::Running
    } else {
        ProcessState::Exited
    })
}

fn wait_for_process_exit(identity: ProcessIdentity, deadline: Instant) -> Result<ProcessState> {
    loop {
        if process_state(identity)? == ProcessState::Exited {
            return Ok(ProcessState::Exited);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(ProcessState::Running);
        };
        std::thread::sleep(Duration::from_millis(25).min(remaining));
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
    let setup_errno = read_daemon_setup_errno(|buffer| {
        let read = unsafe {
            libc::read(
                pipe[0],
                buffer.as_mut_ptr().cast::<libc::c_void>(),
                buffer.len(),
            )
        };
        if read < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(usize::try_from(read).expect("BUG: nonnegative read length fits usize"))
        }
    });
    unsafe {
        libc::close(pipe[0]);
    }
    if let Some(errno) = setup_errno.context("read daemon setup observer")? {
        return Err(io::Error::from_raw_os_error(errno)).context("daemon setup failed");
    }
    if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
        Ok(())
    } else {
        anyhow::bail!("intermediate daemon child exited unexpectedly: status={status}");
    }
}

fn read_daemon_setup_errno(
    mut read: impl FnMut(&mut [u8]) -> io::Result<usize>,
) -> io::Result<Option<i32>> {
    let mut buffer = [0_u8; std::mem::size_of::<i32>()];
    loop {
        match read(&mut buffer) {
            Ok(0) => return Ok(None),
            Ok(n) if n == buffer.len() => return Ok(Some(i32::from_ne_bytes(buffer))),
            Ok(n) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("daemon setup observer returned {n} bytes"),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
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
    daemon_grandchild(config, release_lock_fd, pipe_w, extra_env);
}

fn daemon_grandchild(
    config: &Config,
    release_lock_fd: i32,
    pipe_w: i32,
    extra_env: &[(&str, String)],
) -> ! {
    close_fds_except(&[release_lock_fd, pipe_w]);
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
            let errno = io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(libc::EIO);
            write_errno_and_exit(pipe_w, errno);
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

    // The O_CLOEXEC pipe write-end doubles as the exec observer:
    // a successful exec closes it (the launcher reads EOF), a failed exec
    // reports errno — so a missing host binary fails the spawn loudly
    // instead of surfacing as a later connect error.
    let use_execv =
        config.host_bin.is_absolute() || config.host_bin.as_os_str().as_bytes().contains(&b'/');
    unsafe {
        if use_execv {
            libc::execv(prog.as_ptr(), argv.as_ptr());
        } else {
            libc::execvp(prog.as_ptr(), argv.as_ptr());
        }
    }
    let errno = io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO);
    write_errno_and_exit(pipe_w, errno);
}

fn close_fds_except(keep_fds: &[i32]) {
    let max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    let max = if max > 0 { max } else { 1024 };
    for fd in 3..max {
        let fd_i32 = i32::try_from(fd).expect("BUG: fd range fits i32");
        if !keep_fds.contains(&fd_i32) {
            unsafe {
                libc::close(fd_i32);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::read_daemon_setup_errno;

    #[test]
    fn daemon_setup_observer_retries_interrupted_read() {
        let mut interrupted = true;
        let errno = read_daemon_setup_errno(|buffer| {
            if interrupted {
                interrupted = false;
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            buffer.copy_from_slice(&libc::ENOENT.to_ne_bytes());
            Ok(buffer.len())
        })
        .expect("interrupted observer read should be retried");

        assert_eq!(errno, Some(libc::ENOENT));
    }

    #[test]
    fn daemon_setup_observer_propagates_read_error() {
        let error = read_daemon_setup_errno(|_| Err(io::Error::from_raw_os_error(libc::EIO)))
            .expect_err("observer read failure must fail daemon setup");

        assert_eq!(error.raw_os_error(), Some(libc::EIO));
    }
}
