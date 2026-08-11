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

//! Google Cast (CastV2) protocol controller.
//!
//! Implements the Chromecast protocol over TLS sockets:
//! - Length-prefixed protobuf framing
//! - Heartbeat (PING/PONG every 5s)
//! - Connection management (sender-0 ↔ receiver-0, then transport sessions)
//! - Receiver status (running apps, volume)
//! - Media session (play/pause/stop/seek/next/prev, track metadata)

use std::cell::RefCell;

use bmc_wasm_sdk::socket::{Socket, SocketEvent, tls_connect_insecure};
#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;
use prost::Message;

// Generated protobuf types from cast_channel.proto. `allow(dead_code)`
// because prost generates every message in the .proto regardless of which
// ones we actually reference (e.g. `DeviceAuthMessage`).
mod proto {
    #![allow(clippy::trivially_copy_pass_by_ref, dead_code)]
    include!(concat!(env!("OUT_DIR"), "/cast_channel.rs"));
}

use proto::CastMessage;
use proto::cast_message::{PayloadType, ProtocolVersion};

// ── Namespaces ──────────────────────────────────────────────────

const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";

/// Default sender/receiver virtual endpoints.
const SENDER_ID: &str = "sender-0";
const RECEIVER_ID: &str = "receiver-0";

/// Heartbeat interval (ms).
pub const HEARTBEAT_MS: u32 = 5_000;

// ── State machine ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// TLS handshake in progress.
    Connecting,
    /// TLS connected, sent CONNECT + GET_STATUS to receiver.
    AwaitingReceiverStatus,
    /// Receiver has a media app running, connected to its transport.
    MediaSession,
    /// Connection was lost or intentionally closed.
    Closed,
    /// Waiting to reconnect (accumulates ms_since_heartbeat as delay timer).
    Reconnecting,
}

/// Cast metadata type (from metadataType field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContentType {
    #[default]
    Generic,
    Movie,
    TvShow,
    Music,
    Photo,
}

/// Parsed media status from a Cast device.
#[derive(Debug, Clone, Default)]
pub struct CastMediaStatus {
    pub media_session_id: i64,
    pub player_state: String,
    pub content_type: ContentType,
    pub title: Option<String>,
    /// Secondary metadata lines — protocol decides labels and order.
    pub fields: Vec<(String, String)>,
    pub album_art_url: Option<String>,
    pub duration_secs: f64,
    pub current_time: f64,
    pub volume_level: f32,
    pub volume_muted: bool,
    /// Bitmask from `supportedMediaCommands` (0 = unknown/all).
    pub supported_commands: u32,
    /// Stream type: "BUFFERED", "LIVE", or "NONE".
    pub stream_type: Option<String>,
}

// Cast supportedMediaCommands bitmask flags.
const CMD_PAUSE: u32 = 1;
const CMD_SEEK: u32 = 2;
const CMD_QUEUE_NEXT: u32 = 64;
const CMD_QUEUE_PREV: u32 = 128;

impl CastMediaStatus {
    /// Convert Cast capabilities to `TransportActions`.
    ///
    /// When `supported_commands == 0` (not yet received), defaults to all-true.
    pub fn transport_actions(&self) -> crate::upnp::TransportActions {
        let cmds = self.supported_commands;
        if cmds == 0 {
            return crate::upnp::TransportActions::default();
        }
        let is_live = self.stream_type.as_deref() == Some("LIVE");
        crate::upnp::TransportActions {
            can_play: true, // Cast always allows play
            can_pause: cmds & CMD_PAUSE != 0,
            can_seek: cmds & CMD_SEEK != 0 && !is_live,
            can_next: cmds & CMD_QUEUE_NEXT != 0,
            can_previous: cmds & CMD_QUEUE_PREV != 0,
        }
    }
}

/// Callback the widget registers to receive state updates.
pub type StatusCallback = fn(&CastMediaStatus);

/// Reconnect delay after socket close (ms).
const RECONNECT_DELAY_MS: u32 = 3_000;

/// Per-connection Cast state.
struct CastState {
    socket: Socket,
    phase: Phase,
    /// Incoming data buffer for frame reassembly.
    recv_buf: Vec<u8>,
    /// Monotonic request ID counter.
    next_request_id: i64,
    /// Transport ID of the active media app (e.g. "web-5").
    transport_id: Option<String>,
    /// Media session ID for media commands.
    media_session_id: Option<i64>,
    /// Latest parsed media status.
    status: CastMediaStatus,
    /// Widget callback for status updates.
    on_status: StatusCallback,
    /// Frames since last PONG (for timeout detection).
    heartbeat_miss_count: u8,
    /// Milliseconds since last heartbeat PING.
    ms_since_heartbeat: u32,
    /// Connection parameters for reconnect.
    host: String,
    port: u16,
    /// How many times we've tried to reconnect (for backoff).
    reconnect_attempts: u8,
    /// Request ID of a pending SET_VOLUME. While set, incoming volume from
    /// RECEIVER_STATUS is suppressed to prevent stale in-flight responses
    /// from overwriting our optimistic local update.
    pending_volume_req: Option<i64>,
}

thread_local! {
    static CAST: RefCell<Option<CastState>> = const { RefCell::new(None) };
}

// ── Public API ──────────────────────────────────────────────────

/// Connect to a Google Cast device.
pub fn connect(host: &str, port: u16, on_status: StatusCallback) {
    let Some(socket) = tls_connect_insecure(host, port, on_socket_event) else {
        log_warn!("cast: connect rejected by host runtime limits");
        return;
    };
    CAST.with(|c| {
        *c.borrow_mut() = Some(CastState {
            socket,
            phase: Phase::Connecting,
            recv_buf: Vec::new(),
            next_request_id: 1,
            transport_id: None,
            media_session_id: None,
            status: CastMediaStatus::default(),
            on_status,
            heartbeat_miss_count: 0,
            ms_since_heartbeat: 0,
            host: host.into(),
            port,
            reconnect_attempts: 0,
            pending_volume_req: None,
        });
    });
}

/// Disconnect from the Cast device.
pub fn disconnect() {
    CAST.with(|c| {
        if let Some(state) = c.borrow_mut().take() {
            state.socket.close();
        }
    });
}

/// Whether the Cast connection is alive (connecting, awaiting status, or in session).
/// Returns false when closed or reconnecting.
pub fn is_alive() -> bool {
    CAST.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|s| !matches!(s.phase, Phase::Closed | Phase::Reconnecting))
    })
}

/// Send Play command.
pub fn play() {
    with_media_command("PLAY", "");
}

/// Send Pause command.
pub fn pause() {
    with_media_command("PAUSE", "");
}

/// Seek to position in seconds.
pub fn seek(position_secs: f64) {
    let secs = position_secs as u32;
    let extra = fmt!(", \"currentTime\": {}", secs);
    with_media_command("SEEK", &extra);
}

/// Skip to next track.
pub fn next() {
    with_media_command("QUEUE_NEXT", "");
}

/// Skip to previous track.
pub fn previous() {
    with_media_command("QUEUE_PREV", "");
}

/// Set volume (0.0–1.0).
pub fn set_volume(level: f32) {
    CAST.with(|c| {
        let mut borrow = c.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };
        let req_id = state.alloc_request_id();
        state.pending_volume_req = Some(req_id);
        let level_str = f32_json(level);
        let msg = json!({"type": "SET_VOLUME", "volume": {"level": #(level_str)}, "requestId": #(req_id)});
        send_json(state, NS_RECEIVER, SENDER_ID, RECEIVER_ID, &msg);
    });
}

/// Set mute state.
pub fn set_mute(muted: bool) {
    CAST.with(|c| {
        let mut borrow = c.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };
        let req_id = state.alloc_request_id();
        state.pending_volume_req = Some(req_id);
        let msg =
            json!({"type": "SET_VOLUME", "volume": {"muted": #(muted)}, "requestId": #(req_id)});
        send_json(state, NS_RECEIVER, SENDER_ID, RECEIVER_ID, &msg);
    });
}

/// Called from render(delta_ms) to drive heartbeat timing and reconnect.
pub fn tick(delta_ms: u32) {
    CAST.with(|c| {
        let mut borrow = c.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };

        match state.phase {
            Phase::Closed | Phase::Connecting => return,
            Phase::Reconnecting => {
                state.ms_since_heartbeat += delta_ms;
                let delay = RECONNECT_DELAY_MS * u32::from(state.reconnect_attempts.min(5));
                if state.ms_since_heartbeat >= delay {
                    log_info!("cast: reconnecting to {}:{}", state.host, state.port);
                    // New socket but preserve accumulated media status
                    let Some(socket) =
                        tls_connect_insecure(&state.host, state.port, on_socket_event)
                    else {
                        log_warn!("cast: reconnect rejected by host runtime limits");
                        state.reconnect_attempts = state.reconnect_attempts.saturating_add(1);
                        state.ms_since_heartbeat = 0;
                        request_frame_after(delay);
                        return;
                    };
                    state.socket = socket;
                    state.phase = Phase::Connecting;
                    state.recv_buf.clear();
                    state.next_request_id = 1;
                    state.transport_id = None;
                    state.media_session_id = None;
                    state.heartbeat_miss_count = 0;
                    state.ms_since_heartbeat = 0;
                }
                return;
            }
            _ => {}
        }

        state.ms_since_heartbeat += delta_ms;
        if state.ms_since_heartbeat >= HEARTBEAT_MS {
            send_heartbeat(state);
            // Schedule next render for heartbeat check
            request_frame_after(HEARTBEAT_MS);
        } else if delta_ms > 100 {
            // Log significant tick calls to debug heartbeat chain
            log_info!(
                "cast: tick delta={}ms accum={}ms",
                delta_ms,
                state.ms_since_heartbeat
            );
        }
    });
}

/// Send heartbeat PING and check for timeout.
fn send_heartbeat(state: &mut CastState) {
    log_info!(
        "cast: PING (miss={}, phase={})",
        state.heartbeat_miss_count,
        state.phase.as_str()
    );
    send_json(
        state,
        NS_HEARTBEAT,
        SENDER_ID,
        RECEIVER_ID,
        "{\"type\": \"PING\"}",
    );
    state.heartbeat_miss_count += 1;
    state.ms_since_heartbeat = 0;

    match state.phase {
        // Re-poll receiver status when waiting for a media app to start
        Phase::AwaitingReceiverStatus => {
            let req_id = state.alloc_request_id();
            let msg = json!({"type": "GET_STATUS", "requestId": #(req_id)});
            send_json(state, NS_RECEIVER, SENDER_ID, RECEIVER_ID, &msg);
        }
        // Poll media status for position updates during active session
        Phase::MediaSession => {
            if let Some(ref tid) = state.transport_id.clone() {
                let req_id = state.alloc_request_id();
                let msg = json!({"type": "GET_STATUS", "requestId": #(req_id)});
                send_json(state, NS_MEDIA, SENDER_ID, tid, &msg);
            }
        }
        _ => {}
    }

    if state.heartbeat_miss_count > 3 {
        log_warn!("cast: heartbeat timeout, closing");
        state.phase = Phase::Closed;
        state.socket.close();
    }
}

// ── Socket event handler ────────────────────────────────────────

fn on_socket_event(_socket: Socket, event: &SocketEvent<'_>) {
    match event {
        SocketEvent::Connected => {
            log_info!("cast: TLS connected");
            CAST.with(|c| {
                let mut borrow = c.borrow_mut();
                let Some(state) = borrow.as_mut() else {
                    return;
                };
                state.phase = Phase::AwaitingReceiverStatus;
                state.reconnect_attempts = 0;

                // 1. Open virtual connection to receiver-0
                send_json(
                    state,
                    NS_CONNECTION,
                    SENDER_ID,
                    RECEIVER_ID,
                    "{\"type\": \"CONNECT\"}",
                );

                // 2. Request receiver status (to discover running apps)
                let req_id = state.alloc_request_id();
                let msg = json!({"type": "GET_STATUS", "requestId": #(req_id)});
                send_json(state, NS_RECEIVER, SENDER_ID, RECEIVER_ID, &msg);

                // 3. Send first heartbeat and schedule periodic ticks
                send_heartbeat(state);
                request_frame_after(HEARTBEAT_MS);
            });
            request_frame();
        }
        SocketEvent::Data(data) => {
            CAST.with(|c| {
                let mut borrow = c.borrow_mut();
                let Some(state) = borrow.as_mut() else {
                    return;
                };
                state.recv_buf.extend_from_slice(data);
                process_frames(state);
            });
        }
        SocketEvent::Closed(code) => {
            CAST.with(|c| {
                let mut borrow = c.borrow_mut();
                if let Some(state) = borrow.as_mut() {
                    state.reconnect_attempts += 1;
                    let delay = RECONNECT_DELAY_MS * u32::from(state.reconnect_attempts.min(5));
                    log_info!(
                        "cast: socket closed (code {}), reconnect in {}ms",
                        code,
                        delay
                    );
                    state.phase = Phase::Reconnecting;
                    state.ms_since_heartbeat = 0;
                    state.recv_buf.clear();
                    state.transport_id = None;
                    state.media_session_id = None;
                    state.heartbeat_miss_count = 0;
                    request_frame_after(delay);
                }
            });
        }
    }
}

// ── Frame processing ────────────────────────────────────────────

/// Extract and process complete length-prefixed protobuf frames from recv_buf.
fn process_frames(state: &mut CastState) {
    loop {
        if state.recv_buf.len() < 4 {
            return;
        }
        let frame_len = u32::from_be_bytes([
            state.recv_buf[0],
            state.recv_buf[1],
            state.recv_buf[2],
            state.recv_buf[3],
        ]) as usize;

        if state.recv_buf.len() < 4 + frame_len {
            return; // incomplete frame
        }

        let frame_data = state.recv_buf[4..4 + frame_len].to_vec();
        state.recv_buf.drain(..4 + frame_len);

        match CastMessage::decode(frame_data.as_slice()) {
            Ok(msg) => handle_message(state, &msg),
            Err(_) => log_warn!("cast: protobuf decode error"),
        }
    }
}

/// Route a decoded CastMessage to the appropriate handler.
fn handle_message(state: &mut CastState, msg: &CastMessage) {
    let payload = match &msg.payload_utf8 {
        Some(s) => s.as_str(),
        None => return, // binary payloads (auth) ignored
    };

    // Trim namespace prefix for concise logging
    let ns_short = msg
        .namespace
        .strip_prefix("urn:x-cast:com.google.cast.")
        .unwrap_or(&msg.namespace);
    let doc = JsonDoc::parse(payload.as_bytes());
    let msg_type = doc.str("/type").unwrap_or_default();
    log_info!(
        "cast: recv [{}] type={} from={}",
        ns_short,
        msg_type,
        msg.source_id
    );

    match msg.namespace.as_str() {
        NS_HEARTBEAT => handle_heartbeat(state, payload),
        NS_RECEIVER => handle_receiver(state, payload),
        NS_MEDIA => handle_media(state, payload),
        NS_CONNECTION => {
            if msg_type == "CLOSE" {
                log_warn!("cast: CLOSE received from {}", msg.source_id);
            }
        }
        ns => log_info!("cast: unknown namespace: {}", ns),
    }
}

fn handle_heartbeat(state: &mut CastState, payload: &str) {
    let doc = JsonDoc::parse(payload.as_bytes());
    let msg_type = doc.str("/type").unwrap_or_default();
    match msg_type.as_str() {
        "PONG" => {
            state.heartbeat_miss_count = 0;
        }
        "PING" => {
            // Device sent us a PING — respond with PONG
            send_json(
                state,
                NS_HEARTBEAT,
                SENDER_ID,
                RECEIVER_ID,
                "{\"type\": \"PONG\"}",
            );
        }
        _ => {}
    }
}

/// Check whether app at index 0 declares the media namespace in its supportedNamespaces.
fn app_has_media_ns(doc: &JsonDoc) -> bool {
    for i in 0..10_u32 {
        let path = fmt!("/status/applications/0/namespaces/{}/name", i);
        match doc.str(&path) {
            Some(ns) if ns == NS_MEDIA => return true,
            Some(_) => {}
            None => break,
        }
    }
    false
}

fn handle_receiver(state: &mut CastState, payload: &str) {
    let doc = JsonDoc::parse(payload.as_bytes());
    let msg_type = doc.str("/type").unwrap_or_default();

    if msg_type != "RECEIVER_STATUS" {
        return;
    }

    // Extract volume from receiver status.
    // If we have a pending SET_VOLUME, suppress stale volume from earlier
    // in-flight responses. Clear the guard when we see our request ID.
    let resp_req_id = doc.i64("/requestId");
    let volume_suppressed = match (state.pending_volume_req, resp_req_id) {
        (Some(pending), Some(resp)) if resp >= pending => {
            // This is the response to our SET_VOLUME (or later) — accept it
            state.pending_volume_req = None;
            false
        }
        (Some(_), _) => true, // stale response, suppress volume
        (None, _) => false,
    };

    if !volume_suppressed {
        if let Some(level) = doc.f64("/status/volume/level") {
            state.status.volume_level = level as f32;
        }
        if let Some(muted) = doc.bool("/status/volume/muted") {
            state.status.volume_muted = muted;
        }
    }

    // Look for a running media app.
    // Only connect to apps that declare the media namespace in supportedNamespaces.
    let transport_id = doc.str("/status/applications/0/transportId");
    let app_id = doc.str("/status/applications/0/appId").unwrap_or_default();
    let display_name = doc
        .str("/status/applications/0/displayName")
        .unwrap_or_default();
    let has_media_ns = app_has_media_ns(&doc);

    if let Some(ref tid) = transport_id {
        if has_media_ns {
            let already_connected = state.transport_id.as_deref() == Some(tid.as_str());
            if !already_connected {
                log_info!(
                    "cast: app '{}' ({}) transport={}",
                    display_name,
                    app_id,
                    tid
                );
                state.transport_id = Some(tid.clone());

                // Connect to the app's transport
                let connect_msg = json!({"type": "CONNECT", "origin": {}});
                send_json(state, NS_CONNECTION, SENDER_ID, tid, &connect_msg);

                // Request media status
                let req_id = state.alloc_request_id();
                let msg = json!({"type": "GET_STATUS", "requestId": #(req_id)});
                send_json(state, NS_MEDIA, SENDER_ID, tid, &msg);

                state.phase = Phase::MediaSession;
            }
        } else {
            log_info!(
                "cast: app '{}' ({}) has no media ns, waiting",
                display_name,
                app_id
            );
            // If we had a previous media transport, drop it
            if state.transport_id.is_some() {
                state.transport_id = None;
                state.media_session_id = None;
                state.status = CastMediaStatus::default();
                state.phase = Phase::AwaitingReceiverStatus;
            }
        }
    } else {
        log_info!("cast: receiver has no running apps");
        if state.transport_id.is_some() {
            state.transport_id = None;
            state.media_session_id = None;
            state.status = CastMediaStatus::default();
            state.phase = Phase::AwaitingReceiverStatus;
        }
    }

    (state.on_status)(&state.status);
    request_frame();
}

fn handle_media(state: &mut CastState, payload: &str) {
    let doc = JsonDoc::parse(payload.as_bytes());
    let msg_type = doc.str("/type").unwrap_or_default();

    if msg_type != "MEDIA_STATUS" {
        log_info!("cast: media msg type={}", msg_type);
        return;
    }

    // Media status is at /status/0
    let player_state = doc.str("/status/0/playerState");
    let session_id = doc.i64("/status/0/mediaSessionId");
    let title = doc.str("/status/0/media/metadata/title");

    let duration = doc.f64("/status/0/media/duration");
    log_info!(
        "cast: MEDIA_STATUS session={} state={} dur={} title={}",
        session_id.unwrap_or(-1),
        player_state.as_deref().unwrap_or("?"),
        duration.map_or(0, |d| d as u32),
        title.as_deref().unwrap_or("(none)")
    );

    if let Some(session_id) = session_id {
        state.media_session_id = Some(session_id);
        state.status.media_session_id = session_id;
    }

    if let Some(player_state) = player_state {
        state.status.player_state = player_state;
    }

    if let Some(current_time) = doc.f64("/status/0/currentTime") {
        state.status.current_time = current_time;
    }

    // Duration from media info (only present in initial status, not polls)
    if let Some(dur) = duration {
        state.status.duration_secs = dur;
    }

    // Content type from metadata
    if let Some(mt) = doc.i64("/status/0/media/metadata/metadataType") {
        state.status.content_type = match mt {
            1 => ContentType::Movie,
            2 => ContentType::TvShow,
            3 => ContentType::Music,
            4 => ContentType::Photo,
            _ => ContentType::Generic,
        };
    }

    // Track metadata
    if let Some(title) = title {
        state.status.title = Some(title);
    }
    state.status.fields.clear();
    if let Some(artist) = doc
        .str("/status/0/media/metadata/artist")
        .or_else(|| doc.str("/status/0/media/metadata/albumArtist"))
        .or_else(|| doc.str("/status/0/media/metadata/subtitle"))
    {
        state.status.fields.push(("Artist".into(), artist));
    }
    if let Some(album) = doc.str("/status/0/media/metadata/albumName") {
        state.status.fields.push(("Album".into(), album));
    }

    // Album art — first image URL
    if let Some(url) = doc.str("/status/0/media/metadata/images/0/url") {
        log_info!("cast: art url={}", url);
        state.status.album_art_url = Some(url);
    }

    // Volume (also in media status sometimes)
    if let Some(level) = doc.f64("/status/0/volume/level") {
        state.status.volume_level = level as f32;
    }
    if let Some(muted) = doc.bool("/status/0/volume/muted") {
        state.status.volume_muted = muted;
    }

    // Capabilities: supportedMediaCommands bitmask + stream type
    if let Some(cmds) = doc.i64("/status/0/supportedMediaCommands") {
        state.status.supported_commands = cmds as u32;
    }
    if let Some(st) = doc.str("/status/0/media/streamType") {
        state.status.stream_type = Some(st);
    }

    (state.on_status)(&state.status);
    request_frame();
}

// ── Helpers ─────────────────────────────────────────────────────

/// Format an f32 as "X.YY" for JSON (ufmt doesn't support floats).
fn f32_json(val: f32) -> String {
    let cents = (val * 100.0).round() as u32;
    let whole = cents / 100;
    let frac = cents % 100;
    if frac < 10 {
        fmt!("{}.0{}", whole, frac)
    } else {
        fmt!("{}.{}", whole, frac)
    }
}

// ── Wire helpers ────────────────────────────────────────────────

/// Send a JSON message on the given namespace.
fn send_json(state: &CastState, namespace: &str, source: &str, destination: &str, json: &str) {
    let msg = CastMessage {
        protocol_version: ProtocolVersion::Castv210 as i32,
        source_id: source.into(),
        destination_id: destination.into(),
        namespace: namespace.into(),
        payload_type: PayloadType::String as i32,
        payload_utf8: Some(json.into()),
        payload_binary: None,
    };
    send_frame(state, &msg);
}

/// Encode a `CastMessage` and send it as a length-prefixed frame.
fn send_frame(state: &CastState, msg: &CastMessage) {
    let encoded = msg.encode_to_vec();
    let len = (encoded.len() as u32).to_be_bytes();
    let mut frame = Vec::with_capacity(4 + encoded.len());
    frame.extend_from_slice(&len);
    frame.extend_from_slice(&encoded);
    state.socket.write(&frame);
}

/// Build and send a media command (PLAY, PAUSE, SEEK, etc.) to the current transport.
fn with_media_command(command_type: &str, extra_fields: &str) {
    CAST.with(|c| {
        let mut borrow = c.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };
        let Some(tid) = state.transport_id.clone() else {
            log_warn!("cast: no transport for media command");
            return;
        };
        let session_id = state.media_session_id.unwrap_or(0);
        let req_id = state.alloc_request_id();
        log_info!(
            "cast: send {} session={} → {}",
            command_type,
            session_id,
            tid
        );
        let msg = json!({"type": #s(command_type), "mediaSessionId": #(session_id), "requestId": #(req_id)});
        // Append extra fields (e.g. ", \"currentTime\": 42") if present
        let msg = if extra_fields.is_empty() {
            msg
        } else {
            // Insert extra fields before the closing brace
            let mut s = msg;
            s.pop(); // remove trailing '}'
            s.push_str(extra_fields);
            s.push('}');
            s
        };
        send_json(state, NS_MEDIA, SENDER_ID, &tid, &msg);
    });
}

impl Phase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::AwaitingReceiverStatus => "AwaitingReceiverStatus",
            Self::MediaSession => "MediaSession",
            Self::Closed => "Closed",
            Self::Reconnecting => "Reconnecting",
        }
    }
}

impl CastState {
    fn alloc_request_id(&mut self) -> i64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }
}
