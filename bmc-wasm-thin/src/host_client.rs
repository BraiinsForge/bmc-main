// Copyright (C) 2026  Braiins Systems s.r.o.

use std::io::{self, Read as _};
use std::os::fd::{AsFd as _, AsRawFd as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use bmc_wasm_thin_protocol::{AckDecoder, AckMsg, HelloMsg, send_hello_with_fd};

pub fn send_load_and_wait_ack(
    control: UnixStream,
    wasm: &Path,
    wayland: UnixStream,
    ack_wait: Duration,
) -> Result<UnixStream> {
    let msg = HelloMsg::Load {
        wasm_path: wasm.display().to_string(),
    };
    send_hello_with_fd(&control, &msg, wayland.as_fd()).context("send Hello with Wayland fd")?;
    drop(wayland);
    wait_for_ack(&control, ack_wait)?;
    Ok(control)
}

pub fn wait_for_ack(control: &UnixStream, ack_wait: Duration) -> Result<()> {
    control
        .set_nonblocking(true)
        .context("set host control socket nonblocking for Ack wait")?;
    let deadline = Instant::now() + ack_wait;
    let mut decoder = AckDecoder::new();
    loop {
        let now = Instant::now();
        if now >= deadline {
            bail!("timed out waiting for host Ack");
        }
        let timeout = deadline.saturating_duration_since(now);
        let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        let mut pfd = libc::pollfd {
            fd: control.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&raw mut pfd, 1, timeout_ms) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(err).context("poll while waiting for host Ack");
        }
        if rc == 0 {
            bail!("timed out waiting for host Ack");
        }
        if (pfd.revents & (libc::POLLERR | libc::POLLNVAL)) != 0 {
            bail!(
                "host control socket error while waiting for Ack: revents={}",
                pfd.revents
            );
        }
        if (pfd.revents & (libc::POLLIN | libc::POLLHUP)) != 0 {
            let mut buf = [0_u8; 256];
            let mut control_r = control;
            match control_r.read(&mut buf) {
                Ok(0) => bail!("EOF while waiting for host Ack"),
                Ok(n) => {
                    if let Some(ack) = decoder.push(&buf[..n])? {
                        return match ack {
                            AckMsg::Ok => Ok(()),
                            AckMsg::Err(msg) => bail!("host rejected widget: {msg}"),
                        };
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e).context("read host Ack"),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleExit {
    Clean,
    Signal,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "idle_until_exit owns the control socket and drops it on return to release the host link"
)]
pub fn idle_until_exit(control: UnixStream) -> Result<IdleExit> {
    control
        .set_nonblocking(true)
        .context("set control socket nonblocking for idle")?;
    let signals = crate::signal::SignalPipe::new()?;
    loop {
        let mut pfds = [
            libc::pollfd {
                fd: control.as_raw_fd(),
                events: libc::POLLIN
                    | libc::POLLHUP
                    | libc::POLLRDHUP
                    | libc::POLLERR
                    | libc::POLLNVAL,
                revents: 0,
            },
            libc::pollfd {
                fd: signals.read_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let nfds = libc::nfds_t::try_from(pfds.len())
            .expect("BUG: fixed idle poll fd array length fits nfds_t");
        let rc = unsafe { libc::poll(pfds.as_mut_ptr(), nfds, -1) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(err).context("poll thin idle fds");
        }
        if (pfds[1].revents & libc::POLLIN) != 0 {
            signals.drain();
            return Ok(IdleExit::Signal);
        }
        classify_idle_revents(pfds[0].revents)?;
        if (pfds[0].revents & libc::POLLIN) != 0 {
            let mut byte = [0_u8; 1];
            let mut control_r = &control;
            match control_r.read(&mut byte) {
                Ok(0) => return Ok(IdleExit::Clean),
                Ok(_) => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) if matches!(e.raw_os_error(), Some(libc::ECONNRESET | libc::EIO)) => {
                    return Err(e).context("control socket read failed during idle");
                }
                Err(e) => return Err(e).context("control socket read failed during idle"),
            }
        }
        if (pfds[0].revents & (libc::POLLHUP | libc::POLLRDHUP)) != 0 {
            return Ok(IdleExit::Clean);
        }
    }
}

pub fn classify_idle_revents(revents: i16) -> Result<()> {
    if (revents & (libc::POLLERR | libc::POLLNVAL)) != 0 {
        anyhow::bail!("control socket error during idle: revents={revents}");
    }
    Ok(())
}
