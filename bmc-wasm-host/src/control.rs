// Copyright (C) 2026  Braiins Systems s.r.o.

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use anyhow::Context as _;
use bmc_wasm_thin_protocol::{AckMsg, HelloMsg, recv_hello_with_fd, write_ack};

use crate::host::SharedHost;
use crate::render_target::EglRenderTargetFactory;
use crate::slot::WidgetSlot;

#[derive(Debug)]
pub struct ListenSocket {
    listener: UnixListener,
    path: PathBuf,
}

impl ListenSocket {
    pub fn bind(path: &Path) -> io::Result<Self> {
        match UnixListener::bind(path) {
            Ok(listener) => Ok(Self {
                listener,
                path: path.to_path_buf(),
            }),
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => match UnixStream::connect(path) {
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another bmc-wasm-host is alive on this socket",
                )),
                Err(connect_err) if connect_err.raw_os_error() == Some(libc::ECONNREFUSED) => {
                    let _ = std::fs::remove_file(path);
                    let listener = UnixListener::bind(path)?;
                    Ok(Self {
                        listener,
                        path: path.to_path_buf(),
                    })
                }
                Err(connect_err) => Err(io::Error::new(
                    connect_err.kind(),
                    format!("socket path exists but stale-check connect failed: {connect_err}"),
                )),
            },
            Err(e) => Err(e),
        }
    }

    #[must_use]
    pub fn as_listener(&self) -> &UnixListener {
        &self.listener
    }

    pub fn set_nonblocking(&self) -> io::Result<()> {
        self.listener.set_nonblocking(true)
    }
}

impl Drop for ListenSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "client ownership is transferred to the slot's control_socket field via try_clone; \
              passing by value signals intent to the caller that accept_and_load takes custody"
)]
pub fn accept_and_load(
    client: UnixStream,
    _shared: &mut SharedHost,
) -> Result<WidgetSlot, anyhow::Error> {
    client
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(anyhow::Error::from)?;
    let (msg, wayland_fd) = recv_hello_with_fd(&client).map_err(|e| {
        let _ = write_ack(&client, &AckMsg::Err(format!("recv: {e}")));
        e
    })?;
    client.set_read_timeout(None).map_err(anyhow::Error::from)?;

    let HelloMsg::Load { wasm_path } = msg;
    let path = PathBuf::from(&wasm_path);
    let peer_pid = peer_pid_of(&client).unwrap_or(0);

    let factory: Rc<dyn crate::render_target::RenderTargetFactory> =
        Rc::new(EglRenderTargetFactory);

    let slot =
        WidgetSlot::from_handshake(&path, wayland_fd, client.try_clone()?, peer_pid, factory)
            .map_err(|e| {
                let _ = write_ack(&client, &AckMsg::Err(format!("load: {e}")));
                e
            })?;

    write_ack(&client, &AckMsg::Ok)?;
    slot.control_socket
        .set_nonblocking(true)
        .context("control_socket.set_nonblocking(true)")?;
    Ok(slot)
}

pub fn try_handshake(client: &UnixStream) -> io::Result<HelloMsg> {
    let (msg, fd) = recv_hello_with_fd(client)?;
    drop(fd);
    write_ack(
        client,
        &AckMsg::Err("slot loading is only available through the Task 9 main loop".into()),
    )?;
    Ok(msg)
}

fn peer_pid_of(client: &UnixStream) -> Option<libc::pid_t> {
    use std::os::fd::AsRawFd;
    let fd = client.as_raw_fd();
    let mut creds: libc::ucred = unsafe { std::mem::zeroed() };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "size_of::<ucred> is small enough to fit in socklen_t (u32) on any realistic platform"
    )]
    let mut len: libc::socklen_t = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut creds).cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    (rc == 0).then_some(creds.pid)
}
