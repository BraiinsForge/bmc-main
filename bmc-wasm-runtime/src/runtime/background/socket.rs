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
use bmc_wasm_protocol::{SocketId, WsHostClose};
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

/// How long a WebSocket waits on each step of connecting:
/// each address it dials, and each read or write of the handshake.
/// The thread reads the widget's close only once the handshake is done,
/// so this is what frees a thread dialled to a peer that stops answering.
/// A peer that trickles the handshake out, or a host with several addresses, can take longer.
/// The write timeout stays set afterwards, so a send the peer leaves unread that long ends the socket as 1006.
pub(in crate::runtime) const WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Bounds on one WebSocket connection.
#[derive(Debug, Clone, Copy)]
pub(in crate::runtime) struct WsLimits {
    pub max_message_bytes: usize,
    pub handshake_timeout: Duration,
}

/// Background thread for a single WebSocket connection.
///
/// `url` is dialled and may carry substituted secrets, so it is never logged;
/// `log_url` is the widget's own placeholder form.
#[expect(clippy::too_many_lines)]
#[expect(clippy::needless_pass_by_value)] // ownership needed: moved into spawned thread
pub(in crate::runtime) fn ws_background_thread(
    ws_id: u32,
    url: &str,
    log_url: &str,
    headers: &[(String, String)],
    limits: WsLimits,
    event_tx: std::sync::mpsc::Sender<WsEvent>,
    msg_rx: std::sync::mpsc::Receiver<WsOutbound>,
) {
    use tungstenite::Message;
    use tungstenite::http::Request;
    use tungstenite::protocol::frame::coding::CloseCode;
    use tungstenite::protocol::{CloseFrame, WebSocketConfig};

    let WsLimits {
        max_message_bytes,
        handshake_timeout,
    } = limits;

    let uri: tungstenite::http::Uri = match url.parse() {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(ws_id, url = %log_url, "WS bad URL: {e}");
            let _ = event_tx.send(WsEvent::Close(1002));
            return;
        }
    };
    let mut request = Request::builder()
        .uri(&uri)
        .header("Host", host_header(&uri))
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
            tracing::error!(ws_id, url = %log_url, "WS bad request: {e}");
            let _ = event_tx.send(WsEvent::Close(1002));
            return;
        }
    };

    let tcp = match dial(&uri, handshake_timeout) {
        Ok(tcp) => tcp,
        Err(reason) => {
            tracing::error!(ws_id, url = %log_url, "WS connect failed: {reason}");
            let _ = event_tx.send(WsEvent::Close(1006));
            return;
        }
    };
    let prepared = tcp
        .set_read_timeout(Some(handshake_timeout))
        .and_then(|()| tcp.set_write_timeout(Some(handshake_timeout)));
    if let Err(e) = prepared {
        tracing::warn!(ws_id, "WS handshake timeout not set: {e}");
    }
    let config = WebSocketConfig {
        max_message_size: Some(max_message_bytes),
        max_frame_size: Some(max_message_bytes),
        ..WebSocketConfig::default()
    };
    // One handshake on the dialled stream: a redirect fails it instead of being followed,
    // so the socket never leaves the host the egress pin judged.
    let (mut socket, _response) =
        match tungstenite::client::client_with_config(request, tcp, Some(config)) {
            Ok(pair) => pair,
            Err(e) => {
                tracing::error!(ws_id, url = %log_url, "WS handshake failed: {e}");
                let _ = event_tx.send(WsEvent::Close(1006));
                return;
            }
        };

    let _ = socket
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(50)));

    let _ = event_tx.send(WsEvent::Open);
    tracing::info!(ws_id, url = %log_url, "WS connected");

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
                Ok(WsOutbound::Close(code)) => {
                    let _ = socket.close(code.map(|code| CloseFrame {
                        code: code.into(),
                        reason: "".into(),
                    }));
                    let _ = event_tx.send(WsEvent::Close(code.unwrap_or(1000)));
                    await_peer_close(&mut socket);
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
            Err(tungstenite::Error::Capacity(e)) => {
                tracing::warn!(ws_id, "WS message over {max_message_bytes} bytes: {e}");
                let _ = socket.close(Some(CloseFrame {
                    code: CloseCode::Size,
                    reason: "".into(),
                }));
                let _ = socket.flush();
                let _ = event_tx.send(WsEvent::HostClosed(WsHostClose::MessageTooLarge));
                await_peer_close(&mut socket);
                return;
            }
            Err(e) => {
                tracing::warn!(ws_id, "WS read error: {e}");
                break;
            }
        }
    }

    let _ = event_tx.send(WsEvent::Close(1006));
    tracing::info!(ws_id, "WS background thread exiting");
}

/// How long a socket the host closes waits for the peer's close frame before dropping the stream.
const WS_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

/// Wait out the closing handshake after sending our close frame, up to [`WS_CLOSE_TIMEOUT`].
///
/// Dropped at once, the stream would meet the peer's close frame already closed,
/// and the kernel would answer it with a reset, which can discard our close frame
/// before the peer reads it.
fn await_peer_close(socket: &mut tungstenite::WebSocket<std::net::TcpStream>) {
    let deadline = std::time::Instant::now() + WS_CLOSE_TIMEOUT;
    while std::time::Instant::now() < deadline {
        match socket.read() {
            Ok(tungstenite::Message::Close(_)) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return,
        }
    }
}

/// Open the TCP connection a `ws://` URI names.
///
/// Dialled here rather than by `tungstenite::connect`,
/// which logs the URI it dials and names it in its errors,
/// and the URI dialled here may carry substituted secrets.
fn dial(uri: &tungstenite::http::Uri, timeout: Duration) -> Result<std::net::TcpStream, String> {
    use std::net::ToSocketAddrs as _;

    // No TLS is built in, and `tungstenite::client` would run a cleartext handshake against a `wss://` port.
    if uri.scheme_str() != Some("ws") {
        return Err("only ws:// is supported".to_owned());
    }
    let host = uri
        .host()
        .ok_or_else(|| "the URI has no host".to_owned())?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let port = uri.port_u16().unwrap_or(80);
    let addrs = (host, port).to_socket_addrs().map_err(|e| e.to_string())?;

    let tcp = addrs
        .into_iter()
        .find_map(|addr| std::net::TcpStream::connect_timeout(&addr, timeout).ok())
        .ok_or_else(|| "unable to connect".to_owned())?;
    // As `tungstenite::connect` does: a frame goes out whole, so Nagle only delays it.
    tcp.set_nodelay(true).map_err(|e| e.to_string())?;
    Ok(tcp)
}

/// The `Host` header for `uri`: its host and any port it names, without the userinfo.
fn host_header(uri: &tungstenite::http::Uri) -> String {
    match (uri.host(), uri.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_owned(),
        (None, _) => "localhost".to_owned(),
    }
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
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    use tungstenite::Message;
    use tungstenite::protocol::frame::coding::CloseCode;

    use super::{
        TlsVerificationMode, WS_HANDSHAKE_TIMEOUT, WsEvent, WsHostClose, WsLimits,
        build_tls_client_config, dial, ws_background_thread,
    };

    const LIMITS: WsLimits = WsLimits {
        max_message_bytes: 1024,
        handshake_timeout: WS_HANDSHAKE_TIMEOUT,
    };

    fn loopback_listener() -> (TcpListener, std::net::SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind loopback");
        let addr = listener.local_addr().expect("BUG: local addr");
        (listener, addr)
    }

    #[test]
    fn tls_client_config_builds_for_both_modes() {
        assert!(build_tls_client_config(TlsVerificationMode::Full).is_ok());
        assert!(build_tls_client_config(TlsVerificationMode::Insecure).is_ok());
    }

    #[test]
    fn a_message_over_the_limit_is_a_host_close_and_tells_the_peer_1009() {
        const LIMIT: usize = LIMITS.max_message_bytes;
        let (listener, addr) = loopback_listener();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().expect("BUG: accept test client");
            let mut socket = tungstenite::accept(tcp).expect("BUG: accept the handshake");
            for len in [LIMIT, LIMIT + 1] {
                socket
                    .send(Message::text("x".repeat(len)))
                    .expect("BUG: send a test message");
            }
            loop {
                match socket.read() {
                    Ok(Message::Close(frame)) => return frame.map(|frame| frame.code),
                    Ok(_) => {}
                    Err(_) => return None,
                }
            }
        });

        let (event_tx, event_rx) = mpsc::channel();
        let (_msg_tx, msg_rx) = mpsc::channel();
        let url = format!("ws://{addr}/");
        ws_background_thread(1, &url, &url, &[], LIMITS, event_tx, msg_rx);

        let events: Vec<WsEvent> = event_rx.try_iter().collect();
        assert!(
            matches!(
                events.as_slice(),
                [
                    WsEvent::Open,
                    WsEvent::Message(text),
                    WsEvent::HostClosed(WsHostClose::MessageTooLarge),
                ] if text.len() == LIMIT
            ),
            "a message at the limit arrives, the one over it closes the socket: {events:?}"
        );
        assert_eq!(
            server.join().expect("BUG: the test server panicked"),
            Some(CloseCode::Size),
            "the peer is told why"
        );
    }

    #[test]
    #[expect(
        clippy::result_large_err,
        reason = "tungstenite's handshake callback must return its own ErrorResponse"
    )]
    fn the_host_header_carries_no_userinfo() {
        let (listener, addr) = loopback_listener();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().expect("BUG: accept test client");
            let mut host = None;
            let _ = tungstenite::accept_hdr(
                tcp,
                |request: &tungstenite::handshake::server::Request,
                 response: tungstenite::handshake::server::Response| {
                    host = request
                        .headers()
                        .get("Host")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    Ok(response)
                },
            );
            host
        });

        let (event_tx, _event_rx) = mpsc::channel();
        let (msg_tx, msg_rx) = mpsc::channel();
        drop(msg_tx);
        let url = format!("ws://user:s3cr3t@{addr}/");
        ws_background_thread(1, &url, &url, &[], LIMITS, event_tx, msg_rx);

        assert_eq!(
            server.join().expect("BUG: the test server panicked"),
            Some(addr.to_string()),
            "the Host header names the host and port alone",
        );
    }

    #[test]
    fn a_peer_that_never_answers_the_handshake_frees_the_thread() {
        let (listener, addr) = loopback_listener();
        let limits = WsLimits {
            handshake_timeout: Duration::from_millis(200),
            ..LIMITS
        };
        let (event_tx, event_rx) = mpsc::channel();
        let (_msg_tx, msg_rx) = mpsc::channel();
        let url = format!("ws://{addr}/");
        let thread = std::thread::spawn(move || {
            ws_background_thread(1, &url, &url, &[], limits, event_tx, msg_rx);
        });
        let (_silent, _) = listener.accept().expect("BUG: accept test client");

        assert!(
            matches!(
                event_rx.recv_timeout(Duration::from_secs(5)),
                Ok(WsEvent::Close(1006))
            ),
            "a handshake nobody answers must fail within its timeout",
        );
        thread.join().expect("BUG: the socket thread panicked");
    }

    #[test]
    fn a_dialled_stream_sends_without_delay() {
        let (_listener, addr) = loopback_listener();
        let uri = format!("ws://{addr}/")
            .parse()
            .expect("BUG: a loopback URI parses");
        let tcp = dial(&uri, WS_HANDSHAKE_TIMEOUT).expect("BUG: the listener accepts");
        assert!(
            tcp.nodelay().expect("BUG: read TCP_NODELAY"),
            "a frame goes out whole, so Nagle would only delay it",
        );
    }
}
