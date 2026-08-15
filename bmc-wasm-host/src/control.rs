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
    shared: &mut SharedHost,
) -> Result<WidgetSlot, anyhow::Error> {
    let peer_pid = peer_pid_of(&client);
    tracing::info!(?peer_pid, "thin control connection accepted");
    client
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(anyhow::Error::from)?;
    let (msg, wayland_fd) = recv_hello_with_fd(&client).map_err(|e| {
        let _ = write_ack(&client, &AckMsg::Err(format!("recv: {e}")));
        e
    })?;
    client.set_read_timeout(None).map_err(anyhow::Error::from)?;

    let HelloMsg::Load {
        wasm_path,
        asset_root,
    } = msg;
    let path = PathBuf::from(&wasm_path);
    let asset_root = asset_root.map(PathBuf::from);
    tracing::info!(
        ?peer_pid,
        wasm = %path.display(),
        "thin requested wasm load"
    );

    let factory: Rc<dyn crate::render_target::RenderTargetFactory> =
        Rc::new(EglRenderTargetFactory);

    // Tag load-time logs (incl. a widget's panic-hook output) with the widget,
    // so a panic during load names the widget, not just the host's target.
    let wasm = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("widget");
    let slot = {
        let _span = tracing::info_span!("widget", wasm).entered();
        WidgetSlot::from_handshake(
            &path,
            asset_root.as_deref(),
            shared,
            wayland_fd,
            client.try_clone()?,
            peer_pid,
            factory,
        )
        .map_err(|e| {
            let _ = write_ack(&client, &AckMsg::Err(format!("load: {e}")));
            e
        })?
    };

    write_ack(&client, &AckMsg::Ok)?;
    tracing::info!(
        ?peer_pid,
        wasm = %path.display(),
        "widget load acknowledged"
    );
    slot.control_socket
        .set_nonblocking(true)
        .context("control_socket.set_nonblocking(true)")?;
    Ok(slot)
}

fn peer_pid_of(client: &UnixStream) -> Option<libc::pid_t> {
    use nix::sys::socket::getsockopt;
    use nix::sys::socket::sockopt::PeerCredentials;
    getsockopt(client, PeerCredentials)
        .ok()
        .map(|creds| creds.pid())
}
