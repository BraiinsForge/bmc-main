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

//! Jellyfin / Emby media server controller.
//!
//! Handles both Jellyfin and Emby via a `ServerType` enum — their REST APIs
//! are nearly identical (Jellyfin forked from Emby).
//!
//! - Single-phase polling: `GET /Sessions` returns everything inline
//! - Auth via API key: `Authorization: MediaBrowser Token="<key>"`
//! - Album art: `GET /Items/{id}/Images/Primary?maxWidth=300&maxHeight=300`

use std::cell::RefCell;

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

// ── Constants ────────────────────────────────────────────────────

/// Poll interval while playing (ms).
pub const POLL_INTERVAL_MS: u32 = 1_000;
/// Poll interval when idle/no media (ms).
pub const POLL_IDLE_INTERVAL_MS: u32 = 3_000;

/// Consecutive poll failures before `is_alive()` returns false.
const DEAD_THRESHOLD: u8 = 5;

// ── Public types ─────────────────────────────────────────────────

/// Whether we're talking to a Jellyfin or Emby server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerType {
    Jellyfin,
    Emby,
}

/// Parsed media status from a Jellyfin/Emby device.
#[derive(Debug, Clone, Default)]
pub struct JellyfinMediaStatus {
    /// `"playing"`, `"paused"`, or `""` (no media).
    pub player_state: String,
    pub title: Option<String>,
    /// Secondary metadata lines.
    pub fields: Vec<(String, String)>,
    pub album_art_url: Option<String>,
    pub duration_secs: f64,
    pub current_time: f64,
    pub can_seek: bool,
    /// 0–100.
    pub volume_level: f64,
    pub volume_muted: bool,
}

/// Callback the widget registers to receive state updates.
pub type StatusCallback = fn(&JellyfinMediaStatus);

// ── Internal state ───────────────────────────────────────────────

/// A discovered client session on the server.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub device_name: String,
    pub client: String,
    pub has_now_playing: bool,
}

struct State {
    host: String,
    port: u16,
    server_type: ServerType,
    /// Pre-built HTTP header: `Authorization: MediaBrowser Token="..."`.
    headers: String,
    /// All controllable sessions from last poll.
    sessions: Vec<Session>,
    /// Currently selected session ID (`None` = auto-pick).
    target_session: Option<String>,
    on_status: StatusCallback,
    status: JellyfinMediaStatus,
    /// Accumulated time since last poll (ms).
    ms_since_poll: u32,
    /// Consecutive poll failures.
    fail_count: u8,
    /// Server returned 401/403 — API key is missing or invalid.
    auth_required: bool,
    /// Whether the connection has been intentionally closed.
    closed: bool,
}

thread_local! {
    static SERVER: RefCell<Option<State >> = const { RefCell::new(None) };
}

/// Zero-pad a non-negative integer to two digits (`5` → `"05"`, `12` → `"12"`).
/// Negative or 3+ digit values pass through unchanged.
/// `ufmt` has no width specifier, so we do this by hand.
fn pad2(n: i64) -> String {
    if (0..10).contains(&n) {
        fmt!("0{n}")
    } else {
        fmt!("{n}")
    }
}

// ── Public API ───────────────────────────────────────────────────

/// Connect to a Jellyfin or Emby device.
pub fn connect(host: &str, port: u16, server_type: ServerType, on_status: StatusCallback) {
    let label = match server_type {
        ServerType::Jellyfin => "jellyfin",
        ServerType::Emby => "emby",
    };
    log_info!("{}: connecting to {}:{}", label, host, port);

    let kv_key = fmt!("{}_api_key", label);
    let api_key = kv::get_string(&kv_key).unwrap_or_default();
    if api_key.is_empty() {
        log_info!("{}: no API key found in KV ({})", label, kv_key);
    }

    let headers = if api_key.is_empty() {
        String::new()
    } else {
        fmt!("Authorization: MediaBrowser Token=\"{}\"", api_key)
    };

    SERVER.with(|j| {
        *j.borrow_mut() = Some(State {
            host: host.into(),
            port,
            server_type,
            headers,
            sessions: Vec::new(),
            target_session: None,
            on_status,
            status: JellyfinMediaStatus::default(),
            ms_since_poll: 0,
            fail_count: 0,
            auth_required: false,
            closed: false,
        });
    });
    // Kick off first poll immediately
    poll_sessions();
}

/// Disconnect from the Jellyfin/Emby device.
pub fn disconnect() {
    SERVER.with(|j| {
        if let Some(state) = j.borrow_mut().as_mut() {
            state.closed = true;
        }
        *j.borrow_mut() = None;
    });
}

/// Get the authorization header string for image fetches.
pub fn auth_headers() -> Option<String> {
    SERVER.with(|j| j.borrow().as_ref().map(|s| s.headers.clone()))
}

/// Whether the server returned 401/403 — API key is missing or invalid.
pub fn auth_required() -> bool {
    SERVER.with(|j| j.borrow().as_ref().is_some_and(|s| s.auth_required))
}

/// The KV key name for the API token of the active server type.
pub fn auth_kv_key() -> String {
    SERVER.with(|j| {
        let label = match j.borrow().as_ref().map(|s| s.server_type) {
            Some(ServerType::Emby) => "emby",
            _ => "jellyfin",
        };
        fmt!("{}_api_key", label)
    })
}

/// The currently targeted session ID (if any).
pub fn active_session_id() -> Option<String> {
    SERVER.with(|j| j.borrow().as_ref().and_then(|s| s.target_session.clone()))
}

/// All controllable sessions from the last poll.
pub fn sessions() -> Vec<Session> {
    SERVER.with(|j| {
        j.borrow()
            .as_ref()
            .map(|s| s.sessions.clone())
            .unwrap_or_default()
    })
}

/// Select a session by ID for control. Triggers a re-poll.
pub fn select_session(id: &str) {
    SERVER.with(|j| {
        if let Some(state) = j.borrow_mut().as_mut() {
            state.target_session = Some(id.into());
        }
    });
    poll_sessions();
}

/// Clear the selected session (return to session picker).
pub fn clear_session() {
    SERVER.with(|j| {
        if let Some(state) = j.borrow_mut().as_mut() {
            state.target_session = None;
            state.status = JellyfinMediaStatus::default();
            (state.on_status)(&state.status);
        }
    });
}

/// Whether the server is reachable (fewer than `DEAD_THRESHOLD` failures).
pub fn is_alive() -> bool {
    SERVER.with(|j| {
        j.borrow()
            .as_ref()
            .is_some_and(|s| !s.closed && s.fail_count < DEAD_THRESHOLD)
    })
}

/// Drive the poll timer. Called from `render(delta_ms)`.
pub fn tick(delta_ms: u32) {
    let should_poll = SERVER.with(|j| {
        let mut borrow = j.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };
        if state.closed || state.fail_count >= DEAD_THRESHOLD {
            return false;
        }
        state.ms_since_poll += delta_ms;
        let interval = if state.status.player_state == "playing" {
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
        poll_sessions();
    }
}

// ── Playback commands ────────────────────────────────────────────

/// Resume playback.
pub fn play() {
    jellyfin_session_command("Playing/Unpause", None);
}

/// Pause playback.
pub fn pause() {
    jellyfin_session_command("Playing/Pause", None);
}

/// Skip to next track.
pub fn next() {
    jellyfin_session_command("Playing/NextTrack", None);
}

/// Skip to previous track.
pub fn previous() {
    jellyfin_session_command("Playing/PreviousTrack", None);
}

/// Seek to an absolute position in seconds.
pub fn seek(position_secs: u32) {
    let ticks = u64::from(position_secs) * 10_000_000;
    let path = fmt!("Playing/Seek?SeekPositionTicks={}", ticks);
    jellyfin_session_command(&path, None);
}

/// Set volume (0.0–1.0).
pub fn set_volume(level: f32) {
    let vol = (f64::from(level) * 100.0).round().clamp(0.0, 100.0) as u32;
    let body = json!({"Name": "SetVolume", "Arguments": {"Volume": #s(&fmt!("{}", vol))}});
    jellyfin_general_command(&body);
}

/// Set mute state.
pub fn set_mute(muted: bool) {
    let name = if muted { "Mute" } else { "Unmute" };
    let body = json!({"Name": #s(name)});
    jellyfin_general_command(&body);
}

// ── Command helpers ──────────────────────────────────────────────

/// Send a GeneralCommand (`POST /Sessions/{id}/Command`). Re-polls after response.
fn jellyfin_general_command(body: &str) {
    jellyfin_session_command("Command", Some(body));
}

/// Send a session command. Re-polls after response.
fn jellyfin_session_command(subpath: &str, body: Option<&str>) {
    let Some((base_url, headers, session_id)) = SERVER.with(|j| {
        let borrow = j.borrow();
        let state = borrow.as_ref()?;
        let session_id = state.target_session.as_ref()?.clone();
        Some((
            fmt!("http://{}:{}", state.host, state.port),
            state.headers.clone(),
            session_id,
        ))
    }) else {
        return;
    };

    let url = fmt!("{}/Sessions/{}/{}", base_url, session_id, subpath);
    let all_headers = if headers.is_empty() {
        "Content-Type: application/json".into()
    } else {
        fmt!("{}\nContent-Type: application/json", headers)
    };
    let req = FetchRequest::post(&url).headers(&all_headers);
    if let Some(body) = body {
        if req.body(body.as_bytes()).send(on_command_done).is_none() {
            log_warn!("jellyfin: command rejected by host runtime limits");
        }
    } else {
        if req.send(on_command_done).is_none() {
            log_warn!("jellyfin: command rejected by host runtime limits");
        }
    }
}

// ── Polling ──────────────────────────────────────────────────────

fn poll_sessions() {
    let Some((url, headers)) = jellyfin_url("/Sessions") else {
        return;
    };
    let mut req = FetchRequest::get(&url);
    if !headers.is_empty() {
        req = req.headers(&headers);
    }
    if req.send(on_sessions_response).is_none() {
        log_warn!("jellyfin: sessions poll rejected by host runtime limits");
    }
}

fn on_sessions_response(response: &FetchResponse) {
    if !response.ok() {
        SERVER.with(|j| {
            if let Some(state) = j.borrow_mut().as_mut() {
                state.fail_count += 1;
                if response.status == 401 || response.status == 403 {
                    state.auth_required = true;
                    log_info!(
                        "jellyfin: authentication required (HTTP {})",
                        response.status
                    );
                }
            }
        });
        return;
    }

    let doc = JsonDoc::parse(response.body());

    // Collect all controllable sessions
    let mut all_sessions = Vec::new();
    for i in 0..50 {
        let prefix = fmt!("/{}", i);
        let Some(id) = doc.str(&fmt!("{}/Id", prefix)) else {
            break;
        };
        // Skip sessions that don't support remote control
        if doc.bool(&fmt!("{}/SupportsRemoteControl", prefix)) == Some(false) {
            continue;
        }
        let device_name = doc.str(&fmt!("{}/DeviceName", prefix)).unwrap_or_default();
        let client = doc.str(&fmt!("{}/Client", prefix)).unwrap_or_default();
        let has_now_playing = doc.str(&fmt!("{}/NowPlayingItem/Name", prefix)).is_some();
        all_sessions.push((
            i,
            Session {
                id,
                device_name,
                client,
                has_now_playing,
            },
        ));
    }

    SERVER.with(|j| {
        let mut borrow = j.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };
        state.fail_count = 0;

        // Store sessions for sub_targets()
        state.sessions = all_sessions.iter().map(|(_, s)| s.clone()).collect();

        // Resolve which session to control:
        // 1. If user explicitly selected one, keep it (if still present)
        // 2. Otherwise auto-pick first with NowPlayingItem
        let target_idx = if let Some(ref selected) = state.target_session {
            all_sessions
                .iter()
                .find(|(_, s)| s.id == *selected)
                .map(|(i, _)| *i)
        } else {
            all_sessions
                .iter()
                .find(|(_, s)| s.has_now_playing)
                .map(|(i, _)| *i)
        };

        if let Some(idx) = target_idx {
            let prefix = fmt!("/{}", idx);
            let session_id = doc.str(&fmt!("{}/Id", prefix)).unwrap_or_default();
            state.target_session = Some(session_id.to_string());

            let np = fmt!("{}/NowPlayingItem", prefix);

            // Title & metadata — adapt to content type
            let name = doc.str(&fmt!("{}/Name", np));
            let media_type = doc.str(&fmt!("{}/Type", np)).unwrap_or_default();

            state.status.title = name;
            state.status.fields.clear();

            match media_type.as_str() {
                "Episode" => {
                    if let Some(series) = doc.str(&fmt!("{}/SeriesName", np)) {
                        let season_num = doc.i64(&fmt!("{}/ParentIndexNumber", np));
                        let episode_num = doc.i64(&fmt!("{}/IndexNumber", np));
                        let series_val = match (season_num, episode_num) {
                            (Some(s), Some(e)) => {
                                let (s, e) = (pad2(s), pad2(e));
                                fmt!("{series} S{s}E{e}")
                            }
                            (None, Some(e)) => {
                                let e = pad2(e);
                                fmt!("{series} E{e}")
                            }
                            _ => series,
                        };
                        state.status.fields.push(("Series".into(), series_val));
                    }
                    if let Some(season) = doc.str(&fmt!("{}/SeasonName", np)) {
                        state.status.fields.push(("Season".into(), season));
                    }
                }
                "Movie" => {
                    if let Some(year) = doc.i64(&fmt!("{}/ProductionYear", np)) {
                        state.status.fields.push(("Year".into(), fmt!("{}", year)));
                    }
                    if let Some(genre) = doc.str(&fmt!("{}/Genres/0", np)) {
                        state.status.fields.push(("Genre".into(), genre));
                    }
                }
                // Audio, MusicVideo, etc.
                _ => {
                    if let Some(artist) = doc
                        .str(&fmt!("{}/AlbumArtist", np))
                        .or_else(|| doc.str(&fmt!("{}/Artists/0", np)))
                    {
                        state.status.fields.push(("Artist".into(), artist));
                    }
                    if let Some(album) = doc.str(&fmt!("{}/Album", np)) {
                        state.status.fields.push(("Album".into(), album));
                    }
                }
            }

            // Duration: RunTimeTicks (100ns units) -> seconds
            let duration_ticks = doc.i64(&fmt!("{}/RunTimeTicks", np)).unwrap_or(0);
            state.status.duration_secs = duration_ticks as f64 / 10_000_000.0;

            // Current position from PlayState
            let ps = fmt!("{}/PlayState", prefix);
            let pos_ticks = doc.i64(&fmt!("{}/PositionTicks", ps)).unwrap_or(0);
            state.status.current_time = pos_ticks as f64 / 10_000_000.0;

            let is_paused = doc.bool(&fmt!("{}/IsPaused", ps)).unwrap_or(false);
            state.status.player_state = if is_paused {
                "paused".into()
            } else {
                "playing".into()
            };
            state.status.can_seek = true;

            // Volume
            if let Some(vol) = doc.i64(&fmt!("{}/VolumeLevel", ps)) {
                state.status.volume_level = vol as f64;
            }
            state.status.volume_muted = doc.bool(&fmt!("{}/IsMuted", ps)).unwrap_or(false);

            // Album art URL
            let item_id = doc.str(&fmt!("{}/Id", np));
            if let Some(id) = item_id {
                state.status.album_art_url = Some(fmt!(
                    "http://{}:{}/Items/{}/Images/Primary?maxWidth=300&maxHeight=300",
                    state.host,
                    state.port,
                    id
                ));
            } else {
                state.status.album_art_url = None;
            }
        } else {
            // No active session
            state.target_session = None;
            state.status = JellyfinMediaStatus {
                player_state: String::new(),
                ..Default::default()
            };
        }

        (state.on_status)(&state.status);
    });

    request_frame();
}

// ── Utility helpers ──────────────────────────────────────────────

fn jellyfin_url(path: &str) -> Option<(String, String)> {
    SERVER.with(|j| {
        let borrow = j.borrow();
        let state = borrow.as_ref()?;
        Some((
            fmt!("http://{}:{}{}", state.host, state.port, path),
            state.headers.clone(),
        ))
    })
}

/// After any command completes, re-poll for fresh state.
fn on_command_done(_response: &FetchResponse) {
    poll_sessions();
}
