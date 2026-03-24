// Copyright (C) 2026  Braiins Systems s.r.o.
#![allow(dead_code)]

//! DACP (Digital Audio Control Protocol) controller.
//!
//! Implements the iTunes/Music.app remote control protocol:
//! - Pairing via mDNS service registration + HTTP listener
//! - Session management (login with pairing GUID)
//! - Now-playing status (long-poll with revision tracking)
//! - Playback commands (play/pause, next, prev, seek, volume)
//!
//! DACP is HTTP-based with DMAP binary responses. All protocol logic
//! runs in WASM using the SDK's `FetchRequest` API.

use std::cell::RefCell;

use bmc_wasm_sdk::http_listener::{self, HttpListener, HttpRequest};
use bmc_wasm_sdk::mdns::{self, MdnsRegistration};
#[allow(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

use crate::dmap;

// ── Constants ───────────────────────────────────────────────────

/// DACP request headers (sent with every request).
const DACP_HEADERS: &str = "\
Client-DAAP-Version: 3.13\n\
User-Agent: Remote/1021\n\
Viewer-Only-Client: 1";

/// How often to retry login after failure (ms).
const LOGIN_RETRY_MS: u32 = 3_000;

/// Max consecutive failures before declaring disconnected.
const MAX_FAILURES: u8 = 3;

/// Artwork dimensions requested from the DACP server.
const ART_SIZE: u32 = 640;

// ── Public types ────────────────────────────────────────────────

/// Parsed now-playing status from a DACP server.
#[derive(Debug, Clone, Default)]
pub struct DacpMediaStatus {
    pub player_state: PlayerState,
    pub track_name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Total track duration in milliseconds.
    pub duration_ms: u32,
    /// Remaining time in milliseconds.
    pub remaining_ms: u32,
    /// Volume level (0-100).
    pub volume: u32,
}

/// DACP player state from `caps` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerState {
    #[default]
    Stopped,
    Paused,
    Playing,
}

/// Widget callback for status updates.
pub type StatusCallback = fn(&DacpMediaStatus);

// ── State machine ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Not connected.
    Idle,
    /// Pairing: mDNS registered, HTTP listener waiting for iTunes.
    Pairing,
    /// Sent /login, waiting for session ID.
    LoggingIn,
    /// Have session, actively polling.
    Active,
    /// Session lost, waiting to retry.
    Reconnecting,
}

/// Per-connection DACP state.
struct DacpState {
    phase: Phase,
    /// Base URL for requests (e.g. "http://192.168.1.50:3689").
    base_url: String,
    /// Pairing GUID (hex string, e.g. "0000000000000001").
    guid: String,
    /// Session ID from /login response.
    session_id: u32,
    /// Revision number for long-poll (playstatusupdate).
    revision: u32,
    /// Latest parsed status.
    status: DacpMediaStatus,
    /// Widget callback.
    on_status: StatusCallback,
    /// Track name at the time artwork was last fetched.
    last_art_track: String,
    /// Consecutive failures for disconnect detection.
    consecutive_failures: u8,
    /// Milliseconds accumulated for reconnect timing.
    ms_since_action: u32,
    // Pairing state
    pairing_listener: Option<HttpListener>,
    pairing_registration: Option<MdnsRegistration>,
}

thread_local! {
    static DACP: RefCell<Option<DacpState>> = const { RefCell::new(None) };
}

// ── Public API ──────────────────────────────────────────────────

/// Start the pairing flow. Returns a 4-digit PIN to display on the widget.
///
/// `host` and `port` identify the DACP server discovered via mDNS
/// (`_touch-able._tcp`). After pairing completes, automatically connects.
///
/// The flow:
/// 1. Start HTTP listener on ephemeral port
/// 2. Register mDNS `_touch-remote._tcp` service
/// 3. iTunes shows "Braiins Deck Remote" in its device list
/// 4. User clicks it, enters the PIN displayed on the widget
/// 5. iTunes connects to our HTTP listener → we respond with GUID
/// 6. GUID is persisted via `kv::set` for future sessions
/// 7. Auto-transitions to login phase
pub fn start_pairing(host: &str, port: u16, on_status: StatusCallback) -> String {
    // Restore or generate a pairing GUID
    let guid = kv::get_string("dacp_guid").unwrap_or_else(|| "0000000000000001".into());

    // Build base URL for post-pairing connection
    let base_url = fmt!("http://{}:{}", host, port);

    // Start HTTP listener on ephemeral port
    let listener = http_listener::http_listen(0, on_pairing_request);
    let listen_port = listener.port();
    log_info!("dacp: pairing listener on port {}", listen_port);

    // Register mDNS service as a remote control
    let registration = mdns::mdns_register(
        "_touch-remote._tcp",
        "Braiins Deck Remote",
        listen_port,
        &[
            ("DvNm", "Braiins Deck"),
            ("RemV", "10000"),
            ("DvTy", "iPod"),
            ("RemN", "Remote"),
            ("txtvers", "1"),
            ("Pair", &guid),
        ],
    );
    log_info!("dacp: mDNS registered, PIN=1234");

    DACP.with(|d| {
        *d.borrow_mut() = Some(DacpState {
            phase: Phase::Pairing,
            base_url,
            guid,
            session_id: 0,
            revision: 1,
            status: DacpMediaStatus::default(),
            on_status,
            last_art_track: String::new(),
            consecutive_failures: 0,
            ms_since_action: 0,
            pairing_listener: Some(listener),
            pairing_registration: Some(registration),
        });
    });

    // POC: fixed PIN. Production would generate random digits.
    "1234".into()
}

/// Connect to a DACP server with a known pairing GUID.
///
/// `host` and `port` identify the iTunes/Music.app DACP service
/// (typically discovered via mDNS `_touch-able._tcp`).
pub fn connect(host: &str, port: u16, guid: &str, on_status: StatusCallback) {
    let base_url = fmt!("http://{}:{}", host, port);
    log_info!("dacp: connecting to {}", base_url);

    DACP.with(|d| {
        *d.borrow_mut() = Some(DacpState {
            phase: Phase::LoggingIn,
            base_url: base_url.clone(),
            guid: guid.into(),
            session_id: 0,
            revision: 1,
            status: DacpMediaStatus::default(),
            on_status,
            last_art_track: String::new(),
            consecutive_failures: 0,
            ms_since_action: 0,
            pairing_listener: None,
            pairing_registration: None,
        });
    });

    send_login(&base_url, guid);
}

/// Disconnect from the DACP server and clean up resources.
pub fn disconnect() {
    DACP.with(|d| {
        if let Some(state) = d.borrow_mut().take() {
            if let Some(listener) = state.pairing_listener {
                listener.close();
            }
            if let Some(reg) = state.pairing_registration {
                reg.unregister();
            }
        }
    });
}

/// Whether we have an active DACP session.
pub fn is_connected() -> bool {
    DACP.with(|d| {
        d.borrow()
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Active)
    })
}

/// Whether the DACP connection is alive (not idle).
pub fn is_alive() -> bool {
    DACP.with(|d| {
        d.borrow()
            .as_ref()
            .is_some_and(|s| !matches!(s.phase, Phase::Idle))
    })
}

/// Toggle play/pause.
pub fn play_pause() {
    send_command("playpause");
}

/// Skip to next track.
pub fn next() {
    send_command("nextitem");
}

/// Skip to previous track.
pub fn previous() {
    send_command("previtem");
}

/// Seek to position in seconds.
pub fn seek(position_secs: u32) {
    let ms = position_secs * 1000;
    DACP.with(|d| {
        let borrow = d.borrow();
        let Some(state) = borrow.as_ref() else { return };
        if state.phase != Phase::Active {
            return;
        }
        let url = fmt!(
            "{}/ctrl-int/1/setproperty?dacp.playingtime={}&session-id={}",
            state.base_url,
            ms,
            state.session_id
        );
        FetchRequest::get(&url)
            .headers(DACP_HEADERS)
            .send(on_command_response);
    });
}

/// Set volume (0-100).
pub fn set_volume(level: u32) {
    let level = level.min(100);
    DACP.with(|d| {
        let borrow = d.borrow();
        let Some(state) = borrow.as_ref() else { return };
        if state.phase != Phase::Active {
            return;
        }
        let url = fmt!(
            "{}/ctrl-int/1/setproperty?dmcp.volume={}&session-id={}",
            state.base_url,
            level,
            state.session_id
        );
        FetchRequest::get(&url)
            .headers(DACP_HEADERS)
            .send(on_command_response);
    });
}

/// Called from render(delta_ms) to drive reconnect timing.
pub fn tick(delta_ms: u32) {
    DACP.with(|d| {
        let mut borrow = d.borrow_mut();
        let Some(state) = borrow.as_mut() else { return };

        if state.phase == Phase::Reconnecting {
            state.ms_since_action += delta_ms;
            if state.ms_since_action >= LOGIN_RETRY_MS {
                state.ms_since_action = 0;
                state.phase = Phase::LoggingIn;
                let url = fmt!("{}/login?pairing-guid=0x{}", state.base_url, state.guid);
                FetchRequest::get(&url)
                    .headers(DACP_HEADERS)
                    .send(on_login_response);
            }
        }
    });
}

// ── Pairing handler ─────────────────────────────────────────────

fn on_pairing_request(_listener: HttpListener, req: &HttpRequest) {
    log_info!("dacp: pairing request: {} {}", req.method, req.path);

    if !req.path.starts_with("/pair") {
        req.respond(404, "", b"");
        return;
    }

    // Build DMAP pairing response:
    //   cmpa (container)
    //     cmpg (u64) — pairing GUID
    //     cmnm (string) — remote name
    //     cmty (string) — device type
    let guid_bytes = DACP.with(|d| {
        let borrow = d.borrow();
        borrow.as_ref().map_or([0u8; 8], |s| hex_to_bytes(&s.guid))
    });
    let name = b"Braiins Deck";
    let type_str = b"iPod";

    let cmpg = dmap_node(*b"cmpg", &guid_bytes);
    let cmnm = dmap_node(*b"cmnm", name);
    let cmty = dmap_node(*b"cmty", type_str);

    let inner_len = cmpg.len() + cmnm.len() + cmty.len();
    let mut body = Vec::with_capacity(8 + inner_len);
    body.extend_from_slice(b"cmpa");
    body.extend_from_slice(&(inner_len as u32).to_be_bytes());
    body.extend_from_slice(&cmpg);
    body.extend_from_slice(&cmnm);
    body.extend_from_slice(&cmty);

    req.respond(200, "Content-Type: application/x-dmap-tagged", &body);
    log_info!("dacp: pairing response sent");

    // Persist GUID and auto-transition to login
    DACP.with(|d| {
        let mut borrow = d.borrow_mut();
        if let Some(state) = borrow.as_mut() {
            kv::set("dacp_guid", state.guid.as_bytes());

            // Tear down pairing infrastructure
            if let Some(listener) = state.pairing_listener.take() {
                listener.close();
            }
            if let Some(reg) = state.pairing_registration.take() {
                reg.unregister();
            }

            // Auto-connect: transition to LoggingIn
            if !state.base_url.is_empty() {
                log_info!("dacp: pairing complete, logging in to {}", state.base_url);
                state.phase = Phase::LoggingIn;
                let url = fmt!("{}/login?pairing-guid=0x{}", state.base_url, state.guid);
                FetchRequest::get(&url)
                    .headers(DACP_HEADERS)
                    .send(on_login_response);
            }
        }
    });
}

/// Build a single DMAP TLV node: `[4-byte tag][4-byte BE length][data]`.
fn dmap_node(tag: [u8; 4], data: &[u8]) -> Vec<u8> {
    let mut node = Vec::with_capacity(8 + data.len());
    node.extend_from_slice(&tag);
    node.extend_from_slice(&(data.len() as u32).to_be_bytes());
    node.extend_from_slice(data);
    node
}

/// Parse a hex string into 8 bytes (truncated/zero-padded).
fn hex_to_bytes(hex: &str) -> [u8; 8] {
    let hex = hex.as_bytes();
    let mut bytes = [0u8; 8];
    for (i, byte) in bytes.iter_mut().enumerate() {
        let hi = hex.get(i * 2).copied().map_or(0, hex_nibble);
        let lo = hex.get(i * 2 + 1).copied().map_or(0, hex_nibble);
        *byte = (hi << 4) | lo;
    }
    bytes
}

fn hex_nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

// ── Response callbacks ──────────────────────────────────────────

fn send_login(base_url: &str, guid: &str) {
    let url = fmt!("{}/login?pairing-guid=0x{}", base_url, guid);
    FetchRequest::get(&url)
        .headers(DACP_HEADERS)
        .send(on_login_response);
}

fn on_login_response(response: &FetchResponse) {
    if !response.ok() {
        log_warn!("dacp: login failed (status {})", response.status);
        DACP.with(|d| {
            let mut borrow = d.borrow_mut();
            if let Some(state) = borrow.as_mut() {
                state.phase = Phase::Reconnecting;
                state.ms_since_action = 0;
            }
        });
        request_frame_after(LOGIN_RETRY_MS);
        return;
    }

    let nodes = dmap::parse(response.body());
    let session_id = dmap::find_u32(&nodes, *b"mlid").unwrap_or(0);

    if session_id == 0 {
        log_warn!("dacp: login response missing session ID");
        DACP.with(|d| {
            let mut borrow = d.borrow_mut();
            if let Some(state) = borrow.as_mut() {
                state.phase = Phase::Reconnecting;
                state.ms_since_action = 0;
            }
        });
        request_frame_after(LOGIN_RETRY_MS);
        return;
    }

    log_info!("dacp: logged in, session_id={}", session_id);

    DACP.with(|d| {
        let mut borrow = d.borrow_mut();
        if let Some(state) = borrow.as_mut() {
            state.session_id = session_id;
            state.phase = Phase::Active;
            state.consecutive_failures = 0;
            state.revision = 1;
        }
    });

    // Start status long-poll
    poll_status();
    request_frame();
}

fn on_status_update(response: &FetchResponse) {
    if !response.ok() {
        log_warn!("dacp: status poll failed");
        let should_reconnect = DACP.with(|d| {
            let mut borrow = d.borrow_mut();
            let Some(state) = borrow.as_mut() else {
                return false;
            };
            state.consecutive_failures += 1;
            if state.consecutive_failures >= MAX_FAILURES {
                state.phase = Phase::Reconnecting;
                state.ms_since_action = 0;
                true
            } else {
                false
            }
        });
        if should_reconnect {
            log_warn!("dacp: too many failures, reconnecting");
            request_frame_after(LOGIN_RETRY_MS);
        } else {
            poll_status_after(1_000);
        }
        return;
    }

    let nodes = dmap::parse(response.body());

    let player_state = match dmap::find_u8(&nodes, *b"caps") {
        Some(4) => PlayerState::Playing,
        Some(3) => PlayerState::Paused,
        _ => PlayerState::Stopped,
    };
    let track_name = dmap::find_str(&nodes, *b"cann").map(String::from);
    let artist = dmap::find_str(&nodes, *b"cana").map(String::from);
    let album = dmap::find_str(&nodes, *b"canl").map(String::from);
    let duration_ms = dmap::find_u32(&nodes, *b"cast").unwrap_or(0);
    let remaining_ms = dmap::find_u32(&nodes, *b"cant").unwrap_or(0);
    let volume = dmap::find_u32(&nodes, *b"cmvo").unwrap_or(0);
    let revision = dmap::find_u32(&nodes, *b"cmsr").unwrap_or(1);

    let new_status = DacpMediaStatus {
        player_state,
        track_name,
        artist,
        album,
        duration_ms,
        remaining_ms,
        volume,
    };

    let should_fetch_art = DACP.with(|d| {
        let mut borrow = d.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };

        let current_track = new_status.track_name.as_deref().unwrap_or("");
        let track_changed = state.last_art_track != current_track;
        if track_changed {
            state.last_art_track = current_track.into();
        }

        state.status = new_status;
        state.consecutive_failures = 0;
        state.revision = revision + 1;

        (state.on_status)(&state.status);

        track_changed && duration_ms > 0
    });

    if should_fetch_art {
        fetch_artwork();
    }

    // Continue long-polling (server blocks until next change)
    poll_status();
    request_frame();
}

fn on_artwork_response(response: &FetchResponse) {
    if response.ok() && !response.body().is_empty() {
        let bitmap_id = host::register_bitmap(response.body());
        if bitmap_id > 0 {
            log_info!("dacp: artwork registered as bitmap {}", bitmap_id);
        }
    }
}

fn on_command_response(_response: &FetchResponse) {
    // DACP commands return minimal DMAP. The long-poll picks up changes.
}

// ── Internal helpers ────────────────────────────────────────────

fn send_command(command: &str) {
    DACP.with(|d| {
        let borrow = d.borrow();
        let Some(state) = borrow.as_ref() else { return };
        if state.phase != Phase::Active {
            return;
        }
        let url = fmt!(
            "{}/ctrl-int/1/{}?session-id={}",
            state.base_url,
            command,
            state.session_id
        );
        FetchRequest::get(&url)
            .headers(DACP_HEADERS)
            .send(on_command_response);
    });
}

fn poll_status() {
    DACP.with(|d| {
        let borrow = d.borrow();
        let Some(state) = borrow.as_ref() else { return };
        if state.phase != Phase::Active {
            return;
        }
        let url = fmt!(
            "{}/ctrl-int/1/playstatusupdate?revision-number={}&session-id={}",
            state.base_url,
            state.revision,
            state.session_id
        );
        FetchRequest::get(&url)
            .headers(DACP_HEADERS)
            .send(on_status_update);
    });
}

fn poll_status_after(delay_ms: u32) {
    DACP.with(|d| {
        let borrow = d.borrow();
        let Some(state) = borrow.as_ref() else { return };
        if state.phase != Phase::Active {
            return;
        }
        let url = fmt!(
            "{}/ctrl-int/1/playstatusupdate?revision-number={}&session-id={}",
            state.base_url,
            state.revision,
            state.session_id
        );
        FetchRequest::get(&url)
            .headers(DACP_HEADERS)
            .send_after(delay_ms, on_status_update);
    });
}

fn fetch_artwork() {
    DACP.with(|d| {
        let borrow = d.borrow();
        let Some(state) = borrow.as_ref() else { return };
        if state.phase != Phase::Active {
            return;
        }
        let url = fmt!(
            "{}/ctrl-int/1/nowplayingartwork?mw={}&mh={}&session-id={}",
            state.base_url,
            ART_SIZE,
            ART_SIZE,
            state.session_id
        );
        FetchRequest::get(&url)
            .headers(DACP_HEADERS)
            .send(on_artwork_response);
    });
}
