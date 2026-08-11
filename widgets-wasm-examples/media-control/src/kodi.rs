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

//! Kodi JSON-RPC protocol controller.
//!
//! Implements media control via Kodi's HTTP JSON-RPC interface:
//! - POST to `http://host:port/jsonrpc` with JSON-RPC 2.0 payloads
//! - Two-phase polling: active players → parallel status queries
//! - Fire-and-forget commands with post-command re-poll
//!
//! mDNS discovery: `_xbmc-jsonrpc-h._tcp`

use std::cell::RefCell;

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

// ── Constants ────────────────────────────────────────────────────

/// Poll interval while playing (ms).
pub const POLL_INTERVAL_MS: u32 = 1_000;
/// Poll interval when idle/no media (ms).
pub const POLL_IDLE_INTERVAL_MS: u32 = 3_000;

/// Consecutive Phase 1 failures before `is_alive()` returns false.
const DEAD_THRESHOLD: u8 = 5;

// ── Public types ─────────────────────────────────────────────────

/// Parsed media status from a Kodi device.
#[derive(Debug, Clone, Default)]
pub struct KodiMediaStatus {
    /// `"playing"`, `"paused"`, or `""` (no media).
    pub player_state: String,
    pub title: Option<String>,
    /// Secondary metadata lines.
    pub fields: Vec<(String, String)>,
    pub album_art_url: Option<String>,
    pub duration_secs: f64,
    pub current_time: f64,
    /// Speed: 0 = paused, 1 = playing, >1 = fast-forward.
    pub speed: i64,
    pub can_seek: bool,
    /// 0–100.
    pub volume_level: f64,
    pub volume_muted: bool,
}

/// Callback the widget registers to receive state updates.
pub type StatusCallback = fn(&KodiMediaStatus);

// ── Internal state ───────────────────────────────────────────────

struct KodiState {
    host: String,
    port: u16,
    /// Pre-built HTTP headers (Content-Type + Authorization).
    headers: String,
    on_status: StatusCallback,
    /// Monotonic JSON-RPC request ID counter.
    next_id: i64,
    /// Active player ID from `Player.GetActivePlayers` (-1 = none).
    active_player_id: i64,
    /// Latest aggregated status.
    status: KodiMediaStatus,
    /// How many parallel responses we're still waiting for in Phase 2.
    pending_responses: u8,
    /// Accumulated time since last poll (ms).
    ms_since_poll: u32,
    /// Consecutive Phase 1 failures.
    fail_count: u8,
    /// Whether the connection has been intentionally closed.
    closed: bool,
}

thread_local! {
    static KODI: RefCell<Option<KodiState>> = const { RefCell::new(None) };
}

// ── Public API ───────────────────────────────────────────────────

/// Connect to a Kodi device.
pub fn connect(host: &str, port: u16, on_status: StatusCallback) {
    log_info!("kodi: connecting to {}:{}", host, port);
    let headers = fmt!(
        "Content-Type: application/json\nAuthorization: Basic {}",
        base64_encode(
            fmt!(
                "{}:{}",
                kv::get_string("kodi_username").unwrap_or_default(),
                kv::get_string("kodi_password").unwrap_or_default(),
            )
            .as_bytes(),
        )
    );
    KODI.with(|k| {
        *k.borrow_mut() = Some(KodiState {
            host: host.into(),
            port,
            headers,
            on_status,
            next_id: 1,
            active_player_id: -1,
            status: KodiMediaStatus::default(),
            pending_responses: 0,
            ms_since_poll: 0,
            fail_count: 0,
            closed: false,
        });
    });
    // Kick off first poll immediately
    poll_active_players();
}

/// Disconnect from the Kodi device.
pub fn disconnect() {
    KODI.with(|k| {
        if let Some(state) = k.borrow_mut().as_mut() {
            state.closed = true;
        }
        *k.borrow_mut() = None;
    });
}

/// Get the authorization header string for Kodi image fetches.
pub fn auth_headers() -> Option<String> {
    KODI.with(|k| k.borrow().as_ref().map(|s| s.headers.clone()))
}

/// Whether the Kodi device is reachable (fewer than `DEAD_THRESHOLD` failures).
pub fn is_alive() -> bool {
    KODI.with(|k| {
        k.borrow()
            .as_ref()
            .is_some_and(|s| !s.closed && s.fail_count < DEAD_THRESHOLD)
    })
}

/// Drive the poll timer. Called from `render(delta_ms)`.
pub fn tick(delta_ms: u32) {
    let should_poll = KODI.with(|k| {
        let mut borrow = k.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };
        if state.closed {
            return false;
        }
        state.ms_since_poll += delta_ms;
        let interval = if state.status.speed > 0 {
            POLL_INTERVAL_MS
        } else {
            POLL_IDLE_INTERVAL_MS
        };
        if state.ms_since_poll >= interval {
            state.ms_since_poll = 0;
            true
        } else {
            false
        }
    });
    if should_poll {
        poll_active_players();
    }
}

/// Send Play command (toggle play/pause).
pub fn play() {
    kodi_command(
        "Player.PlayPause",
        |state| json!({"playerid": #(state.active_player_id), "play": true}),
    );
}

/// Send Pause command (toggle play/pause).
pub fn pause() {
    kodi_command(
        "Player.PlayPause",
        |state| json!({"playerid": #(state.active_player_id), "play": false}),
    );
}

/// Skip to next track.
pub fn next() {
    kodi_command(
        "Player.GoTo",
        |state| json!({"playerid": #(state.active_player_id), "to": "next"}),
    );
}

/// Skip to previous track.
pub fn previous() {
    kodi_command(
        "Player.GoTo",
        |state| json!({"playerid": #(state.active_player_id), "to": "previous"}),
    );
}

/// Seek to a fractional position (0.0–1.0).
pub fn seek(fraction: f64) {
    let pct = (fraction * 100.0).clamp(0.0, 100.0) as u32;
    let Some((url, headers, pid)) = kodi_url_and_player() else {
        return;
    };
    let id = alloc_id();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "Player.Seek",
        "params": {"playerid": #(pid), "value": {"percentage": #(pct)}},
        "id": #(id)
    });
    FetchRequest::post(&url)
        .headers(&headers)
        .body(body.as_bytes())
        .send(on_command_done)
        .is_none()
        .then(|| {
            log_warn!("kodi: seek command rejected by host runtime limits");
        });
}

/// Set volume (0.0–1.0).
pub fn set_volume(level: f32) {
    let vol = (f64::from(level) * 100.0).round().clamp(0.0, 100.0) as u32;
    kodi_fire_and_forget("Application.SetVolume", &json!({"volume": #(vol)}));
}

/// Set mute state.
pub fn set_mute(muted: bool) {
    kodi_fire_and_forget("Application.SetMute", &json!({"mute": #(muted)}));
}

// ── Polling — Phase 1: Active players ────────────────────────────

fn poll_active_players() {
    let Some((url, headers)) = kodi_url() else {
        return;
    };
    let id = alloc_id();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "Player.GetActivePlayers",
        "id": #(id)
    });
    FetchRequest::post(&url)
        .headers(&headers)
        .body(body.as_bytes())
        .send(on_active_players)
        .is_none()
        .then(|| {
            log_warn!("kodi: active player poll rejected by host runtime limits");
        });
}

fn on_active_players(response: &FetchResponse) {
    if !response.ok() {
        log_info!("kodi: GetActivePlayers FAILED");
        KODI.with(|k| {
            let mut borrow = k.borrow_mut();
            if let Some(state) = borrow.as_mut() {
                state.fail_count += 1;
            }
        });
        return;
    }

    let doc = JsonDoc::parse(response.body());
    // Result is an array: [{"playerid": 0, "type": "audio"}]
    let player_id = doc.i64("/result/0/playerid");

    KODI.with(|k| {
        let mut borrow = k.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };
        state.fail_count = 0;

        if let Some(pid) = player_id {
            state.active_player_id = pid;
            // Phase 2: parallel status queries
            state.pending_responses = 3;
        } else {
            // No active player — report NoMedia, poll volume only
            state.active_player_id = -1;
            state.status.player_state.clear();
            state.status.title = None;
            state.status.fields.clear();
            state.status.album_art_url = None;
            state.status.duration_secs = 0.0;
            state.status.current_time = 0.0;
            state.status.speed = 0;
            state.status.can_seek = false;
            state.pending_responses = 1; // volume only
        }
    });

    if player_id.is_some() {
        poll_player_properties();
        poll_player_item();
    }
    poll_app_properties();
}

// ── Polling — Phase 2a: Player properties ────────────────────────

fn poll_player_properties() {
    let Some((url, headers, pid)) = kodi_url_and_player() else {
        return;
    };
    let id = alloc_id();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "Player.GetProperties",
        "params": {"playerid": #(pid), "properties": ["time", "totaltime", "speed", "canseek"]},
        "id": #(id)
    });
    FetchRequest::post(&url)
        .headers(&headers)
        .body(body.as_bytes())
        .send(on_player_properties)
        .is_none()
        .then(|| {
            log_warn!("kodi: player property poll rejected by host runtime limits");
        });
}

fn on_player_properties(response: &FetchResponse) {
    if !response.ok() {
        maybe_fire_callback();
        return;
    }

    let doc = JsonDoc::parse(response.body());

    KODI.with(|k| {
        let mut borrow = k.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };

        // Time: {"hours": H, "minutes": M, "seconds": S, "milliseconds": MS}
        let time_secs = kodi_time_to_secs(&doc, "/result/time");
        let total_secs = kodi_time_to_secs(&doc, "/result/totaltime");
        state.status.current_time = time_secs;
        state.status.duration_secs = total_secs;

        if let Some(speed) = doc.i64("/result/speed") {
            state.status.speed = speed;
            state.status.player_state = if speed > 0 {
                "playing".into()
            } else {
                "paused".into()
            };
        }

        if let Some(can_seek) = doc.bool("/result/canseek") {
            state.status.can_seek = can_seek;
        }
    });

    maybe_fire_callback();
}

// ── Polling — Phase 2b: Player item (metadata) ──────────────────

fn poll_player_item() {
    let Some((url, headers, pid)) = kodi_url_and_player() else {
        return;
    };
    let id = alloc_id();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "Player.GetItem",
        "params": {"playerid": #(pid), "properties": ["title", "artist", "album", "thumbnail"]},
        "id": #(id)
    });
    FetchRequest::post(&url)
        .headers(&headers)
        .body(body.as_bytes())
        .send(on_player_item)
        .is_none()
        .then(|| {
            log_warn!("kodi: player item poll rejected by host runtime limits");
        });
}

fn on_player_item(response: &FetchResponse) {
    if !response.ok() {
        maybe_fire_callback();
        return;
    }

    let doc = JsonDoc::parse(response.body());

    KODI.with(|k| {
        let mut borrow = k.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };

        state.status.title = doc.str("/result/item/title");

        state.status.fields.clear();
        // Artist is an array: ["Artist Name"]
        if let Some(artist) = doc
            .str("/result/item/artist/0")
            .or_else(|| doc.str("/result/item/artist"))
        {
            state.status.fields.push(("Artist".into(), artist));
        }
        if let Some(album) = doc.str("/result/item/album") {
            state.status.fields.push(("Album".into(), album));
        }

        // Thumbnail: Kodi returns "image://..." paths, clear if absent
        state.status.album_art_url = None;
        if let Some(thumb) = doc.str("/result/item/thumbnail") {
            if let Some(image_path) = thumb.strip_prefix("image://") {
                let encoded = percent_encode(image_path);
                let art_url = KODI.with(|_| {
                    // Need host:port — already in state
                    fmt!(
                        "http://{}:{}/image/image%3A%2F%2F{}",
                        state.host,
                        state.port,
                        encoded
                    )
                });
                state.status.album_art_url = Some(art_url);
            } else if !thumb.is_empty() {
                state.status.album_art_url = Some(thumb);
            }
        }
    });

    maybe_fire_callback();
}

// ── Polling — Phase 2c: Application properties (volume) ─────────

fn poll_app_properties() {
    let Some((url, headers)) = kodi_url() else {
        return;
    };
    let id = alloc_id();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "Application.GetProperties",
        "params": {"properties": ["volume", "muted"]},
        "id": #(id)
    });
    FetchRequest::post(&url)
        .headers(&headers)
        .body(body.as_bytes())
        .send(on_app_properties)
        .is_none()
        .then(|| {
            log_warn!("kodi: application property poll rejected by host runtime limits");
        });
}

fn on_app_properties(response: &FetchResponse) {
    if !response.ok() {
        maybe_fire_callback();
        return;
    }

    let doc = JsonDoc::parse(response.body());

    KODI.with(|k| {
        let mut borrow = k.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };

        if let Some(vol) = doc.f64("/result/volume") {
            state.status.volume_level = vol;
        }
        if let Some(muted) = doc.bool("/result/muted") {
            state.status.volume_muted = muted;
        }
    });

    maybe_fire_callback();
}

// ── Callback coordination ────────────────────────────────────────

/// Decrement pending counter; when last response arrives, fire the widget callback.
fn maybe_fire_callback() {
    KODI.with(|k| {
        let mut borrow = k.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };
        if state.pending_responses > 0 {
            state.pending_responses -= 1;
        }
        if state.pending_responses == 0 {
            (state.on_status)(&state.status);
            request_frame();
        }
    });
}

// ── Command helpers ──────────────────────────────────────────────

/// Send a command that requires an active player. Re-polls after response.
fn kodi_command(method: &str, params_fn: fn(&KodiState) -> String) {
    let Some((url, headers, params)) = KODI.with(|k| {
        let borrow = k.borrow();
        let state = borrow.as_ref()?;
        if state.active_player_id < 0 {
            return None;
        }
        let url = fmt!("http://{}:{}/jsonrpc", state.host, state.port);
        let headers = state.headers.clone();
        let params = params_fn(state);
        Some((url, headers, params))
    }) else {
        return;
    };

    let id = alloc_id();
    let body = json!({
        "jsonrpc": "2.0",
        "method": #s(method),
        "params": #(params),
        "id": #(id)
    });
    FetchRequest::post(&url)
        .headers(&headers)
        .body(body.as_bytes())
        .send(on_command_done)
        .is_none()
        .then(|| {
            log_warn!("kodi: command rejected by host runtime limits");
        });
}

/// Fire-and-forget: send command, re-poll on response. No active player required.
fn kodi_fire_and_forget(method: &str, params: &str) {
    let Some((url, headers)) = kodi_url() else {
        return;
    };
    let id = alloc_id();
    let body = json!({
        "jsonrpc": "2.0",
        "method": #s(method),
        "params": #(params),
        "id": #(id)
    });
    FetchRequest::post(&url)
        .headers(&headers)
        .body(body.as_bytes())
        .send(on_command_done)
        .is_none()
        .then(|| {
            log_warn!("kodi: fire-and-forget command rejected by host runtime limits");
        });
}

/// After any command completes, re-poll for fresh state.
fn on_command_done(_response: &FetchResponse) {
    poll_active_players();
}

// ── Utility helpers ──────────────────────────────────────────────

fn kodi_url() -> Option<(String, String)> {
    KODI.with(|k| {
        let borrow = k.borrow();
        let state = borrow.as_ref()?;
        Some((
            fmt!("http://{}:{}/jsonrpc", state.host, state.port),
            state.headers.clone(),
        ))
    })
}

fn kodi_url_and_player() -> Option<(String, String, i64)> {
    KODI.with(|k| {
        let borrow = k.borrow();
        let state = borrow.as_ref()?;
        if state.active_player_id < 0 {
            return None;
        }
        Some((
            fmt!("http://{}:{}/jsonrpc", state.host, state.port),
            state.headers.clone(),
            state.active_player_id,
        ))
    })
}

fn alloc_id() -> i64 {
    KODI.with(|k| {
        let mut borrow = k.borrow_mut();
        match borrow.as_mut() {
            Some(state) => {
                let id = state.next_id;
                state.next_id += 1;
                id
            }
            None => 0,
        }
    })
}

/// Parse Kodi time object `{"hours": H, "minutes": M, "seconds": S}` to seconds.
fn kodi_time_to_secs(doc: &JsonDoc, prefix: &str) -> f64 {
    let h = doc.i64(&fmt!("{}/hours", prefix)).unwrap_or(0);
    let m = doc.i64(&fmt!("{}/minutes", prefix)).unwrap_or(0);
    let s = doc.i64(&fmt!("{}/seconds", prefix)).unwrap_or(0);
    let ms = doc.i64(&fmt!("{}/milliseconds", prefix)).unwrap_or(0);
    (h * 3600 + m * 60 + s) as f64 + ms as f64 / 1000.0
}

/// Percent-encode a string for use in Kodi image URLs.
///
/// Encodes everything except unreserved characters (RFC 3986):
/// `A-Z a-z 0-9 - . _ ~`
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_' || b == b'~' {
            out.push(b as char);
        } else {
            out.push('%');
            // Upper nibble
            let hi = b >> 4;
            out.push(hex_char(hi));
            // Lower nibble
            let lo = b & 0x0F;
            out.push(hex_char(lo));
        }
    }
    out
}

const fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + nibble - 10) as char,
    }
}

/// Base64-encode a byte slice (standard alphabet, with padding).
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let chunks = input.chunks(3);
    for chunk in chunks {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 {
            u32::from(chunk[1])
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            u32::from(chunk[2])
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
