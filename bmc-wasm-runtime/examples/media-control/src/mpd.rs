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

//! MPD (Music Player Daemon) protocol controller.
//!
//! Implements media control via MPD's line-based text protocol over plain TCP:
//! - Connect → receive banner → status/currentsong → readpicture → idle loop
//! - Push updates via `idle player mixer options` subsystem watching
//! - Album art via `readpicture` (chunked binary responses)
//! - Mute emulation (MPD has no native mute — uses `setvol 0` / restore)
//!
//! mDNS discovery: `_mpd._tcp`

use std::cell::RefCell;

use bmc_wasm_sdk::socket::{Socket, SocketEvent, tcp_connect};
#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

// ── Constants ────────────────────────────────────────────────────

/// Liveness timeout (ms) — if no socket activity for this long, send a heartbeat.
const LIVENESS_TIMEOUT_MS: u32 = 30_000;

/// Default volume to restore when unmuting (if we never saw a non-zero volume).
const DEFAULT_UNMUTE_VOLUME: u32 = 50;

// ── Public types ─────────────────────────────────────────────────

/// Parsed media status from an MPD server.
#[derive(Debug, Clone, Default)]
pub struct MpdMediaStatus {
    /// `"play"`, `"pause"`, or `"stop"`.
    pub state: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub elapsed_secs: f64,
    pub duration_secs: f64,
    /// 0–100.
    pub volume: u32,
}

/// Callback the widget registers to receive state updates.
pub type StatusCallback = fn(&MpdMediaStatus);

/// Callback for album art binary data.
pub type ArtCallback = fn(&[u8]);

// ── Internal state ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// TCP connect in progress.
    Connecting,
    /// Waiting for `OK MPD x.y.z\n` banner.
    AwaitingBanner,
    /// Normal operation.
    Ready,
    /// Socket closed.
    Closed,
}

/// What command we're currently waiting for a response to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Status,
    CurrentSong,
    AlbumArt,
    Idle,
    /// A fire-and-forget user command (play, pause, setvol, etc.).
    Fire,
}

/// A queued user command waiting for `noidle` to complete.
#[derive(Debug, Clone)]
struct QueuedCommand {
    cmd: String,
}

/// State for accumulating a chunked `readpicture` response.
struct ArtAccumulator {
    /// Total size of the image (from the `size:` header).
    total_size: usize,
    /// Accumulated image bytes so far.
    data: Vec<u8>,
}

struct MpdState {
    socket: Socket,
    phase: Phase,
    /// Raw bytes accumulated from the socket.
    recv_buf: Vec<u8>,
    /// What command response we're waiting for.
    pending: Option<Pending>,
    /// Key-value pairs accumulated for the current response.
    response: Vec<(String, String)>,
    /// Whether we're currently in idle mode (waiting for subsystem changes).
    idle_active: bool,
    /// Command queued while idle — sent after noidle response arrives.
    queued_command: Option<QueuedCommand>,
    on_status: StatusCallback,
    on_art: ArtCallback,
    /// Last status (carried across status→currentsong sequence).
    status: MpdMediaStatus,
    /// Saved volume for mute emulation.
    volume_before_mute: u32,
    /// Milliseconds since last socket activity (for liveness heartbeat).
    ms_since_activity: u32,
    /// Current song's `file:` field — used for `readpicture`.
    current_file: String,
    /// File URI for which we last fetched/delivered art (avoid re-fetching).
    last_art_file: String,
    /// Binary accumulator for chunked `readpicture` responses.
    art_accum: Option<ArtAccumulator>,
    /// How many binary bytes we still need to read in the current chunk.
    binary_remaining: usize,
}

thread_local! {
    static MPD: RefCell<Option<MpdState>> = const { RefCell::new(None) };
}

// ── Public API ───────────────────────────────────────────────────

pub fn connect(host: &str, port: u16, on_status: StatusCallback, on_art: ArtCallback) {
    disconnect();
    let Some(socket) = tcp_connect(host, port, on_socket_event) else {
        log_warn!("mpd: connect rejected by host runtime limits");
        return;
    };
    MPD.with(|m| {
        *m.borrow_mut() = Some(MpdState {
            socket,
            phase: Phase::Connecting,
            recv_buf: Vec::new(),
            pending: None,
            response: Vec::new(),
            idle_active: false,
            queued_command: None,
            on_status,
            on_art,
            status: MpdMediaStatus::default(),
            volume_before_mute: DEFAULT_UNMUTE_VOLUME,
            ms_since_activity: 0,
            current_file: String::new(),
            last_art_file: String::new(),
            art_accum: None,
            binary_remaining: 0,
        });
    });
}

pub fn disconnect() {
    MPD.with(|m| {
        if let Some(state) = m.borrow_mut().take() {
            state.socket.close();
        }
    });
}

pub fn is_alive() -> bool {
    MPD.with(|m| {
        m.borrow()
            .as_ref()
            .is_some_and(|s| s.phase != Phase::Closed)
    })
}

pub fn tick(delta_ms: u32) {
    MPD.with(|m| {
        let mut m = m.borrow_mut();
        let Some(state) = m.as_mut() else { return };
        if state.phase != Phase::Ready {
            return;
        }
        state.ms_since_activity += delta_ms;
        if state.ms_since_activity >= LIVENESS_TIMEOUT_MS {
            state.ms_since_activity = 0;
            // Break out of idle with a status poll as heartbeat
            if state.idle_active {
                send_noidle_for_refresh(state);
            } else {
                send_cmd(state, "status\n", Pending::Status);
            }
        }
    });
}

pub fn play() {
    send_user_command("play\n");
}

pub fn pause() {
    send_user_command("pause 1\n");
}

pub fn next() {
    send_user_command("next\n");
}

pub fn previous() {
    send_user_command("previous\n");
}

pub fn seek(position_secs: f64) {
    let secs = position_secs as u32;
    send_user_command(&fmt!("seekcur {}\n", secs));
}

pub fn set_volume(level: u32) {
    let clamped = level.min(100);
    send_user_command(&fmt!("setvol {}\n", clamped));
}

pub fn set_mute(muted: bool) {
    MPD.with(|m| {
        let mut m = m.borrow_mut();
        let Some(state) = m.as_mut() else { return };
        if muted {
            // Save current volume and set to 0
            if state.status.volume > 0 {
                state.volume_before_mute = state.status.volume;
            }
            do_send_user_command(state, "setvol 0\n");
        } else {
            // Restore saved volume
            let vol = state.volume_before_mute;
            do_send_user_command(state, &fmt!("setvol {}\n", vol));
        }
    });
}

// ── Socket event handler ─────────────────────────────────────────

fn on_socket_event(_socket: Socket, event: &SocketEvent<'_>) {
    MPD.with(|m| {
        let mut m = m.borrow_mut();
        let Some(state) = m.as_mut() else { return };

        match event {
            SocketEvent::Connected => {
                log_info!("mpd: TCP connected, awaiting banner");
                state.phase = Phase::AwaitingBanner;
                state.ms_since_activity = 0;
            }
            SocketEvent::Data(data) => {
                state.ms_since_activity = 0;
                state.recv_buf.extend_from_slice(data);
                process_data(state);
            }
            SocketEvent::Closed(code) => {
                log_info!("mpd: socket closed (code {})", code);
                state.phase = Phase::Closed;
            }
        }
    });
}

// ── Data processing (text + binary) ──────────────────────────────

fn process_data(state: &mut MpdState) {
    loop {
        if state.binary_remaining > 0 {
            // Binary mode: consume exactly `binary_remaining` bytes
            if state.recv_buf.len() < state.binary_remaining {
                return; // need more data
            }
            let chunk: Vec<u8> = state.recv_buf.drain(..state.binary_remaining).collect();
            state.binary_remaining = 0;

            if let Some(ref mut accum) = state.art_accum {
                accum.data.extend_from_slice(&chunk);
            }

            // After binary data, MPD sends \n then OK\n or more headers.
            // The \n right after binary data is consumed by the next line read.
            continue;
        }

        // Text mode: find the next complete line
        let newline_pos = match state.recv_buf.iter().position(|&b| b == b'\n') {
            Some(pos) => pos,
            None => return, // need more data
        };

        let line_bytes: Vec<u8> = state.recv_buf.drain(..=newline_pos).collect();
        // Strip trailing \n (and \r if present)
        let trimmed = line_bytes.strip_suffix(b"\n").unwrap_or(&line_bytes);
        let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
        let line = String::from_utf8_lossy(trimmed);

        if line.is_empty() {
            continue;
        }

        match state.phase {
            Phase::AwaitingBanner => {
                if line.starts_with("OK MPD") {
                    log_info!("mpd: banner received: {}", &*line);
                    state.phase = Phase::Ready;
                    send_cmd(state, "status\n", Pending::Status);
                } else {
                    log_info!("mpd: unexpected banner line: {}", &*line);
                }
            }
            Phase::Ready => {
                handle_ready_line(state, &line);
            }
            _ => {}
        }
    }
}

fn handle_ready_line(state: &mut MpdState, line: &str) {
    // Check for binary chunk header: "binary: <size>"
    if let Some(size_str) = line.strip_prefix("binary: ") {
        if let Ok(size) = size_str.parse::<usize>() {
            state.binary_remaining = size;
            return;
        }
    }

    if line == "OK" {
        dispatch_response(state);
    } else if line.starts_with("ACK ") {
        log_info!("mpd: error: {}", line);
        state.response.clear();
        // On ACK for album art, just skip art and continue to idle
        if state.pending == Some(Pending::AlbumArt) {
            state.art_accum = None;
            state.pending = None;
            enter_idle(state);
        } else {
            state.pending = None;
            state.idle_active = false;
            send_cmd(state, "status\n", Pending::Status);
        }
    } else if let Some((key, value)) = line.split_once(": ") {
        state.response.push((key.to_string(), value.to_string()));
    }
}

fn dispatch_response(state: &mut MpdState) {
    let pending = state.pending.take();
    let response = std::mem::take(&mut state.response);

    match pending {
        Some(Pending::Status) => {
            parse_status_response(state, &response);
            send_cmd(state, "currentsong\n", Pending::CurrentSong);
        }
        Some(Pending::CurrentSong) => {
            parse_currentsong_response(state, &response);
            (state.on_status)(&state.status);

            // Fetch album art if the file changed
            if !state.current_file.is_empty() && state.current_file != state.last_art_file {
                state.last_art_file.clone_from(&state.current_file);
                state.art_accum = None;
                let cmd = fmt!("readpicture \"{}\" 0\n", state.current_file);
                send_cmd(state, &cmd, Pending::AlbumArt);
            } else {
                enter_idle(state);
            }
        }
        Some(Pending::AlbumArt) => {
            // Check if we got image data in this chunk
            let total_size = response
                .iter()
                .find(|(k, _)| k == "size")
                .and_then(|(_, v)| v.parse::<usize>().ok());

            if let Some(total) = total_size {
                // First or continuation chunk — initialize or keep accumulator
                let accum = state.art_accum.get_or_insert_with(|| ArtAccumulator {
                    total_size: total,
                    data: Vec::with_capacity(total),
                });

                if accum.data.len() >= accum.total_size {
                    // All bytes received — deliver art
                    let art_data = std::mem::take(&mut accum.data);
                    state.art_accum = None;
                    (state.on_art)(&art_data);
                    enter_idle(state);
                } else {
                    // Need more chunks — request next offset
                    let offset = accum.data.len();
                    let cmd = fmt!("readpicture \"{}\" {}\n", state.current_file, offset);
                    send_cmd(state, &cmd, Pending::AlbumArt);
                }
            } else {
                // No size header — no art embedded, or empty response
                state.art_accum = None;
                enter_idle(state);
            }
        }
        Some(Pending::Idle) => {
            state.idle_active = false;
            if let Some(queued) = state.queued_command.take() {
                send_cmd(state, &queued.cmd, Pending::Fire);
            } else {
                send_cmd(state, "status\n", Pending::Status);
            }
        }
        Some(Pending::Fire) => {
            send_cmd(state, "status\n", Pending::Status);
        }
        None => {}
    }
}

fn enter_idle(state: &mut MpdState) {
    send_cmd(state, "idle player mixer options\n", Pending::Idle);
    state.idle_active = true;
}

// ── Response parsers ─────────────────────────────────────────────

fn parse_status_response(state: &mut MpdState, response: &[(String, String)]) {
    state.status.state = "stop".to_string();
    state.status.elapsed_secs = 0.0;
    state.status.duration_secs = 0.0;
    state.status.volume = 100;

    for (key, value) in response {
        match key.as_str() {
            "state" => state.status.state = value.clone(),
            "elapsed" => state.status.elapsed_secs = value.parse().unwrap_or(0.0),
            "duration" => state.status.duration_secs = value.parse().unwrap_or(0.0),
            "volume" => state.status.volume = value.parse().unwrap_or(100),
            _ => {}
        }
    }

    // Track volume for mute emulation — remember last non-zero volume
    if state.status.volume > 0 {
        state.volume_before_mute = state.status.volume;
    }
}

fn parse_currentsong_response(state: &mut MpdState, response: &[(String, String)]) {
    state.status.title = None;
    state.status.artist = None;
    state.status.album = None;
    state.current_file.clear();

    for (key, value) in response {
        match key.as_str() {
            "Title" => state.status.title = Some(value.clone()),
            "Artist" => state.status.artist = Some(value.clone()),
            "Album" => state.status.album = Some(value.clone()),
            "file" => state.current_file = value.clone(),
            _ => {}
        }
    }
}

// ── Command helpers ──────────────────────────────────────────────

fn send_cmd(state: &mut MpdState, cmd: &str, pending: Pending) {
    state.pending = Some(pending);
    state.socket.write(cmd.as_bytes());
}

fn send_user_command(cmd: &str) {
    MPD.with(|m| {
        let mut m = m.borrow_mut();
        let Some(state) = m.as_mut() else { return };
        do_send_user_command(state, cmd);
    });
}

fn do_send_user_command(state: &mut MpdState, cmd: &str) {
    if state.phase != Phase::Ready {
        return;
    }
    if state.idle_active {
        // Queue the command and break out of idle
        state.queued_command = Some(QueuedCommand {
            cmd: cmd.to_string(),
        });
        state.socket.write(b"noidle\n");
    } else if state.pending.is_none() {
        send_cmd(state, cmd, Pending::Fire);
    }
}

fn send_noidle_for_refresh(state: &mut MpdState) {
    if state.idle_active {
        state.queued_command = None;
        state.socket.write(b"noidle\n");
    }
}
