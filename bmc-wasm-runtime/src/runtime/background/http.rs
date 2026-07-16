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

//! Background HTTP listener helpers for the WASM runtime.

use std::time::Duration;

use crate::host_api::{HttpInboundRequest, HttpListenerResponse};

/// Background thread for an HTTP listener.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
pub(in crate::runtime) fn http_listener_thread(
    port: u16,
    request_tx: std::sync::mpsc::Sender<HttpInboundRequest>,
    stop_rx: std::sync::mpsc::Receiver<()>,
    port_report_tx: std::sync::mpsc::Sender<u16>,
) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("HTTP listener bind failed on port {port}: {e}");
            let _ = port_report_tx.send(0);
            return;
        }
    };
    listener
        .set_nonblocking(true)
        .expect("BUG: set_nonblocking failed");

    let actual_port = listener.local_addr().map_or(port, |a| a.port());
    let _ = port_report_tx.send(actual_port);
    tracing::info!("HTTP listener started on port {actual_port}");

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        match listener.accept() {
            Ok((mut stream, addr)) => {
                tracing::debug!("HTTP connection from {addr}");
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

                let mut reader = BufReader::new(&stream);

                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let parts: Vec<&str> = request_line.trim().splitn(3, ' ').collect();
                if parts.len() < 2 {
                    continue;
                }
                let method = parts[0].to_owned();
                let path = parts[1].to_owned();

                let mut headers = String::new();
                let mut content_length: usize = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                        break;
                    }
                    if let Some(val) = line
                        .to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().to_owned())
                    {
                        content_length = val.parse().unwrap_or(0);
                    }
                    headers.push_str(&line);
                }

                let mut body = vec![0_u8; content_length];
                if content_length > 0 {
                    let _ = reader.read_exact(&mut body);
                }

                let (resp_tx, resp_rx) = std::sync::mpsc::channel::<HttpListenerResponse>();

                let req = HttpInboundRequest {
                    method,
                    path,
                    headers,
                    body,
                    response_tx: resp_tx,
                };

                if request_tx.send(req).is_err() {
                    break;
                }

                if let Ok(resp) = resp_rx.recv_timeout(Duration::from_secs(10)) {
                    let status_text = match resp.status {
                        204 => "No Content",
                        400 => "Bad Request",
                        404 => "Not Found",
                        500 => "Internal Server Error",
                        _ => "OK",
                    };
                    let response = format!(
                        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\n{}\r\n",
                        resp.status,
                        status_text,
                        resp.body.len(),
                        resp.headers,
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(&resp.body);
                    let _ = stream.flush();
                } else {
                    let response = "HTTP/1.1 504 Gateway Timeout\r\nContent-Length: 0\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                tracing::error!("HTTP listener accept error: {e}");
                break;
            }
        }
    }
    tracing::info!("HTTP listener stopped on port {actual_port}");
}
