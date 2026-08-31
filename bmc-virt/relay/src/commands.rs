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

//! gRPC-Web client for the BMC web API.
//!
//! The transport is `ureq` (HTTP/1.1 with native chunked-transfer) plus a thin
//! local helper for the 5-byte gRPC frame header.

use prost::Message;
use std::time::Duration;

use bmc_grpc::web;

const BMC_URL: &str = "http://127.0.0.1:80";
const TIMEOUT: Duration = Duration::from_secs(5);
const GRPC_WEB_CONTENT_TYPE: &str = "application/grpc-web+proto";

pub use web::LedEffectType;

pub struct EffectPreset {
    pub name: &'static str,
    pub effect: LedEffectType,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub period_ms: u32,
}

pub const PRESETS: &[EffectPreset] = &[
    EffectPreset {
        name: "knight-rider red",
        effect: LedEffectType::KnightRider,
        r: 255,
        g: 0,
        b: 0,
        period_ms: 800,
    },
    EffectPreset {
        name: "chase green",
        effect: LedEffectType::Chase,
        r: 0,
        g: 255,
        b: 0,
        period_ms: 600,
    },
    EffectPreset {
        name: "breathe blue",
        effect: LedEffectType::Breathe,
        r: 0,
        g: 100,
        b: 255,
        period_ms: 3_000,
    },
    EffectPreset {
        name: "snake orange",
        effect: LedEffectType::Snake,
        r: 255,
        g: 165,
        b: 0,
        period_ms: 1_000,
    },
    EffectPreset {
        name: "solid white",
        effect: LedEffectType::Solid,
        r: 255,
        g: 255,
        b: 255,
        period_ms: 0,
    },
    EffectPreset {
        name: "scan purple",
        effect: LedEffectType::Scan,
        r: 128,
        g: 0,
        b: 255,
        period_ms: 1_200,
    },
];

pub struct GrpcClient {
    agent: ureq::Agent,
    base_url: String,
    session_token: Option<String>,
}

impl Default for GrpcClient {
    fn default() -> Self {
        Self::new()
    }
}

impl GrpcClient {
    pub fn new() -> Self {
        Self::with_base_url(BMC_URL.to_owned())
    }

    fn with_base_url(base_url: String) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(TIMEOUT))
            .timeout_send_request(Some(TIMEOUT))
            .timeout_send_body(Some(TIMEOUT))
            .timeout_recv_response(Some(TIMEOUT))
            .timeout_recv_body(Some(TIMEOUT))
            .build();
        Self {
            agent: config.new_agent(),
            base_url,
            session_token: None,
        }
    }

    /// Send a `SetEffect` gRPC call with the given preset.
    pub fn set_effect(&mut self, preset: &EffectPreset) -> Result<(), String> {
        self.ensure_session()?;
        let req = web::SetLedEffectRequest {
            effect: preset.effect.into(),
            color: Some(web::RgbColor {
                r: u32::from(preset.r),
                g: u32::from(preset.g),
                b: u32::from(preset.b),
            }),
            period_ms: preset.period_ms,
            duration_ms: 0,
        };
        self.call(
            "/braiins.bmc.web.LedTestService/SetEffect",
            &req.encode_to_vec(),
        )?;
        Ok(())
    }

    /// Send `SetEffect(NONE)` to clear the test override.
    pub fn clear_effect(&mut self) -> Result<(), String> {
        self.ensure_session()?;
        let req = web::SetLedEffectRequest {
            effect: LedEffectType::None.into(),
            color: None,
            period_ms: 0,
            duration_ms: 0,
        };
        self.call(
            "/braiins.bmc.web.LedTestService/SetEffect",
            &req.encode_to_vec(),
        )?;
        Ok(())
    }

    /// Get the current sound volume (0–100).
    #[expect(clippy::cast_possible_truncation, reason = "volume is 0–100")]
    pub fn get_volume(&mut self) -> Result<u8, String> {
        self.ensure_session()?;
        let payload = self.call(
            "/braiins.bmc.web.ConfigurationService/GetSoundVolumeSettings",
            &[], // request type is google.protobuf.Empty; empty proto encodes to zero bytes
        )?;
        let settings = web::SoundVolumeSettingsResponse::decode(payload.as_slice())
            .map_err(|e| format!("decode SoundVolumeSettingsResponse: {e}"))?;
        Ok(settings.volume.map_or(0, |v| v.value as u8))
    }

    fn ensure_session(&mut self) -> Result<(), String> {
        if self.session_token.is_some() {
            return Ok(());
        }
        let req = web::LoginRequest {
            password: "root".into(),
        };
        let payload = self.call(
            "/braiins.bmc.web.AuthenticationService/Login",
            &req.encode_to_vec(),
        )?;
        let response = web::LoginResponse::decode(payload.as_slice())
            .map_err(|e| format!("decode LoginResponse: {e}"))?;
        if response.token.is_empty() {
            return Err("login response contained an empty session token".into());
        }
        self.session_token = Some(response.token);
        Ok(())
    }

    /// Send one gRPC-Web request and return the first frame's protobuf
    /// payload, ready to feed to [`prost::Message::decode`]. `proto` is the
    /// already-encoded request message (empty slice for
    /// `google.protobuf.Empty`).
    fn call(&self, path: &str, proto: &[u8]) -> Result<Vec<u8>, String> {
        let framed = grpc_web_frame(proto);
        let url = format!("{}{path}", self.base_url);
        let request = self
            .agent
            .post(&url)
            .header("content-type", GRPC_WEB_CONTENT_TYPE);
        let request = match &self.session_token {
            Some(token) => request.header("cookie", format!("session_id={token}")),
            None => request,
        };
        let response = request
            .send(&framed[..])
            .map_err(|e| format!("POST {path}: {e}"))?;
        let body = response
            .into_body()
            .read_to_vec()
            .map_err(|e| format!("read {path} body: {e}"))?;
        extract_grpc_payload(&body)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| format!("no grpc data frame in {path} response"))
    }
}

/// Read the leading data frame's payload from a fully-buffered gRPC-Web
/// response body. The body is a sequence of `[flag: u8][len: u32 BE][data]`
/// frames; the first non-trailer frame is the message payload, anything
/// after (typically the `grpc-status` trailer frame) is intentionally
/// discarded — callers either care about the payload or about a transport
/// error already surfaced by `ureq`.
fn extract_grpc_payload(body: &[u8]) -> Option<&[u8]> {
    // Bit 0x80 of the flag byte distinguishes trailer frames (0x80..) from
    // data frames (0x00 / 0x01 = uncompressed / compressed). A trailer-only
    // response means the server replied with status metadata and no payload.
    let flag = *body.first()?;
    if flag & 0x80 != 0 {
        return None;
    }
    let len = u32::from_be_bytes(body.get(1..5)?.try_into().ok()?) as usize;
    body.get(5..5 + len)
}

fn grpc_web_frame(message: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + message.len());
    frame.push(0x00); // uncompressed
    #[expect(clippy::cast_possible_truncation, reason = "grpc-web frames are small")]
    frame.extend((message.len() as u32).to_be_bytes());
    frame.extend(message);
    frame
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    use prost::Message;

    use super::{GrpcClient, grpc_web_frame, web};

    fn read_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let body_start = loop {
            if let Some(offset) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break offset + 4;
            }
            let mut chunk = [0_u8; 1_024];
            let read = stream.read(&mut chunk).expect("BUG: read test request");
            assert_ne!(read, 0, "client closed before finishing request headers");
            request.extend_from_slice(&chunk[..read]);
        };
        let headers = std::str::from_utf8(&request[..body_start])
            .expect("BUG: ureq must send ASCII request headers");
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then(|| {
                    value
                        .trim()
                        .parse::<usize>()
                        .expect("BUG: valid content length")
                })
            })
            .unwrap_or_default();
        let request_length = body_start + content_length;
        while request.len() < request_length {
            let mut chunk = [0_u8; 1_024];
            let read = stream
                .read(&mut chunk)
                .expect("BUG: read test request body");
            assert_ne!(read, 0, "client closed before finishing request body");
            request.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8(request[..body_start].to_vec())
            .expect("BUG: ureq must send ASCII request headers")
    }

    fn serve_response(listener: &TcpListener, extra_headers: &str, body: &[u8]) -> String {
        let (mut stream, _) = listener.accept().expect("BUG: accept test client");
        let request = read_request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n",
            body.len()
        )
        .expect("BUG: write test response headers");
        stream
            .write_all(body)
            .expect("BUG: write test response body");
        request
    }

    #[test]
    fn login_token_authenticates_subsequent_calls_without_a_cookie_jar() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind loopback listener");
        let addr = listener.local_addr().expect("BUG: read listener address");
        let server = std::thread::spawn(move || {
            let login = web::LoginResponse {
                token: "test-session".into(),
                timeout_s: 3_600,
            };
            let login_request = serve_response(
                &listener,
                "Set-Cookie: session_id=jar-session\r\n",
                &grpc_web_frame(&login.encode_to_vec()),
            );
            let authenticated_request = serve_response(&listener, "", &grpc_web_frame(&[]));
            (login_request, authenticated_request)
        });

        let mut client = GrpcClient::with_base_url(format!("http://{addr}"));
        client
            .clear_effect()
            .expect("explicit session cookie must authenticate the call");

        let (login_request, authenticated_request) =
            server.join().expect("server thread must finish cleanly");
        assert!(
            !login_request.to_ascii_lowercase().contains("\r\ncookie:"),
            "the login request must not carry a session cookie",
        );
        let authenticated_request = authenticated_request.to_ascii_lowercase();
        let cookie_headers = authenticated_request
            .lines()
            .filter(|line| line.starts_with("cookie:"))
            .collect::<Vec<_>>();
        assert_eq!(
            cookie_headers,
            ["cookie: session_id=test-session"],
            "the explicit session cookie must be the only cookie header",
        );
    }
}
