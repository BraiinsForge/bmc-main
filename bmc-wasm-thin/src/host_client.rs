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

pub fn idle_until_exit(_control: UnixStream) -> Result<()> {
    anyhow::bail!("idle loop is implemented in Task 4")
}
