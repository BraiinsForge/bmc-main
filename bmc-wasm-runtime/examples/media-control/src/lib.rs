// Copyright (C) 2026  Braiins Systems s.r.o.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

//! Media Remote Control Widget — POC (BDK-334).
//!
//! Controls media playback on UPnP/DLNA, Google Cast, and DACP devices over LAN.
//! Discovers devices via mDNS and presents a picker UI.
//!
//! **Stage 5:** Device discovery + picker UI.

mod cast;
mod dacp;
mod dmap;
mod icons;
mod protocol;
mod upnp;

use std::cell::{Cell, RefCell};

#[allow(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

use upnp::{
    PositionInfo, TransportState, UpnpDevice, VolumeInfo, format_duration_hms, parse_mute,
    parse_position_info, parse_transport_info, parse_volume,
};

// ── Configuration ────────────────────────────────────────────────

/// Which protocol backend is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveProtocol {
    Upnp,
    Cast,
    Dacp,
}

/// A device found via mDNS discovery.
#[derive(Debug, Clone)]
struct DiscoveredDevice {
    /// Display name (from TXT records: Cast `fn`, DACP `Machine Name`/`CtlN`).
    name: String,
    /// Resolved IP address.
    host: String,
    /// Service port.
    port: u16,
    /// Protocol type.
    protocol: ActiveProtocol,
    /// mDNS full service name (unique key for deduplication).
    service_name: String,
}

/// How often to poll `GetPositionInfo` while playing (ms).
const POLL_INTERVAL_MS: u32 = 1_000;
/// How often to poll when paused/stopped (ms).
const POLL_IDLE_INTERVAL_MS: u32 = 3_000;

// ── State ────────────────────────────────────────────────────────

/// How many consecutive fetch failures before declaring disconnected.
const DISCONNECT_THRESHOLD: u8 = 3;
/// Reconnect poll interval (ms).
const RECONNECT_INTERVAL_MS: u32 = 5_000;

/// Widget-level state combining all UPnP status.
struct MediaState {
    transport: TransportState,
    position: PositionInfo,
    volume: VolumeInfo,
    /// Album art bitmap ID (0 = none registered).
    art_bitmap_id: u16,
    /// Album art natural aspect ratio (width / height). 1.0 = square.
    art_aspect: f32,
    /// URL of the currently loaded album art (to avoid re-fetching).
    art_url: String,
    /// Whether the current media is video, music, etc.
    is_video: bool,
    /// Consecutive fetch failures for disconnect detection.
    consecutive_failures: u8,
}

impl Default for MediaState {
    fn default() -> Self {
        Self {
            transport: TransportState::NoMedia,
            position: PositionInfo::default(),
            volume: VolumeInfo::default(),
            art_bitmap_id: 0,
            art_aspect: 1.0,
            art_url: String::new(),
            is_video: false,
            consecutive_failures: 0,
        }
    }
}

enum WidgetState {
    /// Browsing for devices — show device picker.
    Discovering,
    /// DACP pairing in progress — show PIN screen.
    Pairing {
        device: DiscoveredDevice,
        pin: String,
    },
    /// Connected to a device, polling state.
    Connected(MediaState),
    /// Device became unreachable after repeated failures.
    Disconnected(MediaState),
}

impl Default for WidgetState {
    fn default() -> Self {
        Self::Discovering
    }
}

thread_local! {
    static SIZE: Cell<WidgetSize> = const { Cell::new(WidgetSize {
        variant: SizeVariant::Full,
        width: 1_280,
        height: 480,
    }) };
    static STATE: RefCell<WidgetState> = RefCell::new(WidgetState::Discovering);
    static DEVICE: RefCell<Option<UpnpDevice>> = const { RefCell::new(None) };
    static PROTOCOL: Cell<ActiveProtocol> = const { Cell::new(ActiveProtocol::Upnp) };
    /// Devices found via mDNS (independent of widget state).
    static DISCOVERED: RefCell<Vec<DiscoveredDevice>> = const { RefCell::new(Vec::new()) };
    /// Display name of the currently connected device.
    static CONNECTED_DEVICE_NAME: RefCell<String> = const { RefCell::new(String::new()) };
    /// Rolling discovery log (newest first, max `DISCOVERY_LOG_MAX` entries).
    static DISCOVERY_LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Maximum lines shown in the discovery activity log.
const DISCOVERY_LOG_MAX: usize = 8;

/// Push a message to the on-screen discovery log (newest first, deduped).
fn discovery_log(msg: String) {
    DISCOVERY_LOG.with(|log| {
        let mut log = log.borrow_mut();
        // Skip duplicate of the most recent entry (e.g. multi-interface mDNS events)
        if log.first() == Some(&msg) {
            return;
        }
        log.insert(0, msg);
        log.truncate(DISCOVERY_LOG_MAX);
    });
}

// ── Entry points ─────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    SIZE.set(WidgetSize::from_dimensions(width, height));

    // Start mDNS discovery for Cast and DACP devices
    mdns::mdns_browse(
        &["_googlecast._tcp", "_touch-able._tcp", "_upnp._tcp"],
        on_mdns_event,
    );
    log_info!("media: mDNS browse started");
    discovery_log("Browsing _googlecast._tcp".into());
    discovery_log("Browsing _touch-able._tcp".into());
    discovery_log("Browsing _upnp._tcp".into());

    // Check for auto-reconnect target from last session
    if let Some(last) = kv::get_string("last_device") {
        log_info!("media: last device = {}", last);
    }

    request_frame();
}

// ── mDNS discovery ──────────────────────────────────────────────

fn on_mdns_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => on_mdns_found(json),
        mdns::MdnsEvent::Removed(name) => on_mdns_removed(name),
    }
}

fn on_mdns_found(json: &str) {
    let doc = JsonDoc::parse(json.as_bytes());

    let service_type = doc.str("/service_type").unwrap_or_default();
    let name = doc.str("/name").unwrap_or_default();
    let host = doc.str("/host").unwrap_or_default();
    let port = doc.i64("/port").unwrap_or(0) as u16;

    if host.is_empty() || port == 0 {
        return;
    }

    let (protocol, display_name) = if service_type.contains("_googlecast._tcp") {
        let display = doc.str("/txt/fn").unwrap_or_else(|| name.clone());
        (ActiveProtocol::Cast, display)
    } else if service_type.contains("_touch-able._tcp") {
        let display = doc
            .str("/txt/Machine Name")
            .or_else(|| doc.str("/txt/CtlN"))
            .unwrap_or_else(|| name.clone());
        (ActiveProtocol::Dacp, display)
    } else if service_type.contains("_upnp._tcp") {
        // mDNS name is "Foo._upnp._tcp.local." — strip the suffix
        let display = name
            .strip_suffix("._upnp._tcp.local.")
            .unwrap_or(&name)
            .to_string();
        (ActiveProtocol::Upnp, display)
    } else {
        return;
    };

    let service_name = name;
    let proto_label = match protocol {
        ActiveProtocol::Cast => "Cast",
        ActiveProtocol::Dacp => "DACP",
        ActiveProtocol::Upnp => "UPnP",
    };
    log_info!(
        "media: found {} ({}) at {}:{}",
        display_name,
        proto_label,
        host,
        port
    );
    discovery_log(fmt!(
        "{} ({}) at {}:{}",
        display_name,
        proto_label,
        host,
        port
    ));

    let device = DiscoveredDevice {
        name: display_name,
        host: host.to_string(),
        port,
        protocol,
        service_name: service_name.clone(),
    };

    // Deduplicate by service_name
    DISCOVERED.with(|d| {
        let mut list = d.borrow_mut();
        if let Some(existing) = list.iter_mut().find(|d| d.service_name == service_name) {
            *existing = device.clone();
        } else {
            list.push(device.clone());
        }
    });

    // Auto-connect if this matches the last-used device
    let auto_target = kv::get_string("last_device");
    if auto_target.as_deref() == Some(&service_name) {
        log_info!("media: auto-connecting to {}", service_name);
        connect_to_device(&device);
    }

    request_frame();
}

fn on_mdns_removed(name: &str) {
    log_info!("media: device removed: {}", name);
    discovery_log(fmt!("Removed {}", name));
    DISCOVERED.with(|d| {
        d.borrow_mut().retain(|dev| dev.service_name != name);
    });
    request_frame();
}

// ── Connection management ───────────────────────────────────────

fn connect_to_device(device: &DiscoveredDevice) {
    let proto_str = match device.protocol {
        ActiveProtocol::Upnp => "UPnP",
        ActiveProtocol::Cast => "Cast",
        ActiveProtocol::Dacp => "DACP",
    };
    log_info!(
        "media: connecting to {} ({}) at {}:{}",
        device.name,
        proto_str,
        device.host,
        device.port
    );
    // Persist selection for auto-reconnect
    kv::set("last_device", device.service_name.as_bytes());

    CONNECTED_DEVICE_NAME.with(|n| {
        *n.borrow_mut() = device.name.clone();
    });

    match device.protocol {
        ActiveProtocol::Cast => {
            PROTOCOL.set(ActiveProtocol::Cast);
            STATE.with(|s| *s.borrow_mut() = WidgetState::Disconnected(MediaState::default()));
            cast::connect(&device.host, device.port, on_cast_status);
        }
        ActiveProtocol::Dacp => {
            PROTOCOL.set(ActiveProtocol::Dacp);
            // Check for stored pairing GUID
            if let Some(guid) = kv::get_string("dacp_guid") {
                STATE.with(|s| *s.borrow_mut() = WidgetState::Disconnected(MediaState::default()));
                dacp::connect(&device.host, device.port, &guid, on_dacp_status);
            } else {
                // Need to pair first
                let pin = dacp::start_pairing(&device.host, device.port, on_dacp_status);
                STATE.with(|s| {
                    *s.borrow_mut() = WidgetState::Pairing {
                        device: device.clone(),
                        pin,
                    };
                });
            }
        }
        ActiveProtocol::Upnp => {
            PROTOCOL.set(ActiveProtocol::Upnp);
            let base_url = fmt!("http://{}:{}", device.host, device.port);
            let upnp_device = UpnpDevice {
                base_url,
                av_transport_path: "/upnp/control/rendertransport1".into(),
                rendering_control_path: "/upnp/control/rendercontrol1".into(),
                name: device.name.clone(),
            };
            DEVICE.with(|d| *d.borrow_mut() = Some(upnp_device));
            STATE.with(|s| {
                *s.borrow_mut() = WidgetState::Disconnected(MediaState::default());
            });
            // Kick off initial status poll — will transition to Connected on success
            with_device(|d| {
                upnp::get_position_info(d, on_position_info);
                upnp::get_transport_info(d, on_transport_info);
                upnp::get_volume(d, on_volume);
                upnp::get_mute(d, on_mute);
            });
        }
    }

    request_frame();
}

fn disconnect_and_return_to_picker() {
    log_info!("media: disconnecting, returning to picker");
    let proto = PROTOCOL.with(Cell::get);
    match proto {
        ActiveProtocol::Cast => cast::disconnect(),
        ActiveProtocol::Dacp => dacp::disconnect(),
        ActiveProtocol::Upnp => {
            DEVICE.with(|d| *d.borrow_mut() = None);
        }
    }

    // Clear auto-reconnect target
    kv::delete("last_device");

    CONNECTED_DEVICE_NAME.with(|n| n.borrow_mut().clear());
    STATE.with(|s| *s.borrow_mut() = WidgetState::Discovering);
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let proto = PROTOCOL.with(Cell::get);

    // Drive Cast heartbeat timing and detect disconnect
    if proto == ActiveProtocol::Cast {
        cast::tick(delta_ms);
        if !cast::is_alive() {
            transition_to_disconnected();
        }

        interpolate_position(delta_ms);
        if cast::is_alive() {
            let is_playing = is_transport_playing();
            let interval = if is_playing {
                POLL_INTERVAL_MS
            } else {
                cast::HEARTBEAT_MS
            };
            request_frame_after(interval);
        }
    }

    // Drive DACP reconnect timing and interpolate position
    if proto == ActiveProtocol::Dacp {
        dacp::tick(delta_ms);
        if !dacp::is_alive() {
            transition_to_disconnected();
        }

        interpolate_position(delta_ms);
        if dacp::is_connected() {
            let is_playing = is_transport_playing();
            request_frame_after(if is_playing { POLL_INTERVAL_MS } else { 3_000 });
        }
    }

    let size = SIZE.with(Cell::get);

    // Determine which screen we're on before building the tree
    #[derive(Clone, Copy)]
    enum Screen {
        Discovering,
        Pairing,
        Media, // Connected or Disconnected
    }

    // Build tree inside STATE borrow, capture screen kind, then drop borrow
    let (result, screen) = STATE.with(|s| {
        let state = s.borrow();
        let (root, screen) = match &*state {
            WidgetState::Discovering => (render_discovering(size), Screen::Discovering),
            WidgetState::Pairing { pin, .. } => (render_pairing(size, pin), Screen::Pairing),
            WidgetState::Connected(media) => (render_media_screen(size, media), Screen::Media),
            WidgetState::Disconnected(_) => (render_disconnected(size), Screen::Media),
        };
        (render_ui(size.width, size.height, root), screen)
    });

    // Handle clicks outside the STATE borrow so handlers can borrow_mut
    match screen {
        Screen::Discovering => {
            // Each button maps to a device in DISCOVERED
            for (i, &clicked) in result.clicks.iter().enumerate() {
                if clicked {
                    let device = DISCOVERED.with(|d| d.borrow().get(i).cloned());
                    if let Some(device) = device {
                        connect_to_device(&device);
                    }
                }
            }
        }
        Screen::Media => {
            // Button 0 = switcher (disconnect), rest = media controls (offset by 1)
            // Touch canvases: "progress", "volume"

            // Progress bar: drag for visual feedback, release to seek
            if let Some(frac) = bar_frac(&result, "progress") {
                STATE.with(|s| {
                    let mut state = s.borrow_mut();
                    if let WidgetState::Connected(media) = &mut *state {
                        if media.position.duration_secs > 0 {
                            media.position.position_secs =
                                (frac * media.position.duration_secs as f32) as u32;
                        }
                    }
                });
                if result.touch.contains_key("progress") {
                    seek_to_fraction(frac);
                }
                request_frame();
            }

            // Volume bar: drag for visual feedback, release to commit
            if let Some(frac) = bar_frac(&result, "volume") {
                STATE.with(|s| {
                    let mut state = s.borrow_mut();
                    if let WidgetState::Connected(media) | WidgetState::Disconnected(media) =
                        &mut *state
                    {
                        media.volume.level = (frac * 1_000.0) as u32;
                    }
                });
                if result.touch.contains_key("volume") {
                    match proto {
                        ActiveProtocol::Cast => cast::set_volume(frac),
                        ActiveProtocol::Dacp => dacp::set_volume((frac * 100.0) as u32),
                        ActiveProtocol::Upnp => {
                            let new_level = (frac * 100.0) as u32;
                            with_device(|device| {
                                upnp::set_volume(device, new_level, on_volume_set);
                            });
                        }
                    }
                }
                request_frame();
            }

            for (i, &clicked) in result.clicks.iter().enumerate() {
                if clicked {
                    if i == 0 {
                        // Switcher button — return to picker
                        disconnect_and_return_to_picker();
                    } else {
                        // Media controls (shifted by 1 for the switcher button)
                        let media_idx = i - 1;
                        match proto {
                            ActiveProtocol::Cast => handle_cast_click(media_idx),
                            ActiveProtocol::Dacp => handle_dacp_click(media_idx),
                            ActiveProtocol::Upnp => {
                                with_device(|device| match media_idx {
                                    0 => upnp::previous(device, on_command_response),
                                    1 => handle_play_pause(device),
                                    2 => upnp::next(device, on_command_response),
                                    3 => adjust_volume(device, -5),
                                    4 => adjust_volume(device, 5),
                                    5 => toggle_mute(device),
                                    _ => {}
                                });
                            }
                        }
                    }
                }
            }
        }
        Screen::Pairing => {
            // No interactive buttons on the pairing screen
        }
    }
}

// ── Device access helper ─────────────────────────────────────────

fn with_device(f: impl FnOnce(&UpnpDevice)) {
    DEVICE.with(|d| {
        if let Some(device) = d.borrow().as_ref() {
            f(device);
        }
    });
}

// ── Shared protocol helpers ──────────────────────────────────────

/// Transition Connected → Disconnected (protocol-agnostic).
fn transition_to_disconnected() {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        if matches!(&*state, WidgetState::Connected(_)) {
            let taken = std::mem::take(&mut *state);
            if let WidgetState::Connected(m) = taken {
                *state = WidgetState::Disconnected(m);
            }
        }
    });
}

/// Locally interpolate position while playing (smooth progress bar).
fn interpolate_position(delta_ms: u32) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        if let WidgetState::Connected(media) = &mut *state {
            if media.transport == TransportState::Playing && media.position.duration_secs > 0 {
                media.position.position_secs = (media.position.position_secs + delta_ms / 1_000)
                    .min(media.position.duration_secs);
            }
        }
    });
}

/// Check if the current transport state is Playing.
fn is_transport_playing() -> bool {
    STATE.with(|s| {
        let state = s.borrow();
        matches!(
            &*state,
            WidgetState::Connected(m) if m.transport == TransportState::Playing
        )
    })
}

// ── DACP command handlers ───────────────────────────────────────

fn handle_dacp_click(index: usize) {
    match index {
        0 => dacp::previous(),
        1 => dacp::play_pause(),
        2 => dacp::next(),
        3 => adjust_dacp_volume(-5),
        4 => adjust_dacp_volume(5),
        _ => {}
    }
}

fn adjust_dacp_volume(delta: i32) {
    let current = STATE.with(|s| {
        let state = s.borrow();
        match &*state {
            WidgetState::Connected(m) | WidgetState::Disconnected(m) => m.volume.level / 10,
            _ => 50,
        }
    });
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
    let new_level = (current as i32 + delta).clamp(0, 100) as u32;
    dacp::set_volume(new_level);
}

// ── Cast command handlers ────────────────────────────────────────

fn handle_cast_click(index: usize) {
    match index {
        0 => cast::previous(),
        1 => {
            let is_playing = STATE.with(|s| {
                let state = s.borrow();
                matches!(
                    &*state,
                    WidgetState::Connected(m) if m.transport == TransportState::Playing
                )
            });
            if is_playing {
                cast::pause();
            } else {
                cast::play();
            }
        }
        2 => cast::next(),
        3 => adjust_cast_volume(-0.05),
        4 => adjust_cast_volume(0.05),
        5 => {
            let muted = STATE.with(|s| {
                let state = s.borrow();
                match &*state {
                    WidgetState::Connected(m) | WidgetState::Disconnected(m) => m.volume.muted,
                    _ => false,
                }
            });
            cast::set_mute(!muted);
        }
        _ => {}
    }
}

fn adjust_cast_volume(delta: f32) {
    let current = STATE.with(|s| {
        let state = s.borrow();
        match &*state {
            WidgetState::Connected(m) | WidgetState::Disconnected(m) => {
                m.volume.level as f32 / 1_000.0
            }
            _ => 0.5,
        }
    });
    cast::set_volume((current + delta).clamp(0.0, 1.0));
}

// ── UPnP command handlers ───────────────────────────────────────

fn handle_play_pause(device: &UpnpDevice) {
    let is_playing = STATE.with(|s| {
        let state = s.borrow();
        matches!(
            &*state,
            WidgetState::Connected(m) | WidgetState::Disconnected(m)
                if m.transport == TransportState::Playing
        )
    });

    if is_playing {
        upnp::pause(device, on_command_response);
    } else {
        upnp::play(device, on_command_response);
    }
}

fn adjust_volume(device: &UpnpDevice, delta: i32) {
    let current_pct = STATE.with(|s| {
        let state = s.borrow();
        match &*state {
            WidgetState::Connected(m) | WidgetState::Disconnected(m) => m.volume.level / 10,
            _ => 50,
        }
    });
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
    let new_level = (current_pct as i32 + delta).clamp(0, 100) as u32;
    upnp::set_volume(device, new_level, on_volume_set);
}

/// Get the touch fraction for a named bar (drag takes priority, then release).
fn bar_frac(result: &TreeRenderResult, key: &str) -> Option<f32> {
    result
        .drag
        .get(key)
        .or_else(|| result.touch.get(key))
        .map(bmc_wasm_sdk::TouchHit::frac_x)
}

fn seek_to_fraction(frac: f32) {
    let dur = STATE.with(|s| {
        let state = s.borrow();
        match &*state {
            WidgetState::Connected(m) | WidgetState::Disconnected(m) => m.position.duration_secs,
            _ => 0,
        }
    });
    if dur == 0 {
        return;
    }

    let new_pos = (frac * dur as f32) as u32;
    match PROTOCOL.with(Cell::get) {
        ActiveProtocol::Cast => cast::seek(f64::from(new_pos)),
        ActiveProtocol::Dacp => dacp::seek(new_pos),
        ActiveProtocol::Upnp => {
            with_device(|device| upnp::seek(device, new_pos, on_command_response));
        }
    }
}

fn toggle_mute(device: &UpnpDevice) {
    let muted = STATE.with(|s| {
        let state = s.borrow();
        match &*state {
            WidgetState::Connected(m) | WidgetState::Disconnected(m) => m.volume.muted,
            _ => false,
        }
    });
    upnp::set_mute(device, !muted, on_mute_set);
}

// ── Response callbacks ───────────────────────────────────────────

fn on_position_info(response: &FetchResponse) {
    if !response.ok() {
        log_info!("media: position_info FAILED");
        record_failure();
        // Keep polling so failures accumulate toward DISCONNECT_THRESHOLD
        schedule_position_poll();
        return;
    }

    reset_failures();

    if let Some(info) = parse_position_info(response.body()) {
        // Check if album art changed
        let art_uri = info.track_meta.album_art_uri.clone();

        STATE.with(|s| {
            let mut state = s.borrow_mut();
            if let WidgetState::Connected(media) = &mut *state {
                media.position = info;
            }
        });

        // Fetch new album art if URI changed
        if let Some(uri) = art_uri {
            let should_fetch = STATE.with(|s| {
                let state = s.borrow();
                match &*state {
                    WidgetState::Connected(m) => m.art_url != uri,
                    _ => false,
                }
            });
            if should_fetch {
                STATE.with(|s| {
                    let mut state = s.borrow_mut();
                    if let WidgetState::Connected(media) = &mut *state {
                        media.art_url.clone_from(&uri);
                    }
                });
                fetch(&uri, None, on_album_art);
            }
        }

        request_frame();
    }

    // Schedule next poll
    schedule_position_poll();
}

fn on_transport_info(response: &FetchResponse) {
    if !response.ok() {
        log_info!("media: transport_info FAILED");
        record_failure();
        return;
    }

    reset_failures();

    if let Some(transport) = parse_transport_info(response.body()) {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            if let WidgetState::Connected(media) = &mut *state {
                media.transport = transport;
            }
        });
        request_frame();
    }
}

fn on_volume(response: &FetchResponse) {
    if !response.ok() {
        log_info!("media: volume FAILED");
        record_failure();
        return;
    }

    reset_failures();

    if let Some(level) = parse_volume(response.body()) {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            if let WidgetState::Connected(media) = &mut *state {
                media.volume.level = level * 10; // UPnP 0–100 → permille
            }
        });
        request_frame();
    }
}

fn on_mute(response: &FetchResponse) {
    if !response.ok() {
        log_info!("media: mute FAILED");
        record_failure();
        return;
    }

    reset_failures();

    if let Some(muted) = parse_mute(response.body()) {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            if let WidgetState::Connected(media) = &mut *state {
                media.volume.muted = muted;
            }
        });
        request_frame();
    }
}

fn on_album_art(response: &FetchResponse) {
    if response.ok() && !response.body().is_empty() {
        let bitmap_id = host::register_bitmap(response.body());
        if bitmap_id > 0 {
            // Get natural dimensions for aspect ratio
            let aspect = host::decode_image(response.body()).map_or(1.0, |(_, w, h)| {
                if h > 0 { w as f32 / h as f32 } else { 1.0 }
            });
            STATE.with(|s| {
                let mut state = s.borrow_mut();
                if let WidgetState::Connected(media) = &mut *state {
                    media.art_bitmap_id = bitmap_id;
                    media.art_aspect = aspect;
                }
            });
            request_frame();
        }
    }
}

/// After a command (play/pause/next/prev/stop), refresh state.
fn on_command_response(response: &FetchResponse) {
    if !response.ok() {
        record_failure();
        return;
    }
    reset_failures();
    with_device(|d| {
        upnp::get_position_info(d, on_position_info);
        upnp::get_transport_info(d, on_transport_info);
    });
}

/// After setting volume, re-fetch to confirm.
fn on_volume_set(response: &FetchResponse) {
    if !response.ok() {
        record_failure();
        return;
    }
    reset_failures();
    with_device(|d| upnp::get_volume(d, on_volume));
}

/// After toggling mute, re-fetch to confirm.
fn on_mute_set(response: &FetchResponse) {
    if !response.ok() {
        record_failure();
        return;
    }
    reset_failures();
    with_device(|d| upnp::get_mute(d, on_mute));
}

/// Rewrite known large thumbnail URLs to smaller variants.
/// YouTube: use `mqdefault.jpg` (320x180, 16:9, ~10KB) — no embedded letterboxing.
fn downscale_thumbnail_url(url: &str) -> String {
    if url.contains("i.ytimg.com") {
        url.replace("maxresdefault", "mqdefault")
            .replace("sddefault", "mqdefault")
            .replace("hqdefault", "mqdefault")
    } else {
        url.to_string()
    }
}

// ── Cast status callback ────────────────────────────────────────

fn on_cast_status(status: &cast::CastMediaStatus) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        // Transition Discovering/Pairing/Disconnected → Connected on status
        if matches!(
            &*state,
            WidgetState::Disconnected(_) | WidgetState::Discovering | WidgetState::Pairing { .. }
        ) {
            let taken = std::mem::take(&mut *state);
            match taken {
                WidgetState::Disconnected(m) => *state = WidgetState::Connected(m),
                _ => *state = WidgetState::Connected(MediaState::default()),
            }
        }
        if let WidgetState::Connected(media) = &mut *state {
            // Map Cast player state to our TransportState
            media.transport = match status.player_state.as_str() {
                "PLAYING" => TransportState::Playing,
                "PAUSED" => TransportState::Paused,
                "IDLE" | "BUFFERING" => TransportState::Stopped,
                _ => TransportState::NoMedia,
            };

            media.is_video = matches!(
                status.content_type,
                cast::ContentType::Movie | cast::ContentType::TvShow | cast::ContentType::Generic
            );

            media.position.position_secs = status.current_time as u32;
            media.position.duration_secs = status.duration_secs as u32;

            if let Some(ref title) = status.title {
                media.position.track_meta.title = Some(title.clone());
            }
            if let Some(ref artist) = status.artist {
                media.position.track_meta.artist = Some(artist.clone());
            }
            if let Some(ref album) = status.album {
                media.position.track_meta.album = Some(album.clone());
            }

            // Volume: Cast 0.0–1.0 → permille, UPnP 0–100 → ×10
            media.volume.level = (status.volume_level * 1_000.0) as u32;
            media.volume.muted = status.volume_muted;

            // Fetch album art if URL changed
            if let Some(ref url) = status.album_art_url {
                // Rewrite YouTube maxres thumbnails to medium quality (smaller file)
                let fetch_url = downscale_thumbnail_url(url);
                if media.art_url != fetch_url {
                    media.art_url.clone_from(&fetch_url);
                    fetch(&fetch_url, None, on_album_art);
                }
            }
        }
    });
}

// ── DACP status callback ────────────────────────────────────────

fn on_dacp_status(status: &dacp::DacpMediaStatus) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        // Transition Discovering/Pairing/Disconnected → Connected on status
        if matches!(
            &*state,
            WidgetState::Disconnected(_) | WidgetState::Discovering | WidgetState::Pairing { .. }
        ) {
            let taken = std::mem::take(&mut *state);
            match taken {
                WidgetState::Disconnected(m) => *state = WidgetState::Connected(m),
                _ => *state = WidgetState::Connected(MediaState::default()),
            }
        }
        if let WidgetState::Connected(media) = &mut *state {
            media.transport = match status.player_state {
                dacp::PlayerState::Playing => TransportState::Playing,
                dacp::PlayerState::Paused => TransportState::Paused,
                dacp::PlayerState::Stopped => TransportState::Stopped,
            };

            // DACP: duration_ms and remaining_ms → position in seconds
            media.position.duration_secs = status.duration_ms / 1_000;
            if status.duration_ms > 0 && status.remaining_ms <= status.duration_ms {
                media.position.position_secs = (status.duration_ms - status.remaining_ms) / 1_000;
            }

            if let Some(ref title) = status.track_name {
                media.position.track_meta.title = Some(title.clone());
            }
            if let Some(ref artist) = status.artist {
                media.position.track_meta.artist = Some(artist.clone());
            }
            if let Some(ref album) = status.album {
                media.position.track_meta.album = Some(album.clone());
            }

            // DACP volume 0–100 → permille
            media.volume.level = status.volume * 10;
            media.volume.muted = false; // DACP has no mute concept
        }
    });
}

// ── Disconnect detection ────────────────────────────────────────

/// Record a fetch failure. After `DISCONNECT_THRESHOLD` consecutive failures,
/// transition to `Disconnected` and start reconnect polling.
fn record_failure() {
    // Phase 1: increment counter, check threshold
    let should_disconnect = STATE.with(|s| {
        let mut state = s.borrow_mut();
        if let WidgetState::Connected(media) = &mut *state {
            media.consecutive_failures += 1;
            media.consecutive_failures >= DISCONNECT_THRESHOLD
        } else {
            false
        }
    });

    if should_disconnect {
        // Phase 2: transition to Disconnected (separate borrow)
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let taken = std::mem::take(&mut *state);
            if let WidgetState::Connected(m) = taken {
                *state = WidgetState::Disconnected(m);
            }
        });
        with_device(|d| {
            upnp::get_position_info_after(RECONNECT_INTERVAL_MS, d, on_reconnect);
        });
        request_frame();
    }
}

/// Reset failure counter on successful response. If we were disconnected,
/// transition back to connected and resume normal polling.
fn reset_failures() {
    let was_disconnected = STATE.with(|s| {
        let mut state = s.borrow_mut();
        if matches!(&*state, WidgetState::Disconnected(_)) {
            let taken = std::mem::take(&mut *state);
            if let WidgetState::Disconnected(mut m) = taken {
                m.consecutive_failures = 0;
                *state = WidgetState::Connected(m);
            }
            true
        } else {
            if let WidgetState::Connected(media) = &mut *state {
                media.consecutive_failures = 0;
            }
            false
        }
    });

    if was_disconnected {
        log_info!("media: reconnected (response OK after failures)");
        with_device(|d| {
            upnp::get_position_info(d, on_position_info);
            upnp::get_transport_info(d, on_transport_info);
            upnp::get_volume(d, on_volume);
            upnp::get_mute(d, on_mute);
        });
        request_frame();
    }
}

/// Reconnect probe — called on a timer while disconnected.
fn on_reconnect(response: &FetchResponse) {
    if response.ok() {
        reset_failures();
    } else {
        // Still disconnected, schedule another attempt
        with_device(|d| {
            upnp::get_position_info_after(RECONNECT_INTERVAL_MS, d, on_reconnect);
        });
    }
}

/// Schedule the next poll cycle based on transport state.
/// Polls both position and transport info to catch external state changes.
fn schedule_position_poll() {
    let interval = STATE.with(|s| {
        let state = s.borrow();
        match &*state {
            WidgetState::Connected(m) if m.transport == TransportState::Playing => {
                Some(POLL_INTERVAL_MS)
            }
            WidgetState::Disconnected(_) => None, // Don't poll while disconnected
            _ => Some(POLL_IDLE_INTERVAL_MS),
        }
    });
    if let Some(ms) = interval {
        with_device(|d| {
            upnp::get_position_info_after(ms, d, on_position_info);
            upnp::get_transport_info_after(ms, d, on_transport_info);
        });
    }
}

// ── UI rendering ─────────────────────────────────────────────────

// ── Discovery screen ─────────────────────────────────────────────

fn render_discovering(size: WidgetSize) -> Node {
    let devices: Vec<DiscoveredDevice> = DISCOVERED.with(|d| d.borrow().clone());

    let title_sz = match size.variant {
        SizeVariant::Full | SizeVariant::Large => 24,
        _ => 16,
    };

    if devices.is_empty() {
        // Empty state with animated loading indicator
        let icon_sz = 48.0;
        let loader_w: f32 = 80.0;
        let loader_h: f32 = 6.0;
        let mid_y = loader_h / 2.0;
        let mut children = vec![
            canvas(
                props!(width: icon_sz, height: icon_sz),
                vec![Draw::icon(
                    0.0,
                    0.0,
                    icon_sz,
                    icon_sz,
                    &icons::DEVICES_APPS,
                    GRAY_50,
                )],
            ),
            text(
                "Searching for devices...",
                style!(size: title_sz, color: GRAY_40),
            ),
            text(
                "Make sure devices are on the same network",
                style!(size: 14, color: GRAY_60),
            ),
            // Animated squiggle loader
            canvas(
                props!(width: loader_w, height: loader_h),
                vec![squiggle_loader(loader_w, mid_y, GRAY_50)],
            ),
        ];
        if !matches!(size.variant, SizeVariant::Small) {
            children.push(render_discovery_log(8, false));
        }
        center(
            props!(background: GRAY_100),
            [col(
                props!(gap: 12.0, cross_align: CrossAlign::Center),
                children,
            )],
        )
    } else {
        // Device list with log on the right (Full/Large/Medium) or hidden (Small)
        let pad = match size.variant {
            SizeVariant::Full | SizeVariant::Large => 24.0,
            _ => 16.0,
        };
        let buttons: Vec<Node> = devices
            .iter()
            .map(|dev| {
                let proto_icon = match dev.protocol {
                    ActiveProtocol::Cast => &icons::PROTO_GOOGLE_CAST,
                    ActiveProtocol::Dacp => &icons::PROTO_AIRPLAY,
                    ActiveProtocol::Upnp => &icons::PROTO_DLNA,
                };
                button!(
                    &dev.name,
                    icon: tree::ensure_registered(proto_icon),
                    style: Secondary,
                    size: Small
                )
            })
            .collect();

        let left = col(
            props!(gap: 8.0),
            [
                text(
                    "Select a device",
                    style!(size: title_sz, color: GRAY_20, weight: 600),
                ),
                col(props!(gap: 2.0), buttons),
            ],
        );

        match size.variant {
            SizeVariant::Small => col(props!(background: GRAY_100, padding: pad), [left]),
            _ => row(
                props!(background: GRAY_100, padding: pad, gap: 24.0),
                [left, render_discovery_log(DISCOVERY_LOG_MAX, true)],
            ),
        }
    }
}

/// Render the discovery activity log — newest entry at top, each line fading out.
fn render_discovery_log(max_lines: usize, right_aligned: bool) -> Node {
    let entries: Vec<String> = DISCOVERY_LOG.with(|log| log.borrow().clone());
    if entries.is_empty() {
        return spacer(0.0);
    }
    let visible = entries.len().min(max_lines);
    let lines: Vec<Node> = entries[..visible]
        .iter()
        .enumerate()
        .map(|(i, msg)| {
            let alpha = 1.0 - (i as f32 / max_lines as f32);
            text(
                msg.as_str(),
                style!(size: 9, color: color!(GRAY_70, alpha: alpha), line_height: 1.0),
            )
        })
        .collect();
    if right_aligned {
        col(
            props!(gap: 4.0, cross_align: CrossAlign::End, flex: 1.0),
            lines,
        )
    } else {
        col(props!(gap: 4.0, cross_align: CrossAlign::Center), lines)
    }
}

// ── Pairing screen ───────────────────────────────────────────────

fn render_pairing(size: WidgetSize, pin: &str) -> Node {
    let pin_sz = match size.variant {
        SizeVariant::Full | SizeVariant::Large => 72,
        SizeVariant::Medium => 48,
        SizeVariant::Small => 36,
    };

    center(
        props!(background: GRAY_100),
        [col(
            props!(gap: 16.0, cross_align: CrossAlign::Center),
            [
                text(pin, style!(size: pin_sz, color: WHITE, weight: 700)),
                text(
                    "Enter this code in iTunes/Music",
                    style!(size: 18, color: GRAY_40),
                ),
                text("Waiting for pairing...", style!(size: 14, color: GRAY_60)),
            ],
        )],
    )
}

// ── Disconnected screen ──────────────────────────────────────────

fn render_disconnected(size: WidgetSize) -> Node {
    let device_name = CONNECTED_DEVICE_NAME.with(|n| {
        let name = n.borrow();
        if name.is_empty() {
            "device".into()
        } else {
            name.clone()
        }
    });
    let msg = fmt!("Connecting to {}...", device_name);

    let text_size = match size.variant {
        SizeVariant::Full | SizeVariant::Large => 24,
        _ => 16,
    };

    center(
        props!(background: GRAY_100),
        [col(
            props!(gap: 8.0),
            [
                text(&msg, style!(size: text_size, color: GRAY_40)),
                text("Reconnecting...", style!(size: 14, color: GRAY_60)),
            ],
        )],
    )
}

// ── Media screen (connected) ─────────────────────────────────────

/// Wraps the media UI with a switcher button at the top (button index 0).
fn render_media_screen(size: WidgetSize, media: &MediaState) -> Node {
    match size.variant {
        SizeVariant::Full => render_full(size, media),
        _ => render_compact(size, media),
    }
}

/// Full: art fills left side, everything else in right column.
fn render_full(size: WidgetSize, media: &MediaState) -> Node {
    let pad = 16.0;
    let gap = 16.0;
    let art_size = size.height as f32 - 2.0 * pad;
    let bar_w = size.width as f32 - 2.0 * pad - gap - art_size;

    row(
        props!(background: GRAY_100, padding: pad, gap: gap),
        [
            render_album_art(media, art_size),
            col(
                props!(flex: 1.0, gap: 12.0),
                [
                    col(props!(flex: 1.0), [render_track_info(media, size)]),
                    render_progress(media, bar_w),
                    render_controls(media),
                ],
            ),
        ],
    )
}

/// Large/Medium/Small: art + meta row on top, full-width progress + controls below.
fn render_compact(size: WidgetSize, media: &MediaState) -> Node {
    let pad = match size.variant {
        SizeVariant::Large => 16.0,
        SizeVariant::Medium => 12.0,
        _ => 8.0,
    };
    let gap = 8.0;
    let avail_h = size.height as f32 - 2.0 * pad;
    let avail_w = size.width as f32 - 2.0 * pad;
    let bar_w = avail_w;

    // Reserve space for progress bar + controls below the art row
    let controls_h = 80.0; // progress (~30) + controls (~40) + gaps
    let art_max_h = (avail_h - controls_h).max(48.0);
    let art_size = match size.variant {
        SizeVariant::Large => art_max_h.min(avail_w * 0.4),
        SizeVariant::Medium => art_max_h.min(avail_w * 0.3),
        SizeVariant::Small => art_max_h.min(avail_h * 0.6),
        SizeVariant::Full => 48.0_f32.min(avail_h * 0.4),
    };

    col(
        props!(background: GRAY_100, padding: pad, gap: gap),
        [
            // Top: art + track meta side by side
            row(
                props!(gap: gap, height: art_size),
                [
                    render_album_art(media, art_size),
                    col(
                        props!(flex: 1.0, gap: 4.0),
                        [render_track_info(media, size)],
                    ),
                ],
            ),
            // Bottom: full-width progress + controls
            if size.variant == SizeVariant::Small {
                render_controls_stacked(media, avail_w)
            } else {
                col(
                    props!(gap: 8.0),
                    [render_progress(media, bar_w), render_controls(media)],
                )
            },
        ],
    )
}

fn render_album_art(media: &MediaState, art_size: f32) -> Node {
    if media.art_bitmap_id > 0 {
        // Contain: fit image inside art_size×art_size, center, no cropping
        let aspect = media.art_aspect;
        let (bw, bh, bx, by) = if aspect > 1.0 {
            // Wider than tall — fit width, center vertically
            let h = art_size / aspect;
            (art_size, h, 0.0, 0.0)
        } else if aspect < 1.0 {
            // Taller than wide — fit height, center horizontally
            let w = art_size * aspect;
            (w, art_size, (art_size - w) / 2.0, 0.0)
        } else {
            (art_size, art_size, 0.0, 0.0)
        };
        canvas(
            props!(width: art_size, height: art_size),
            vec![Draw::bitmap_id(bx, by, bw, bh, media.art_bitmap_id)],
        )
    } else {
        // Placeholder with music/video icon
        let icon_sz = (art_size * 0.4).min(64.0);
        let placeholder_icon = if media.is_video {
            &icons::VIDEO
        } else {
            &icons::MUSIC
        };
        canvas(
            props!(width: art_size, height: art_size),
            vec![
                Draw::rect(0.0, 0.0, art_size, art_size, GRAY_80),
                Draw::centered(Draw::icon(
                    0.0,
                    0.0,
                    icon_sz,
                    icon_sz,
                    placeholder_icon,
                    GRAY_60,
                )),
            ],
        )
    }
}

fn render_track_info(media: &MediaState, size: WidgetSize) -> Node {
    let title = media
        .position
        .track_meta
        .title
        .as_deref()
        .unwrap_or("No track");
    let artist = media
        .position
        .track_meta
        .artist
        .as_deref()
        .unwrap_or("Unknown artist");
    let album = media.position.track_meta.album.as_deref().unwrap_or("");

    let title_size = match size.variant {
        SizeVariant::Full | SizeVariant::Large => 28,
        SizeVariant::Medium => 18,
        SizeVariant::Small => 14,
    };
    let detail_size = match size.variant {
        SizeVariant::Full | SizeVariant::Large => 18,
        SizeVariant::Medium => 14,
        SizeVariant::Small => 12,
    };

    col(
        props!(gap: 4.0),
        [
            // Switcher button (index 0 in the click list)
            render_switcher_button(),
            text(title, style!(size: title_size, color: GRAY_10, weight: 600)),
            text(artist, style!(size: detail_size, color: GRAY_40)),
            if !album.is_empty() {
                text(album, style!(size: detail_size, color: GRAY_50))
            } else {
                spacer(0.0)
            },
        ],
    )
}

/// Interactive button showing protocol icon + connected device name.
/// Tapping disconnects and returns to the device picker.
fn render_switcher_button() -> Node {
    let device_name = CONNECTED_DEVICE_NAME.with(|n| {
        let name = n.borrow();
        if name.is_empty() {
            "Unknown".into()
        } else {
            name.clone()
        }
    });
    button!(&device_name, icon: tree::ensure_registered(&icons::DEVICES), style: Ghost, size: Small)
}

/// Animated sine-wave loader for the discovery screen.
fn squiggle_loader(width: f32, mid_y: f32, color: u32) -> Draw {
    let step = WAVE_LENGTH / WAVE_POINTS_PER_CYCLE as f32;
    let start_x = -WAVE_LENGTH;
    let end_x = width + WAVE_LENGTH;
    let n_points = ((end_x - start_x) / step) as usize + 1;

    let points: Vec<(f32, f32)> = (0..n_points)
        .map(|i| {
            let x = start_x + i as f32 * step;
            let phase = x / WAVE_LENGTH * std::f32::consts::TAU;
            (x, mid_y + phase.sin() * WAVE_AMPLITUDE)
        })
        .collect();

    Draw::path(points, 2.0, color, false, false, Interpolation::CatmullRom).animate(
        AnimProperty::TranslateX,
        0.0,
        -WAVE_LENGTH,
        800,
        Easing::Linear,
        LoopMode::Forever,
    )
}

/// Sine wave amplitude for the squiggly progress bar.
const WAVE_AMPLITUDE: f32 = 1.5;
/// Points per wave cycle (smoothed by Catmull-Rom on host).
const WAVE_POINTS_PER_CYCLE: usize = 8;
/// Wavelength in pixels.
const WAVE_LENGTH: f32 = 16.0;
/// Playhead dot radius.
const DOT_RADIUS: f32 = 4.0;

/// Animated sine-wave path that loops via `TranslateX`.
fn squiggle_path(end_x: f32, mid_y: f32) -> Draw {
    let step = WAVE_LENGTH / WAVE_POINTS_PER_CYCLE as f32;
    let start_x = -WAVE_LENGTH;
    let end_x = end_x + WAVE_LENGTH;
    let n_points = ((end_x - start_x) / step) as usize + 1;

    let points: Vec<(f32, f32)> = (0..n_points)
        .map(|i| {
            let x = start_x + i as f32 * step;
            let phase = x / WAVE_LENGTH * std::f32::consts::TAU;
            (x, mid_y + phase.sin() * WAVE_AMPLITUDE)
        })
        .collect();

    Draw::path(points, 2.0, WHITE, false, false, Interpolation::CatmullRom).animate(
        AnimProperty::TranslateX,
        0.0,
        -WAVE_LENGTH,
        800,
        Easing::Linear,
        LoopMode::Forever,
    )
}

/// Oversized draw width for background track (canvas clips to layout bounds).
const OVERSIZED_W: f32 = 1_500.0;

/// Touchable progress bar canvas (flex stretches, fills use `bar_w`).
///
/// Layout uses `flex: 1.0` so the bar stretches to fill available space.
/// Background draws use an oversized width (clipped by canvas). Fill position
/// and dot use `bar_w` (an approximation of the actual layout width) so the
/// progress fraction is visually correct.
fn progress_bar_node(media: &MediaState, bar_w: f32) -> Node {
    let pos = media.position.position_secs;
    let dur = media.position.duration_secs;
    let is_playing = media.transport == TransportState::Playing;
    let is_continuous = dur == 0;
    let progress = if dur > 0 {
        (pos as f32 / dur as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let bar_height = DOT_RADIUS * 2.0 + 2.0;
    let mid_y = bar_height / 2.0;
    let fill_w = bar_w * progress;

    let mut draws: Vec<Draw> = Vec::new();

    if is_continuous && is_playing {
        draws.push(squiggle_path(OVERSIZED_W, mid_y));
    } else {
        // Background track: oversized, clipped by canvas
        draws.push(Draw::rect(0.0, mid_y - 1.0, OVERSIZED_W, 2.0, GRAY_70));

        if is_playing && fill_w > 2.0 {
            draws.push(squiggle_path(fill_w, mid_y));
            draws.push(Draw::rect(
                fill_w,
                0.0,
                OVERSIZED_W - fill_w + 1.0,
                bar_height,
                GRAY_100,
            ));
            let track_x = fill_w + DOT_RADIUS;
            draws.push(Draw::rect(
                track_x,
                mid_y - 1.0,
                OVERSIZED_W - track_x,
                2.0,
                GRAY_70,
            ));
        } else if fill_w > 0.0 {
            draws.push(Draw::rect(0.0, mid_y - 1.0, fill_w, 2.0, WHITE));
        }

        if fill_w > 0.0 {
            draws.push(Draw::circle(fill_w, mid_y, DOT_RADIUS, WHITE));
        }
    }

    touchable("progress", props!(flex: 1.0, height: bar_height), draws)
}

/// Formatted time string for the progress bar.
fn progress_time_str(media: &MediaState) -> String {
    let pos = media.position.position_secs;
    let dur = media.position.duration_secs;
    if dur == 0 {
        format_duration_hms(pos)
    } else {
        fmt!(
            "{} / {}",
            format_duration_hms(pos),
            format_duration_hms(dur)
        )
    }
}

/// Approximate width of the progress time label ("xx:xx / xx:xx" at 12px).
const PROGRESS_TIME_APPROX_W: f32 = 90.0;

fn render_progress(media: &MediaState, bar_w: f32) -> Node {
    let progress_w = (bar_w - 8.0 - PROGRESS_TIME_APPROX_W).max(40.0);
    row(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            progress_bar_node(media, progress_w),
            text(
                progress_time_str(media),
                style!(size: 12, color: GRAY_40, text_overflow: TextOverflow::Clip),
            ),
        ],
    )
}

fn render_controls(media: &MediaState) -> Node {
    let (play_icon, mute_icon, vol_str, vol_frac) = controls_data(media);

    row(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            button!("", icon: tree::ensure_registered(&icons::solid::SKIP_BACK), style: Ghost, size: Small),
            button!("", icon: tree::ensure_registered(play_icon), style: Ghost, size: Small),
            button!("", icon: tree::ensure_registered(&icons::solid::SKIP_FORWARD), style: Ghost, size: Small),
            spacer(1.0),
            button!("", icon: tree::ensure_registered(&icons::solid::VOLUME_DOWN), style: Ghost, size: Small),
            button!("", icon: tree::ensure_registered(&icons::solid::VOLUME_UP), style: Ghost, size: Small),
            button!("", icon: tree::ensure_registered(mute_icon), style: Ghost, size: Small),
            volume_bar(vol_frac, VOLUME_BAR_W, false),
            row(
                props!(width: VOL_LABEL_W),
                [text(&vol_str, style!(size: 12, color: GRAY_40))],
            ),
        ],
    )
}

/// Ghost Small icon-only button width: 2×h_padding(12) + icon(14) = 38px.
const BTN_GHOST_SMALL_W: f32 = 38.0;

/// Stacked controls for small layouts:
/// Row 1: prev | play | next | [progress bar] | time
/// Row 2: vol- | vol+ | mute | [volume bar]   | %
fn render_controls_stacked(media: &MediaState, avail_w: f32) -> Node {
    let (play_icon, mute_icon, vol_str, vol_frac) = controls_data(media);

    let btns_3 = BTN_GHOST_SMALL_W * 3.0;
    // Row 1: 3 buttons + 4 gaps + progress + time
    let progress_w = (avail_w - btns_3 - 8.0 * 4.0 - PROGRESS_TIME_APPROX_W).max(40.0);
    // Row 2: 3 buttons + 3 gaps + volume + vol_label
    let volume_w = (avail_w - btns_3 - 8.0 * 3.0 - VOL_LABEL_W).max(40.0);

    col(
        props!(gap: 4.0),
        [
            row(
                props!(gap: 8.0, cross_align: CrossAlign::Center),
                [
                    button!("", icon: tree::ensure_registered(&icons::solid::SKIP_BACK), style: Ghost, size: Small),
                    button!("", icon: tree::ensure_registered(play_icon), style: Ghost, size: Small),
                    button!("", icon: tree::ensure_registered(&icons::solid::SKIP_FORWARD), style: Ghost, size: Small),
                    progress_bar_node(media, progress_w),
                    text(progress_time_str(media), style!(size: 12, color: GRAY_40)),
                ],
            ),
            row(
                props!(gap: 8.0, cross_align: CrossAlign::Center),
                [
                    button!("", icon: tree::ensure_registered(&icons::solid::VOLUME_DOWN), style: Ghost, size: Small),
                    button!("", icon: tree::ensure_registered(&icons::solid::VOLUME_UP), style: Ghost, size: Small),
                    button!("", icon: tree::ensure_registered(mute_icon), style: Ghost, size: Small),
                    volume_bar(vol_frac, volume_w, true),
                    row(
                        props!(width: VOL_LABEL_W),
                        [text(&vol_str, style!(size: 12, color: GRAY_40))],
                    ),
                ],
            ),
        ],
    )
}

/// Fixed width for the volume percentage label (fits "100%").
const VOL_LABEL_W: f32 = 36.0;
/// Fixed-width volume bar default width.
const VOLUME_BAR_W: f32 = 80.0;

fn volume_bar(vol_frac: f32, bar_w: f32, stretch: bool) -> Node {
    let h = 4.0;
    let bg_w = if stretch { OVERSIZED_W } else { bar_w };
    let fill_w = bar_w * vol_frac;
    let draws = vec![
        Draw::rect(0.0, 0.0, bg_w, h, GRAY_70),
        Draw::rect(0.0, 0.0, fill_w, h, GRAY_30),
    ];
    let bar_props = if stretch {
        props!(flex: 1.0, height: h)
    } else {
        props!(width: bar_w, height: h)
    };
    touchable("volume", bar_props, draws)
}

fn controls_data(media: &MediaState) -> (&'static Icon, &'static Icon, String, f32) {
    let play_icon = if media.transport == TransportState::Playing {
        &icons::solid::PAUSE
    } else {
        &icons::solid::PLAY
    };
    let mute_icon = if media.volume.muted {
        &icons::solid::VOLUME_MUTE
    } else {
        &icons::solid::VOLUME_UP
    };
    let vol_pct = (media.volume.level + 5) / 10; // permille → percent, rounded
    let vol_str = fmt!("{}%", vol_pct);
    let vol_frac = media.volume.level as f32 / 1_000.0;
    (play_icon, mute_icon, vol_str, vol_frac)
}
