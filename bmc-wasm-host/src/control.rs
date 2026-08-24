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

use std::io::{self, Read as _};
use std::os::fd::{AsRawFd as _, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use bmc_wasm_thin_protocol::{AckMsg, HelloMsg, HelloReceiveStatus, HelloReceiver, write_ack};
use bmc_widget::surface::{PendingDeckWidgetSurfaceAdvance, PendingDeckWidgetSurfaceClient};

use crate::host::SharedHost;
use crate::render_target::EglRenderTargetFactory;
use crate::slot::WidgetSlot;

fn parse_widget_key(value: &str) -> anyhow::Result<bmc_widget_protocol::WidgetInstanceKey> {
    value
        .parse()
        .map_err(|err| anyhow::anyhow!("invalid widget key: {err}"))
}

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

const HELLO_TIMEOUT: Duration = Duration::from_millis(250);

pub(crate) enum PendingAdmission {
    AwaitHello {
        client: UnixStream,
        receiver: HelloReceiver,
        deadline: Instant,
        peer_pid: Option<libc::pid_t>,
    },
    AwaitConfigure {
        client: UnixStream,
        surface: Box<PendingDeckWidgetSurfaceClient>,
        wasm_path: PathBuf,
        asset_root: Option<PathBuf>,
        peer_pid: Option<libc::pid_t>,
    },
}

pub(crate) enum AdmissionAdvance {
    Pending(PendingAdmission),
    Loaded(Box<WidgetSlot>),
    Rejected(anyhow::Error),
}

impl PendingAdmission {
    pub(crate) fn new(client: UnixStream, now: Instant) -> anyhow::Result<Self> {
        let peer_pid = peer_pid_of(&client);
        tracing::info!(?peer_pid, "thin control connection accepted");
        client
            .set_nonblocking(true)
            .context("control socket set_nonblocking(true)")?;
        Ok(Self::AwaitHello {
            client,
            receiver: HelloReceiver::new(),
            deadline: now + HELLO_TIMEOUT,
            peer_pid,
        })
    }

    pub(crate) fn control_fd(&self) -> RawFd {
        match self {
            Self::AwaitHello { client, .. } | Self::AwaitConfigure { client, .. } => {
                client.as_raw_fd()
            }
        }
    }

    pub(crate) fn wayland_fd(&self) -> Option<RawFd> {
        match self {
            Self::AwaitHello { .. } => None,
            Self::AwaitConfigure { surface, .. } => Some(surface.fd().as_raw_fd()),
        }
    }

    pub(crate) fn remaining(&self, now: Instant) -> Duration {
        match self {
            Self::AwaitHello { deadline, .. } => deadline.saturating_duration_since(now),
            Self::AwaitConfigure { surface, .. } => surface.remaining(now),
        }
    }

    pub(crate) fn advance_control(self, now: Instant) -> AdmissionAdvance {
        match self {
            Self::AwaitHello {
                client,
                mut receiver,
                deadline,
                peer_pid,
            } => match receiver.try_recv(&client) {
                Ok(HelloReceiveStatus::Pending) if now < deadline => {
                    AdmissionAdvance::Pending(Self::AwaitHello {
                        client,
                        receiver,
                        deadline,
                        peer_pid,
                    })
                }
                Ok(HelloReceiveStatus::Pending) => reject(
                    &client,
                    "recv",
                    anyhow::anyhow!("timed out after {HELLO_TIMEOUT:?} waiting for Hello"),
                ),
                Ok(HelloReceiveStatus::Complete(msg, wayland_fd)) => {
                    let HelloMsg::Load {
                        widget_key,
                        wasm_path,
                        asset_root,
                    } = msg;
                    let widget_key = match parse_widget_key(&widget_key) {
                        Ok(key) => key,
                        Err(error) => return reject(&client, "recv", error),
                    };
                    let wasm_path = PathBuf::from(wasm_path);
                    let asset_root = asset_root.map(PathBuf::from);
                    tracing::info!(
                        ?peer_pid,
                        wasm = %wasm_path.display(),
                        "thin requested wasm load"
                    );
                    match PendingDeckWidgetSurfaceClient::start_with_fd_and_key(
                        wayland_fd, widget_key,
                    ) {
                        Ok(surface) => Self::AwaitConfigure {
                            client,
                            surface: Box::new(surface),
                            wasm_path,
                            asset_root,
                            peer_pid,
                        }
                        .advance_control(now),
                        Err(error) => reject(&client, "load", error),
                    }
                }
                Err(error) => reject(&client, "recv", error.into()),
            },
            admission @ Self::AwaitConfigure { .. } => {
                let mut byte = [0_u8; 1];
                let mut socket = admission.client();
                let result = socket.read(&mut byte);
                match crate::slot::classify_control_socket_read(result, byte[0]) {
                    crate::slot::ControlSocketStatus::WouldBlock => {
                        AdmissionAdvance::Pending(admission)
                    }
                    crate::slot::ControlSocketStatus::PeerClosed => reject(
                        admission.client(),
                        "load",
                        anyhow::anyhow!("control socket EOF"),
                    ),
                    crate::slot::ControlSocketStatus::UnsolicitedByte(byte) => reject(
                        admission.client(),
                        "load",
                        anyhow::anyhow!("unsolicited control byte {byte:#04x}"),
                    ),
                    crate::slot::ControlSocketStatus::Error(error) => {
                        reject(admission.client(), "load", error.into())
                    }
                }
            }
        }
    }

    pub(crate) fn advance_configure(self, shared: &SharedHost) -> AdmissionAdvance {
        let Self::AwaitConfigure {
            client,
            surface,
            wasm_path,
            asset_root,
            peer_pid,
        } = self
        else {
            return AdmissionAdvance::Pending(self);
        };
        match surface.advance() {
            Ok(PendingDeckWidgetSurfaceAdvance::Pending(surface)) => {
                AdmissionAdvance::Pending(Self::AwaitConfigure {
                    client,
                    surface: Box::new(surface),
                    wasm_path,
                    asset_root,
                    peer_pid,
                })
            }
            Ok(PendingDeckWidgetSurfaceAdvance::Ready(surface, initial)) => {
                let factory: Rc<dyn crate::render_target::RenderTargetFactory> =
                    Rc::new(EglRenderTargetFactory);
                let wasm = wasm_path
                    .file_name()
                    .and_then(|file| file.to_str())
                    .unwrap_or("widget");
                let slot = {
                    let _span = tracing::info_span!("widget", wasm).entered();
                    WidgetSlot::from_configured(
                        &wasm_path,
                        asset_root.as_deref(),
                        shared,
                        surface,
                        initial,
                        match client.try_clone() {
                            Ok(control) => control,
                            Err(error) => return reject(&client, "load", error.into()),
                        },
                        peer_pid,
                        factory,
                    )
                };
                match slot {
                    Ok(slot) => match acknowledge_loaded(&client, slot, |slot| {
                        slot.control_socket.set_nonblocking(true)
                    }) {
                        Ok(slot) => {
                            tracing::info!(
                                ?peer_pid,
                                wasm = %wasm_path.display(),
                                "widget load acknowledged"
                            );
                            AdmissionAdvance::Loaded(Box::new(slot))
                        }
                        Err(error) => AdmissionAdvance::Rejected(error),
                    },
                    Err(error) => reject(&client, "load", error),
                }
            }
            Err(error) => reject(&client, "load", error),
        }
    }

    fn client(&self) -> &UnixStream {
        match self {
            Self::AwaitHello { client, .. } | Self::AwaitConfigure { client, .. } => client,
        }
    }
}

fn acknowledge_loaded<T>(
    client: &UnixStream,
    loaded: T,
    prepare_established: impl FnOnce(&T) -> io::Result<()>,
) -> anyhow::Result<T> {
    client
        .set_nonblocking(false)
        .context("control socket set_nonblocking(false) for Ack")?;
    client
        .set_write_timeout(Some(HELLO_TIMEOUT))
        .context("control socket set_write_timeout for Ack")?;
    write_ack(client, &AckMsg::Ok).context("write successful load Ack")?;
    client
        .set_write_timeout(None)
        .context("control socket clear Ack write timeout")?;
    prepare_established(&loaded).context("prepare established control socket")?;
    Ok(loaded)
}

fn reject(client: &UnixStream, phase: &str, error: anyhow::Error) -> AdmissionAdvance {
    let _ = write_ack(client, &AckMsg::Err(format!("{phase}: {error}")));
    AdmissionAdvance::Rejected(error)
}

fn peer_pid_of(client: &UnixStream) -> Option<libc::pid_t> {
    use nix::sys::socket::getsockopt;
    use nix::sys::socket::sockopt::PeerCredentials;
    getsockopt(client, PeerCredentials)
        .ok()
        .map(|creds| creds.pid())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::{ErrorKind, Read as _};
    use std::os::fd::AsFd as _;
    use std::os::unix::net::UnixStream;
    use std::rc::Rc;
    use std::time::Instant;

    use bmc_wasm_thin_protocol::{AckDecoder, AckMsg, HelloMsg, send_hello_with_fd};

    use super::{AdmissionAdvance, HELLO_TIMEOUT, PendingAdmission, acknowledge_loaded};

    struct DropWitness(Rc<Cell<bool>>);

    impl Drop for DropWitness {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    #[test]
    fn successful_ack_transfers_loaded_ownership_and_restores_nonblocking() {
        let (host, mut thin) = UnixStream::pair().expect("BUG: control socketpair");
        host.set_nonblocking(true)
            .expect("BUG: initial host socket must become nonblocking");
        let established = host.try_clone().expect("BUG: clone control socket");

        let established =
            acknowledge_loaded(&host, established, |socket| socket.set_nonblocking(true))
                .expect("BUG: local successful Ack must transfer ownership");

        thin.set_read_timeout(Some(HELLO_TIMEOUT))
            .expect("BUG: local Ack read must be bounded");
        let mut decoder = AckDecoder::new();
        let ack = loop {
            let mut bytes = [0_u8; 64];
            let count = thin.read(&mut bytes).expect("BUG: thin must receive Ack");
            if let Some(ack) = decoder.push(&bytes[..count]).expect("BUG: Ack must decode") {
                break ack;
            }
        };
        assert!(matches!(ack, AckMsg::Ok));
        assert_eq!(
            (&established)
                .read(&mut [0_u8; 1])
                .expect_err("established socket must not block")
                .kind(),
            ErrorKind::WouldBlock
        );
    }

    #[test]
    fn failed_ack_drops_loaded_ownership() {
        let (host, thin) = UnixStream::pair().expect("BUG: control socketpair");
        host.set_nonblocking(true)
            .expect("BUG: initial host socket must become nonblocking");
        drop(thin);
        let dropped = Rc::new(Cell::new(false));

        assert!(acknowledge_loaded(&host, DropWitness(Rc::clone(&dropped)), |_| Ok(())).is_err());
        assert!(dropped.get(), "failed Ack must release the loaded slot");
    }

    fn admission_with_complete_hello() -> (PendingAdmission, UnixStream, UnixStream) {
        let (host, thin) = UnixStream::pair().expect("BUG: control socketpair");
        let (wayland_host, wayland_peer) = UnixStream::pair().expect("BUG: Wayland socketpair");
        let admission = PendingAdmission::new(host, Instant::now())
            .expect("BUG: local control admission must start");
        send_hello_with_fd(
            &thin,
            &HelloMsg::Load {
                widget_key: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
                wasm_path: "/test/widget.wasm".to_owned(),
                asset_root: None,
            },
            wayland_host.as_fd(),
        )
        .expect("BUG: complete Hello must send");
        (admission, thin, wayland_peer)
    }

    #[test]
    fn stalled_hello_does_not_delay_a_ready_sibling() {
        let (stalled_host, _stalled_thin) = UnixStream::pair().expect("BUG: control socketpair");
        let stalled = PendingAdmission::new(stalled_host, Instant::now())
            .expect("BUG: stalled admission must start");
        let (ready, _ready_thin, _wayland_peer) = admission_with_complete_hello();

        assert!(matches!(
            stalled.advance_control(Instant::now()),
            AdmissionAdvance::Pending(PendingAdmission::AwaitHello { .. })
        ));
        assert!(matches!(
            ready.advance_control(Instant::now()),
            AdmissionAdvance::Pending(PendingAdmission::AwaitConfigure { .. })
        ));
    }

    #[test]
    fn silent_hello_is_rejected_at_its_deadline() {
        let (host, _thin) = UnixStream::pair().expect("BUG: control socketpair");
        let started = Instant::now();
        let admission =
            PendingAdmission::new(host, started).expect("BUG: silent admission must start");

        assert!(matches!(
            admission.advance_control(started + HELLO_TIMEOUT),
            AdmissionAdvance::Rejected(_)
        ));
    }
}
