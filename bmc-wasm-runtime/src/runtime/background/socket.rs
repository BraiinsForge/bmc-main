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

//! Background socket and TLS helpers for the WASM runtime.

#![expect(clippy::cast_possible_truncation)]

use std::time::Duration;

use anyhow::Result;
use bmc_wasm_protocol::SocketId;
use wasmi::Caller;

use crate::host_api::{ActiveSocket, HostState, SocketEvent, SocketOutbound, WsEvent, WsOutbound};

use super::super::memory::read_string;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) enum TlsVerificationMode {
    Full,
    Insecure,
}

pub(in crate::runtime) fn host_tls_connect_impl(
    caller: &mut Caller<'_, HostState>,
    host_ptr: u32,
    host_len: u32,
    port: u32,
    verification_mode: TlsVerificationMode,
) -> u32 {
    let host = read_string(caller, host_ptr, host_len);
    let Some(host) = host else {
        return 0;
    };

    let state = caller.data_mut();
    if state.sockets.len() >= state.resource_limits.max_sockets {
        tracing::warn!(
            max_sockets = state.resource_limits.max_sockets,
            "TLS connect rejected: runtime socket limit reached"
        );
        return 0;
    }
    let socket_id = SocketId::alloc(&mut state.next_socket_id);

    let (event_tx, event_rx) = std::sync::mpsc::channel::<SocketEvent>();

    if let Some(fixtures) = &mut state.event_fixtures {
        let (write_tx, _write_rx) = std::sync::mpsc::channel::<SocketOutbound>();
        state
            .sockets
            .insert(socket_id, ActiveSocket { write_tx, event_rx });
        fixtures.socket_event_txs.insert(socket_id, event_tx);
    } else {
        let (write_tx, write_rx) = std::sync::mpsc::channel::<SocketOutbound>();
        state
            .sockets
            .insert(socket_id, ActiveSocket { write_tx, event_rx });
        let port = port as u16;
        if !state.refuse_live_io("tls", &format!("{host}:{port}")) {
            let socket_id_wire = socket_id.to_wire();
            std::thread::spawn(move || {
                tls_background_thread(
                    socket_id_wire,
                    &host,
                    port,
                    verification_mode,
                    event_tx,
                    write_rx,
                );
            });
        }
    }

    socket_id.to_wire()
}

pub(in crate::runtime) fn build_tls_client_config(
    verification_mode: TlsVerificationMode,
) -> Result<rustls::ClientConfig> {
    use std::sync::Arc;

    let crypto_provider = rustls::crypto::ring::default_provider();
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(crypto_provider))
        .with_safe_default_protocol_versions()?;

    let config = match verification_mode {
        TlsVerificationMode::Full => {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        TlsVerificationMode::Insecure => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertVerifier))
            .with_no_client_auth(),
    };

    Ok(config)
}

/// Background thread for a single WebSocket connection.
#[expect(clippy::too_many_lines)]
#[expect(clippy::needless_pass_by_value)] // ownership needed: moved into spawned thread
pub(in crate::runtime) fn ws_background_thread(
    ws_id: u32,
    url: &str,
    headers: &[(String, String)],
    event_tx: std::sync::mpsc::Sender<WsEvent>,
    msg_rx: std::sync::mpsc::Receiver<WsOutbound>,
) {
    use tungstenite::http::Request;
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, connect};

    let connect_result = if headers.is_empty() {
        connect(url)
    } else {
        let uri: tungstenite::http::Uri = match url.parse() {
            Ok(u) => u,
            Err(e) => {
                tracing::error!(ws_id, "WS bad URL: {e}");
                let _ = event_tx.send(WsEvent::Close(1002));
                return;
            }
        };
        let mut request = Request::builder()
            .uri(&uri)
            .header(
                "Host",
                uri.authority()
                    .map_or_else(|| "localhost".to_owned(), ToString::to_string),
            )
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            );
        for (k, v) in headers {
            request = request.header(k.as_str(), v.as_str());
        }
        let request = match request.body(()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(ws_id, "WS bad request: {e}");
                let _ = event_tx.send(WsEvent::Close(1002));
                return;
            }
        };
        connect(request)
    };

    let (mut socket, _response) = match connect_result {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!(ws_id, "WS connect failed: {e}");
            let _ = event_tx.send(WsEvent::Close(1006));
            return;
        }
    };

    if let MaybeTlsStream::Plain(tcp) = socket.get_ref() {
        let _ = tcp.set_read_timeout(Some(Duration::from_millis(50)));
    }

    let _ = event_tx.send(WsEvent::Open);
    tracing::info!(ws_id, %url, "WS connected");

    loop {
        loop {
            match msg_rx.try_recv() {
                Ok(WsOutbound::Text(text)) => {
                    if let Err(e) = socket.send(Message::Text(text)) {
                        tracing::warn!(ws_id, "WS send error: {e}");
                        let _ = event_tx.send(WsEvent::Close(1006));
                        return;
                    }
                }
                Ok(WsOutbound::Close) => {
                    let _ = socket.close(None);
                    let _ = event_tx.send(WsEvent::Close(1000));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = event_tx.send(WsEvent::Close(1006));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        match socket.read() {
            Ok(Message::Text(text)) => {
                if event_tx.send(WsEvent::Message(text.into_bytes())).is_err() {
                    return;
                }
            }
            Ok(Message::Binary(data)) => {
                if event_tx.send(WsEvent::Message(data.clone())).is_err() {
                    return;
                }
            }
            Ok(Message::Close(frame)) => {
                let code = frame.map_or(1000, |f| f.code.into());
                let _ = event_tx.send(WsEvent::Close(code));
                tracing::info!(ws_id, code, "WS closed by server");
                return;
            }
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                tracing::warn!(ws_id, "WS read error: {e}");
                break;
            }
        }
    }

    let _ = event_tx.send(WsEvent::Close(1006));
    tracing::info!(ws_id, "WS background thread exiting");
}

/// Background thread for a plain TCP socket connection.
#[expect(clippy::needless_pass_by_value)] // ownership needed: moved into spawned thread
pub(in crate::runtime) fn tcp_background_thread(
    socket_id: u32,
    host: &str,
    port: u16,
    event_tx: std::sync::mpsc::Sender<SocketEvent>,
    write_rx: std::sync::mpsc::Receiver<SocketOutbound>,
) {
    use std::io::{Read as _, Write as _};

    let addr = format!("{host}:{port}");
    let mut tcp = match std::net::TcpStream::connect(&addr) {
        Ok(tcp) => tcp,
        Err(e) => {
            tracing::error!(socket_id, %addr, "TCP connect failed: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    if let Err(e) = tcp.set_read_timeout(Some(Duration::from_millis(50))) {
        tracing::warn!(socket_id, "failed to set read timeout: {e}");
    }

    let _ = event_tx.send(SocketEvent::Connected);
    tracing::info!(socket_id, %addr, "TCP connected");

    let mut read_buf = vec![0_u8; 16_384];

    loop {
        loop {
            match write_rx.try_recv() {
                Ok(SocketOutbound::Data(data)) => {
                    if let Err(e) = tcp.write_all(&data) {
                        tracing::warn!(socket_id, "TCP write error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                    if let Err(e) = tcp.flush() {
                        tracing::warn!(socket_id, "TCP flush error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                }
                Ok(SocketOutbound::Close) => {
                    let _ = event_tx.send(SocketEvent::Closed(0));
                    tracing::info!(socket_id, "TCP socket closed by widget");
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = event_tx.send(SocketEvent::Closed(1));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        match tcp.read(&mut read_buf) {
            Ok(0) => {
                let _ = event_tx.send(SocketEvent::Closed(0));
                tracing::info!(socket_id, "TCP EOF");
                return;
            }
            Ok(n) => {
                if event_tx
                    .send(SocketEvent::Data(read_buf[..n].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                tracing::warn!(socket_id, "TCP read error: {e}");
                let _ = event_tx.send(SocketEvent::Closed(1));
                return;
            }
        }
    }
}

/// Background thread for a single TLS socket connection.
#[expect(clippy::needless_pass_by_value)] // ownership needed: moved into spawned thread
fn tls_background_thread(
    socket_id: u32,
    host: &str,
    port: u16,
    verification_mode: TlsVerificationMode,
    event_tx: std::sync::mpsc::Sender<SocketEvent>,
    write_rx: std::sync::mpsc::Receiver<SocketOutbound>,
) {
    use std::io::{Read as _, Write as _};
    use std::sync::Arc;

    let config = match build_tls_client_config(verification_mode) {
        Ok(config) => config,
        Err(e) => {
            tracing::error!(socket_id, "TLS config error: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    let addr = format!("{host}:{port}");
    let tcp = match std::net::TcpStream::connect(&addr) {
        Ok(tcp) => tcp,
        Err(e) => {
            tracing::error!(socket_id, %addr, "TCP connect failed: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    let server_name = match rustls::pki_types::ServerName::try_from(host.to_owned()) {
        Ok(name) => name,
        Err(e) => {
            tracing::error!(socket_id, "invalid server name '{host}': {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    let conn = match rustls::ClientConnection::new(Arc::new(config), server_name) {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!(socket_id, "TLS handshake setup failed: {e}");
            let _ = event_tx.send(SocketEvent::Closed(1));
            return;
        }
    };

    let mut tls = rustls::StreamOwned::new(conn, tcp);

    if let Err(e) = tls.sock.set_read_timeout(Some(Duration::from_millis(50))) {
        tracing::warn!(socket_id, "failed to set read timeout: {e}");
    }

    let _ = event_tx.send(SocketEvent::Connected);
    tracing::info!(socket_id, %addr, "TLS connected");

    let mut read_buf = vec![0_u8; 16_384];

    loop {
        loop {
            match write_rx.try_recv() {
                Ok(SocketOutbound::Data(data)) => {
                    if let Err(e) = tls.write_all(&data) {
                        tracing::warn!(socket_id, "TLS write error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                    if let Err(e) = tls.flush() {
                        tracing::warn!(socket_id, "TLS flush error: {e}");
                        let _ = event_tx.send(SocketEvent::Closed(1));
                        return;
                    }
                }
                Ok(SocketOutbound::Close) => {
                    let _ = event_tx.send(SocketEvent::Closed(0));
                    tracing::info!(socket_id, "TLS socket closed by widget");
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = event_tx.send(SocketEvent::Closed(1));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }

        match tls.read(&mut read_buf) {
            Ok(0) => {
                let _ = event_tx.send(SocketEvent::Closed(0));
                tracing::info!(socket_id, "TLS EOF");
                return;
            }
            Ok(n) => {
                if event_tx
                    .send(SocketEvent::Data(read_buf[..n].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                tracing::warn!(socket_id, "TLS read error: {e}");
                let _ = event_tx.send(SocketEvent::Closed(1));
                return;
            }
        }
    }
}

/// Certificate verifier that accepts all certificates.
#[derive(Debug)]
struct NoCertVerifier;

impl rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::{TlsVerificationMode, build_tls_client_config};

    #[test]
    fn tls_client_config_builds_for_both_modes() {
        assert!(build_tls_client_config(TlsVerificationMode::Full).is_ok());
        assert!(build_tls_client_config(TlsVerificationMode::Insecure).is_ok());
    }
}
