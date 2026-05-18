// Copyright (C) 2026  Braiins Systems s.r.o.

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use bmc_wasm_thin_protocol::{AckMsg, HelloMsg, recv_hello_with_fd, write_ack};

pub const DEFAULT_SOCKET_PATH: &str = "/run/bmc/wasm-host-v1.sock";

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

pub fn try_handshake(client: &UnixStream) -> io::Result<HelloMsg> {
    let (msg, fd) = recv_hello_with_fd(client)?;
    drop(fd);
    write_ack(client, &AckMsg::Err("slot machinery not wired yet".into()))?;
    Ok(msg)
}
