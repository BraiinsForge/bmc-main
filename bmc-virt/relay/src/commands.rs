// Copyright (C) 2026  Braiins Systems s.r.o.

//! gRPC-web client for the BMC web API.
//! Uses prost-generated types from the proto definitions.

use prost::Message;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use bmc_grpc::web;

const BMC_HOST: &str = "127.0.0.1:80";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(2);

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
    session_id: Option<String>,
}

impl GrpcClient {
    pub fn new() -> Self {
        Self { session_id: None }
    }

    /// Send a `SetEffect` gRPC call with the given preset.
    pub fn set_effect(&mut self, preset: &EffectPreset) -> Result<(), String> {
        self.ensure_session()?;
        let msg = web::SetLedEffectRequest {
            effect: preset.effect.into(),
            color: Some(web::RgbColor {
                r: u32::from(preset.r),
                g: u32::from(preset.g),
                b: u32::from(preset.b),
            }),
            period_ms: preset.period_ms,
            duration_ms: 0,
        };
        let body = grpc_web_frame(&msg.encode_to_vec());
        self.post("/braiins.bmc.web.LedTestService/SetEffect", &body)?;
        Ok(())
    }

    /// Send `SetEffect(NONE)` to clear the test override.
    pub fn clear_effect(&mut self) -> Result<(), String> {
        self.ensure_session()?;
        let msg = web::SetLedEffectRequest {
            effect: LedEffectType::None.into(),
            color: None,
            period_ms: 0,
            duration_ms: 0,
        };
        let body = grpc_web_frame(&msg.encode_to_vec());
        self.post("/braiins.bmc.web.LedTestService/SetEffect", &body)?;
        Ok(())
    }

    /// Get the current sound volume (0–100).
    #[expect(clippy::cast_possible_truncation, reason = "volume is 0–100")]
    pub fn get_volume(&mut self) -> Result<u8, String> {
        self.ensure_session()?;
        let body = grpc_web_frame(&[]);
        let resp = self.post(
            "/braiins.bmc.web.ConfigurationService/GetSoundVolumeSettings",
            &body,
        )?;
        let payload = extract_grpc_payload(&resp).ok_or("no grpc payload in response")?;
        let settings = web::SoundVolumeSettingsResponse::decode(payload)
            .map_err(|e| format!("decode error: {e}"))?;
        Ok(settings.volume.map_or(0, |v| v.value as u8))
    }

    fn ensure_session(&mut self) -> Result<(), String> {
        if self.session_id.is_some() {
            return Ok(());
        }
        let msg = web::LoginRequest {
            password: "root".into(),
        };
        let body = grpc_web_frame(&msg.encode_to_vec());
        let response = Self::post_raw("/braiins.bmc.web.AuthenticationService/Login", &body, None)?;
        self.session_id = Some(
            extract_session_cookie(&response).ok_or("no session_id cookie in login response")?,
        );
        Ok(())
    }

    fn post(&mut self, path: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        let sid = self.session_id.clone();
        Self::post_raw(path, body, sid.as_deref())
    }

    fn post_raw(path: &str, body: &[u8], session_id: Option<&str>) -> Result<Vec<u8>, String> {
        let addr = BMC_HOST
            .parse()
            .map_err(|e: std::net::AddrParseError| e.to_string())?;
        let mut stream =
            TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(IO_TIMEOUT)).ok();
        stream.set_write_timeout(Some(IO_TIMEOUT)).ok();

        let mut header = String::with_capacity(256);
        let _ = write!(
            header,
            "POST {path} HTTP/1.1\r\n\
             Host: localhost\r\n\
             Content-Type: application/grpc-web+proto\r\n"
        );
        if let Some(id) = session_id {
            let _ = write!(header, "Cookie: session_id={id}\r\n");
        }
        let _ = write!(
            header,
            "Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            body.len(),
        );

        stream
            .write_all(header.as_bytes())
            .map_err(|e| e.to_string())?;
        stream.write_all(body).map_err(|e| e.to_string())?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).ok();
        Ok(response)
    }
}

fn extract_session_cookie(response: &[u8]) -> Option<String> {
    let end = response.windows(4).position(|w| w == b"\r\n\r\n")?;
    let headers = std::str::from_utf8(&response[..end]).ok()?;
    for line in headers.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("set-cookie:")
            && let Some(start) = line.find("session_id=")
        {
            let val_start = start + "session_id=".len();
            let cookie_value = line.get(val_start..)?;
            let val_end = cookie_value.find(';').map_or(cookie_value.len(), |i| i);
            return Some(cookie_value.get(..val_end)?.to_owned());
        }
    }
    None
}

/// Extract the protobuf payload from a gRPC-web HTTP response.
/// Skips HTTP headers, then reads the 5-byte grpc frame header.
fn extract_grpc_payload(response: &[u8]) -> Option<&[u8]> {
    let body_start = response.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let body = response.get(body_start..)?;
    // grpc frame: [compressed: u8] [len: u32 BE] [data]
    if body.len() < 5 {
        return None;
    }
    let len = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
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
