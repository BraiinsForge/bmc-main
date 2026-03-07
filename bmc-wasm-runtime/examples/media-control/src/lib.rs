// Copyright (C) 2026  Braiins Systems s.r.o.

//! Media Remote Control Widget — POC (BDK-334).
//!
//! Controls media playback on UPnP/DLNA, Google Cast, and Kodi devices over LAN.
//! Discovers devices via mDNS and presents a picker UI.

mod cast;
mod icons;
mod kodi;
mod protocol;
mod upnp;

use std::cell::{Cell, RefCell};

#[allow(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

use protocol::MediaController;
use upnp::{
    PositionInfo, TransportActions, TransportState, UpnpDevice, VolumeInfo, format_duration_hms,
    parse_mute, parse_position_info, parse_transport_actions, parse_transport_info, parse_volume,
};

// ── Configuration ────────────────────────────────────────────────

/// Which protocol backend is in use.
///
/// Variant order defines the display sort order within the same host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DiscoveredProtocol {
    Cast,
    Kodi,
    Upnp,
}

/// A device found via mDNS or SSDP discovery.
#[derive(Debug, Clone)]
struct DiscoveredDevice {
    /// Display name (from TXT records: Cast `fn`, UPnP service name).
    name: String,
    /// Resolved IP address.
    host: String,
    /// Service port.
    port: u16,
    /// Protocol type.
    protocol: DiscoveredProtocol,
    /// mDNS full service name or SSDP USN (unique key for deduplication).
    service_name: String,
    /// UPnP control path for AVTransport (from SSDP device description XML).
    av_transport_path: Option<String>,
    /// UPnP control path for RenderingControl (from SSDP device description XML).
    rendering_control_path: Option<String>,
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
    /// Available transport actions (from protocol capability query).
    actions: TransportActions,
    /// Album art bitmap ID (0 = none registered).
    art_bitmap_id: u16,
    /// Album art natural aspect ratio (width / height). 1.0 = square.
    art_aspect: f32,
    /// URL of the currently loaded album art (to avoid re-fetching).
    art_url: String,
    /// Accent background color extracted from album art (darkened average).
    accent_bg: u32,
    /// Whether the current media is video, music, etc.
    is_video: bool,
    /// Consecutive fetch failures for disconnect detection.
    consecutive_failures: u8,
    /// Whether this session has ever reached Connected state.
    was_ever_connected: bool,
}

impl Default for MediaState {
    fn default() -> Self {
        Self {
            transport: TransportState::NoMedia,
            position: PositionInfo::default(),
            volume: VolumeInfo::default(),
            actions: TransportActions::default(),
            art_bitmap_id: 0,
            art_aspect: 1.0,
            art_url: String::new(),
            accent_bg: GRAY_100,
            is_video: false,
            consecutive_failures: 0,
            was_ever_connected: false,
        }
    }
}

enum WidgetState {
    /// Browsing for devices — show device picker.
    Discovering,
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
    static STATE: RefCell<WidgetState> = const { RefCell::new(WidgetState::Discovering) };
    static DEVICE: RefCell<Option<UpnpDevice>> = const { RefCell::new(None) };
    /// Active protocol controller (set on connect, cleared on disconnect).
    static CONTROLLER: RefCell<Option<Box<dyn MediaController>>> = RefCell::new(None);
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

    // Start mDNS discovery for Cast, UPnP, and Kodi devices
    mdns::mdns_browse(
        &["_googlecast._tcp", "_upnp._tcp", "_xbmc-jsonrpc-h._tcp"],
        on_mdns_event,
    );
    log_info!("media: mDNS browse started");
    discovery_log("Browsing _googlecast._tcp".into());
    discovery_log("Browsing _upnp._tcp".into());
    discovery_log("Browsing _xbmc-jsonrpc-h._tcp".into());

    // Start SSDP discovery for native UPnP/DLNA renderers
    ssdp::ssdp_search(
        "urn:schemas-upnp-org:device:MediaRenderer:1",
        5,
        on_ssdp_event,
    );
    log_info!("media: SSDP search started");
    discovery_log("SSDP M-SEARCH MediaRenderer".into());

    request_frame();
}

/// Insert or update a discovered device, keeping the list sorted by host then protocol.
fn upsert_discovered(device: DiscoveredDevice) {
    DISCOVERED.with(|d| {
        let mut list = d.borrow_mut();
        if let Some(existing) = list
            .iter_mut()
            .find(|d| d.service_name == device.service_name)
        {
            *existing = device.clone();
        } else {
            list.push(device.clone());
        }
        list.sort_by(|a, b| a.host.cmp(&b.host).then(a.protocol.cmp(&b.protocol)));
    });

    // Auto-reconnect: if we're on the picker and this device matches the last
    // connected one, reconnect immediately (handles hot-reload gracefully).
    let is_discovering = STATE.with(|s| matches!(*s.borrow(), WidgetState::Discovering));
    if is_discovering {
        if let Some(last) = kv::get_string("last_device") {
            if last == device.service_name {
                log_info!("media: auto-reconnect to {}", device.name);
                connect_to_device(&device);
            }
        }
    }
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
        (DiscoveredProtocol::Cast, display)
    } else if service_type.contains("_xbmc-jsonrpc-h._tcp") {
        let display = name
            .strip_suffix("._xbmc-jsonrpc-h._tcp.local.")
            .unwrap_or(&name)
            .to_string();
        (DiscoveredProtocol::Kodi, display)
    } else if service_type.contains("_upnp._tcp") {
        // mDNS name is "Foo._upnp._tcp.local." — strip the suffix
        let display = name
            .strip_suffix("._upnp._tcp.local.")
            .unwrap_or(&name)
            .to_string();
        (DiscoveredProtocol::Upnp, display)
    } else {
        return;
    };

    let service_name = name;
    let proto_label = match protocol {
        DiscoveredProtocol::Cast => "Cast",
        DiscoveredProtocol::Kodi => "Kodi",
        DiscoveredProtocol::Upnp => "UPnP",
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
        service_name,
        av_transport_path: None,
        rendering_control_path: None,
    };

    upsert_discovered(device);

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

// ── SSDP discovery ──────────────────────────────────────────────

fn on_ssdp_event(_search: ssdp::SsdpSearch, event: &ssdp::SsdpEvent<'_>) {
    match event {
        ssdp::SsdpEvent::Found(json) => on_ssdp_found(json),
        ssdp::SsdpEvent::Removed(_usn) => { /* future: remove from list */ }
    }
}

fn on_ssdp_found(json: &str) {
    let doc = JsonDoc::parse(json.as_bytes());
    let name = doc.str("/name").unwrap_or_default();
    let host = doc.str("/host").unwrap_or_default();
    let port = doc.i64("/port").unwrap_or(0) as u16;
    let av_path = doc.str("/av_transport_path").unwrap_or_default();
    let rc_path = doc.str("/rendering_control_path").unwrap_or_default();

    if host.is_empty() || port == 0 {
        return;
    }

    let proto_label = "UPnP";
    log_info!(
        "media: SSDP found {} ({}) at {}:{}",
        name,
        proto_label,
        host,
        port
    );
    discovery_log(fmt!("{} (SSDP) at {}:{}", name, host, port));

    // Use host:port as a stable key for deduplication with mDNS-discovered devices
    let dedup_key = fmt!("ssdp:{}:{}", host, port);

    let device = DiscoveredDevice {
        name: name.to_string(),
        host: host.to_string(),
        port,
        protocol: DiscoveredProtocol::Upnp,
        service_name: dedup_key.clone(),
        av_transport_path: if av_path.is_empty() {
            None
        } else {
            Some(av_path.to_string())
        },
        rendering_control_path: if rc_path.is_empty() {
            None
        } else {
            Some(rc_path.to_string())
        },
    };

    // Deduplicate: skip if a device with the same host+port already exists
    // (may have been found via mDNS)
    let already_exists = DISCOVERED.with(|d| {
        d.borrow()
            .iter()
            .any(|existing| existing.host == host && existing.port == port)
    });
    if already_exists {
        return;
    }

    upsert_discovered(device);

    request_frame();
}

// ── Connection management ───────────────────────────────────────

fn connect_to_device(device: &DiscoveredDevice) {
    // Persist selection for auto-reconnect on hot reload
    kv::set("last_device", device.service_name.as_bytes());

    CONNECTED_DEVICE_NAME.with(|n| {
        device.name.clone_into(&mut *n.borrow_mut());
    });

    let controller: Box<dyn MediaController> = match device.protocol {
        DiscoveredProtocol::Cast => {
            cast::connect(&device.host, device.port, on_cast_status);
            Box::new(CastAdapter)
        }
        DiscoveredProtocol::Kodi => {
            kodi::connect(&device.host, device.port, on_kodi_status);
            // Grab auth headers after connect (KODI state now exists)
            Box::new(KodiAdapter {
                art_headers: kodi::auth_headers(),
            })
        }
        DiscoveredProtocol::Upnp => {
            let base_url = fmt!("http://{}:{}", device.host, device.port);
            let upnp_device = UpnpDevice {
                base_url,
                av_transport_path: device
                    .av_transport_path
                    .clone()
                    .unwrap_or_else(|| "/upnp/control/rendertransport1".into()),
                rendering_control_path: device
                    .rendering_control_path
                    .clone()
                    .unwrap_or_else(|| "/upnp/control/rendercontrol1".into()),
                name: device.name.clone(),
            };
            DEVICE.with(|d| *d.borrow_mut() = Some(upnp_device));
            // Kick off initial status poll — will transition to Connected on success
            with_device(|d| {
                upnp::get_position_info(d, on_position_info);
                upnp::get_transport_info(d, on_transport_info);
                upnp::get_transport_actions(d, on_transport_actions);
                upnp::get_volume(d, on_volume);
                upnp::get_mute(d, on_mute);
            });
            Box::new(UpnpAdapter)
        }
    };

    log_info!(
        "media: connecting to {} ({}) at {}:{}",
        device.name,
        controller.protocol_name(),
        device.host,
        device.port
    );

    CONTROLLER.with(|c| *c.borrow_mut() = Some(controller));
    STATE.with(|s| *s.borrow_mut() = WidgetState::Disconnected(MediaState::default()));
    request_frame();
}

fn disconnect_and_return_to_picker() {
    log_info!("media: disconnecting, returning to picker");
    with_controller(|c| c.disconnect());
    CONTROLLER.with(|c| *c.borrow_mut() = None);

    // Clear auto-reconnect target
    kv::delete("last_device");

    CONNECTED_DEVICE_NAME.with(|n| n.borrow_mut().clear());
    STATE.with(|s| *s.borrow_mut() = WidgetState::Discovering);
    request_frame();
}

/// Drive protocol timers, detect disconnect, and schedule next poll.
fn tick_protocol(delta_ms: u32) {
    let has_controller = CONTROLLER.with(|c| c.borrow().is_some());
    if !has_controller {
        return;
    }

    let mut alive = false;
    let mut playing_interval = 0u32;
    let mut idle_interval = 0u32;

    CONTROLLER.with(|c| {
        if let Some(ctrl) = c.borrow().as_deref() {
            ctrl.tick(delta_ms);
            alive = ctrl.is_alive();
            playing_interval = ctrl.poll_interval_playing();
            idle_interval = ctrl.poll_interval_idle();
        }
    });

    if !alive {
        transition_to_disconnected();
    }

    interpolate_position(delta_ms);

    if alive {
        let interval = if is_transport_playing() {
            playing_interval
        } else {
            idle_interval
        };
        request_frame_after(interval);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    tick_protocol(delta_ms);

    let size = SIZE.with(Cell::get);

    // Determine which screen we're on before building the tree
    #[derive(Clone, Copy)]
    enum Screen {
        Discovering,
        Media, // Connected or Disconnected
    }

    // Build tree inside STATE borrow, capture screen kind, then drop borrow
    let (result, screen) = STATE.with(|s| {
        let state = s.borrow();
        let (root, screen) = match &*state {
            WidgetState::Discovering => (render_discovering(size), Screen::Discovering),
            WidgetState::Connected(media) => (render_media_screen(size, media), Screen::Media),
            WidgetState::Disconnected(_) => (render_disconnected(size), Screen::Media),
        };
        (render_ui(size.width, size.height, root), screen)
    });

    // Handle clicks outside the STATE borrow so handlers can borrow_mut
    match screen {
        Screen::Discovering => {
            // Each button maps to a device in DISCOVERED (kept sorted on insert)
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
            let can_seek = STATE.with(|s| {
                let state = s.borrow();
                match &*state {
                    WidgetState::Connected(m) | WidgetState::Disconnected(m) => m.actions.can_seek,
                    WidgetState::Discovering => false,
                }
            });
            if can_seek {
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
                    with_controller(|c| c.set_volume(frac));
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
                        with_controller(|c| handle_media_click(c, media_idx));
                    }
                }
            }
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

/// Fetch album art through the active controller (handles protocol-specific auth).
fn fetch_album_art(url: &str) {
    let url = url.to_string();
    with_controller(|ctrl| ctrl.fetch_art(&url, on_album_art));
}

/// Clear album art and accent background on the current media state.
fn clear_album_art(media: &mut MediaState) {
    if media.art_bitmap_id > 0 {
        media.art_bitmap_id = 0;
        media.art_url.clear();
        media.accent_bg = GRAY_100;
    }
}

/// Dispatch through the active `MediaController`, if any.
fn with_controller(f: impl FnOnce(&dyn MediaController)) {
    CONTROLLER.with(|c| {
        if let Some(ctrl) = c.borrow().as_deref() {
            f(ctrl);
        }
    });
}

// ── Protocol adapters ───────────────────────────────────────────
//
// Thin structs implementing `MediaController` by delegating to module
// functions. Adapters live in lib.rs (not in protocol modules) because
// UPnP commands need response-callback functions defined here.

/// Google Cast adapter — zero-sized, all state in `cast::` thread-locals.
struct CastAdapter;

impl MediaController for CastAdapter {
    fn disconnect(&self) {
        cast::disconnect();
    }
    fn is_alive(&self) -> bool {
        cast::is_alive()
    }
    fn tick(&self, delta_ms: u32) {
        cast::tick(delta_ms);
    }
    fn play(&self) {
        cast::play();
    }
    fn pause(&self) {
        cast::pause();
    }
    fn next(&self) {
        cast::next();
    }
    fn previous(&self) {
        cast::previous();
    }
    fn seek(&self, position_secs: u32, _duration_secs: u32) {
        cast::seek(f64::from(position_secs));
    }
    fn set_volume(&self, level: f32) {
        cast::set_volume(level);
    }
    fn set_mute(&self, muted: bool) {
        cast::set_mute(muted);
    }
    fn poll_interval_playing(&self) -> u32 {
        POLL_INTERVAL_MS
    }
    fn poll_interval_idle(&self) -> u32 {
        cast::HEARTBEAT_MS
    }
    fn protocol_name(&self) -> &'static str {
        "Cast"
    }
}

/// Kodi JSON-RPC adapter — zero-sized, all state in `kodi::` thread-locals.
struct KodiAdapter {
    /// Cached auth headers for image fetches (avoids re-entrant KODI borrow).
    art_headers: Option<String>,
}

impl MediaController for KodiAdapter {
    fn disconnect(&self) {
        kodi::disconnect();
    }
    fn is_alive(&self) -> bool {
        kodi::is_alive()
    }
    fn tick(&self, delta_ms: u32) {
        kodi::tick(delta_ms);
    }
    fn play(&self) {
        kodi::play();
    }
    fn pause(&self) {
        kodi::pause();
    }
    fn next(&self) {
        kodi::next();
    }
    fn previous(&self) {
        kodi::previous();
    }
    fn seek(&self, position_secs: u32, duration_secs: u32) {
        if duration_secs == 0 {
            return;
        }
        let frac = f64::from(position_secs) / f64::from(duration_secs);
        kodi::seek(frac);
    }
    fn set_volume(&self, level: f32) {
        kodi::set_volume(level);
    }
    fn set_mute(&self, muted: bool) {
        kodi::set_mute(muted);
    }
    fn poll_interval_playing(&self) -> u32 {
        kodi::POLL_INTERVAL_MS
    }
    fn poll_interval_idle(&self) -> u32 {
        kodi::POLL_IDLE_INTERVAL_MS
    }
    fn protocol_name(&self) -> &'static str {
        "Kodi"
    }
    fn fetch_art(&self, url: &str, callback: fn(&FetchResponse)) {
        fetch(url, self.art_headers.as_deref(), callback);
    }
}

/// UPnP/DLNA adapter — zero-sized, device state in `DEVICE` thread-local.
/// Response callbacks (`on_command_response`, `on_volume_set`, `on_mute_set`)
/// are defined in lib.rs, which is why the adapter lives here.
struct UpnpAdapter;

impl MediaController for UpnpAdapter {
    fn disconnect(&self) {
        DEVICE.with(|d| *d.borrow_mut() = None);
    }
    fn is_alive(&self) -> bool {
        // UPnP manages alive state through record_failure()/reset_failures()
        true
    }
    fn tick(&self, _delta_ms: u32) {
        // UPnP polling is callback-chain driven, no tick needed
    }
    fn play(&self) {
        with_device(|d| upnp::play(d, on_command_response));
    }
    fn pause(&self) {
        with_device(|d| upnp::pause(d, on_command_response));
    }
    fn next(&self) {
        with_device(|d| upnp::next(d, on_command_response));
    }
    fn previous(&self) {
        with_device(|d| upnp::previous(d, on_command_response));
    }
    fn seek(&self, position_secs: u32, _duration_secs: u32) {
        with_device(|d| upnp::seek(d, position_secs, on_command_response));
    }
    fn set_volume(&self, level: f32) {
        let level_pct = (level * 100.0) as u32;
        with_device(|d| upnp::set_volume(d, level_pct.min(100), on_volume_set));
    }
    fn set_mute(&self, muted: bool) {
        with_device(|d| upnp::set_mute(d, muted, on_mute_set));
    }
    fn poll_interval_playing(&self) -> u32 {
        POLL_INTERVAL_MS
    }
    fn poll_interval_idle(&self) -> u32 {
        POLL_IDLE_INTERVAL_MS
    }
    fn protocol_name(&self) -> &'static str {
        "UPnP"
    }
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

// ── Unified command handlers ─────────────────────────────────────

/// Handle a media control button click (protocol-agnostic).
/// Button indices: 0=prev, 1=play/pause, 2=next, 3=vol-, 4=vol+, 5=mute.
fn handle_media_click(ctrl: &dyn MediaController, index: usize) {
    match index {
        0 => ctrl.previous(),
        1 => {
            if is_transport_playing() {
                ctrl.pause();
            } else {
                ctrl.play();
            }
        }
        2 => ctrl.next(),
        3 => adjust_volume_by_delta(ctrl, -0.05),
        4 => adjust_volume_by_delta(ctrl, 0.05),
        5 => {
            let muted = STATE.with(|s| {
                let state = s.borrow();
                match &*state {
                    WidgetState::Connected(m) | WidgetState::Disconnected(m) => m.volume.muted,
                    WidgetState::Discovering => false,
                }
            });
            ctrl.set_mute(!muted);
        }
        _ => {}
    }
}

/// Adjust volume by a fractional delta (0.0–1.0 scale).
fn adjust_volume_by_delta(ctrl: &dyn MediaController, delta: f32) {
    let current = STATE.with(|s| {
        let state = s.borrow();
        match &*state {
            WidgetState::Connected(m) | WidgetState::Disconnected(m) => {
                m.volume.level as f32 / 1_000.0
            }
            WidgetState::Discovering => 0.5,
        }
    });
    ctrl.set_volume((current + delta).clamp(0.0, 1.0));
}

/// Get the touch fraction for a named bar (drag takes priority, then release).
fn bar_frac(result: &TreeRenderResult, key: &str) -> Option<f32> {
    result
        .drag
        .get(key)
        .or_else(|| result.touch.get(key))
        .map(TouchHit::frac_x)
}

fn seek_to_fraction(frac: f32) {
    let dur = STATE.with(|s| {
        let state = s.borrow();
        match &*state {
            WidgetState::Connected(m) | WidgetState::Disconnected(m) => m.position.duration_secs,
            WidgetState::Discovering => 0,
        }
    });
    if dur == 0 {
        return;
    }

    let new_pos = (frac * dur as f32) as u32;
    with_controller(|c| c.seek(new_pos, dur));
}

// ── Response callbacks ───────────────────────────────────────────

/// Transition state from Disconnected → Connected on first successful UPnP response.
fn ensure_connected() {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        if matches!(
            &*state,
            WidgetState::Disconnected(_) | WidgetState::Discovering
        ) {
            let taken = std::mem::take(&mut *state);
            let mut m = match taken {
                WidgetState::Disconnected(m) => m,
                _ => MediaState::default(),
            };
            m.was_ever_connected = true;
            *state = WidgetState::Connected(m);
        }
    });
}

fn on_position_info(response: &FetchResponse) {
    if !response.ok() {
        log_info!("media: position_info FAILED");
        record_failure();
        // Keep polling so failures accumulate toward DISCONNECT_THRESHOLD
        schedule_position_poll();
        return;
    }

    reset_failures();
    ensure_connected();

    if let Some(info) = parse_position_info(response.body()) {
        // Check if album art changed
        let art_uri = info.track_meta.album_art_uri.clone();

        STATE.with(|s| {
            let mut state = s.borrow_mut();
            if let WidgetState::Connected(media) = &mut *state {
                media.position = info;
            }
        });

        // Fetch new album art if URI changed, clear if gone
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
                fetch_album_art(&uri);
            }
        } else {
            STATE.with(|s| {
                let mut state = s.borrow_mut();
                if let WidgetState::Connected(media) = &mut *state {
                    clear_album_art(media);
                }
            });
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
    ensure_connected();

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

fn on_transport_actions(response: &FetchResponse) {
    if !response.ok() {
        return;
    }

    reset_failures();
    ensure_connected();

    if let Some(actions) = parse_transport_actions(response.body()) {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            if let WidgetState::Connected(media) = &mut *state {
                media.actions = actions;
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
    ensure_connected();

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
    ensure_connected();

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
            // Get natural dimensions for aspect ratio (lightweight — no RGBA allocation)
            let aspect = host::image_dimensions(response.body())
                .map_or(1.0, |(w, h)| if h > 0 { w as f32 / h as f32 } else { 1.0 });

            // Sample full image average and darken for background tint
            let accent_bg = host::bitmap_sample(bitmap_id, 0, 0, u32::MAX, u32::MAX)
                .map_or(GRAY_100, |c| color!(c, lightness: 0.22, chroma: 0.06));

            STATE.with(|s| {
                let mut state = s.borrow_mut();
                if let WidgetState::Connected(media) = &mut *state {
                    media.art_bitmap_id = bitmap_id;
                    media.art_aspect = aspect;
                    media.accent_bg = accent_bg;
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
        upnp::get_transport_actions(d, on_transport_actions);
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
        // Transition Discovering/Disconnected → Connected on status
        if matches!(
            &*state,
            WidgetState::Disconnected(_) | WidgetState::Discovering
        ) {
            let taken = std::mem::take(&mut *state);
            let mut m = match taken {
                WidgetState::Disconnected(m) => m,
                _ => MediaState::default(),
            };
            m.was_ever_connected = true;
            *state = WidgetState::Connected(m);
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

            media.actions = status.transport_actions();

            // Fetch album art if URL changed, clear if gone
            if let Some(ref url) = status.album_art_url {
                let fetch_url = downscale_thumbnail_url(url);
                if media.art_url != fetch_url {
                    media.art_url.clone_from(&fetch_url);
                    fetch_album_art(&fetch_url);
                }
            } else {
                clear_album_art(media);
            }
        }
    });
}

// ── Kodi status callback ─────────────────────────────────────────

fn on_kodi_status(status: &kodi::KodiMediaStatus) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        // Transition Discovering/Disconnected → Connected on status
        if matches!(
            &*state,
            WidgetState::Disconnected(_) | WidgetState::Discovering
        ) {
            let taken = std::mem::take(&mut *state);
            let mut m = match taken {
                WidgetState::Disconnected(m) => m,
                _ => MediaState::default(),
            };
            m.was_ever_connected = true;
            *state = WidgetState::Connected(m);
        }
        if let WidgetState::Connected(media) = &mut *state {
            // Map Kodi player state to our TransportState
            media.transport = match status.player_state.as_str() {
                "playing" => TransportState::Playing,
                "paused" => TransportState::Paused,
                _ => TransportState::NoMedia,
            };

            media.position.position_secs = status.current_time as u32;
            media.position.duration_secs = status.duration_secs as u32;

            status
                .title
                .clone_into(&mut media.position.track_meta.title);
            status
                .artist
                .clone_into(&mut media.position.track_meta.artist);
            status
                .album
                .clone_into(&mut media.position.track_meta.album);

            // Volume: Kodi 0–100 → permille
            media.volume.level = (status.volume_level * 10.0) as u32;
            media.volume.muted = status.volume_muted;

            media.actions = TransportActions {
                can_play: true,
                can_pause: true,
                can_seek: status.can_seek,
                can_next: true,
                can_previous: true,
            };

            // Fetch album art if URL changed, clear if gone
            if let Some(ref url) = status.album_art_url {
                let fetch_url = downscale_thumbnail_url(url);
                if media.art_url != fetch_url {
                    media.art_url.clone_from(&fetch_url);
                    fetch_album_art(&fetch_url);
                }
            } else {
                clear_album_art(media);
            }
        }
    });
}

// ── Disconnect detection ────────────────────────────────────────

/// Record a fetch failure. After `DISCONNECT_THRESHOLD` consecutive failures,
/// transition to `Disconnected` and start reconnect polling.
fn record_failure() {
    let action = STATE.with(|s| {
        let mut state = s.borrow_mut();
        match &mut *state {
            WidgetState::Connected(media) => {
                media.consecutive_failures += 1;
                if media.consecutive_failures >= DISCONNECT_THRESHOLD {
                    // Transition Connected → Disconnected
                    let taken = std::mem::take(&mut *state);
                    if let WidgetState::Connected(m) = taken {
                        *state = WidgetState::Disconnected(m);
                    }
                    Some(true) // start reconnect
                } else {
                    None // keep polling normally
                }
            }
            WidgetState::Disconnected(media) => {
                media.consecutive_failures += 1;
                if media.consecutive_failures >= DISCONNECT_THRESHOLD {
                    Some(false) // already disconnected, just ensure reconnect timer
                } else {
                    None
                }
            }
            WidgetState::Discovering => None,
        }
    });

    if action.is_some() {
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
                m.was_ever_connected = true;
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
            upnp::get_transport_actions(d, on_transport_actions);
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
                    DiscoveredProtocol::Cast => &icons::PROTO_GOOGLE_CAST,
                    DiscoveredProtocol::Kodi => &icons::PROTO_KODI,
                    DiscoveredProtocol::Upnp => &icons::PROTO_DLNA,
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

// ── Disconnected screen ──────────────────────────────────────────

fn render_disconnected(size: WidgetSize) -> Node {
    let (device_name, was_connected) = STATE.with(|s| {
        let state = s.borrow();
        let was = matches!(&*state, WidgetState::Disconnected(m) if m.was_ever_connected);
        drop(state);
        let name = CONNECTED_DEVICE_NAME.with(|n| {
            let n = n.borrow();
            if n.is_empty() {
                "device".into()
            } else {
                n.clone()
            }
        });
        (name, was)
    });

    let subtitle = if was_connected {
        "Reconnecting..."
    } else {
        "Connecting..."
    };
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
                text(subtitle, style!(size: 14, color: GRAY_60)),
            ],
        )],
    )
}

// ── Media screen (connected) ─────────────────────────────────────

/// Wraps the media UI with a switcher button at the top (button index 0).
fn render_media_screen(size: WidgetSize, media: &MediaState) -> Node {
    match size.variant {
        SizeVariant::Full | SizeVariant::Medium => render_full(size, media),
        _ => render_compact(size, media),
    }
}

/// Full/Medium: art fills left side, everything else in right column.
fn render_full(size: WidgetSize, media: &MediaState) -> Node {
    let is_medium = size.variant == SizeVariant::Medium;
    let pad = if is_medium { 12.0 } else { 16.0 };
    let gap = if is_medium { 10.0 } else { 16.0 };
    let inner_gap = if is_medium { 6.0 } else { 12.0 };
    let art_size = size.height as f32 - 2.0 * pad;
    let col_w = size.width as f32 - 2.0 * pad - gap - art_size;

    row(
        props!(background: media.accent_bg, padding: pad, gap: gap),
        [
            render_album_art(media, art_size),
            col(
                props!(flex: 1.0, gap: inner_gap),
                [
                    render_track_info(media, size),
                    render_progress(media, col_w),
                    render_controls(media, col_w),
                ],
            ),
        ],
    )
}

/// Large/Small: art + meta row on top, full-width progress + controls below.
fn render_compact(size: WidgetSize, media: &MediaState) -> Node {
    let pad = if size.variant == SizeVariant::Large {
        16.0
    } else {
        8.0
    };
    let gap = 8.0;
    let avail_h = size.height as f32 - 2.0 * pad;
    let avail_w = size.width as f32 - 2.0 * pad;

    // Reserve space for progress bar + controls below the art row
    let controls_h = if size.variant == SizeVariant::Small {
        76.0 // stacked: 2×32 + 4 gap + 8 col gap
    } else {
        80.0 // progress (~30) + controls (~40) + gaps
    };
    let art_max_h = (avail_h - controls_h - gap).max(40.0);
    let art_size = art_max_h.min(avail_w * 0.4);

    col(
        props!(background: media.accent_bg, padding: pad, gap: gap),
        [
            // Top: art + track meta side by side (flex to push controls to bottom)
            row(
                props!(flex: 1.0, gap: gap),
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
                    [
                        render_progress(media, avail_w),
                        render_controls(media, avail_w),
                    ],
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
            props!(width: art_size, height: art_size, max_height: art_size),
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
            props!(width: art_size, height: art_size, max_height: art_size),
            vec![
                Draw::rect(0.0, 0.0, art_size, art_size, GRAY_80),
                Draw::centered(
                    Draw::icon(0.0, 0.0, icon_sz, icon_sz, placeholder_icon, GRAY_60)
                        .with_anti_alias(),
                ),
            ],
        )
    }
}

fn render_track_info(media: &MediaState, size: WidgetSize) -> Node {
    let has_track = media.position.track_meta.title.is_some();
    let title = media
        .position
        .track_meta
        .title
        .as_deref()
        .unwrap_or("No track");
    let artist = media.position.track_meta.artist.as_deref().unwrap_or("");
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
        props!(flex: 1.0, gap: 4.0),
        [
            text(title, style!(size: title_size, color: GRAY_10, weight: 600)),
            if !has_track || artist.is_empty() {
                spacer(0.0)
            } else {
                text(
                    &fmt!("Artist: {}", artist),
                    style!(size: detail_size, color: GRAY_40),
                )
            },
            if !has_track || album.is_empty() {
                spacer(0.0)
            } else {
                text(
                    &fmt!("Album: {}", album),
                    style!(size: detail_size, color: GRAY_50),
                )
            },
            // Push switcher button to the bottom-right (button index 0)
            spacer(1.0),
            row(props!(), [spacer(1.0), render_switcher_button(size)]),
            spacer(0.0), // breathing room before seek bar
        ],
    )
}

/// Interactive button showing protocol icon + connected device name.
/// Tapping disconnects and returns to the device picker.
/// Small variant uses icon-only to save horizontal space.
fn render_switcher_button(size: WidgetSize) -> Node {
    let icon = tree::ensure_registered(&icons::DEVICES);
    if size.variant == SizeVariant::Small {
        button!("", icon: icon, style: Secondary, size: Small)
    } else {
        let device_name = CONNECTED_DEVICE_NAME.with(|n| {
            let name = n.borrow();
            if name.is_empty() {
                "Unknown".into()
            } else {
                name.clone()
            }
        });
        button!(&device_name, icon: icon, style: Secondary, size: Small)
    }
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

    Draw::path(
        points,
        BAR_TRACK_H,
        color,
        false,
        false,
        Interpolation::CatmullRom,
    )
    .animate(
        AnimProperty::TranslateX,
        0.0,
        -WAVE_LENGTH,
        800,
        Easing::Linear,
        LoopMode::Forever,
    )
}

/// Track thickness for both seek and volume bars (pixels).
const BAR_TRACK_H: f32 = 2.0;
/// Sine wave amplitude — matches half the track thickness for visual consistency.
const WAVE_AMPLITUDE: f32 = BAR_TRACK_H / 2.0;
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

    Draw::path(
        points,
        BAR_TRACK_H,
        WHITE,
        false,
        false,
        Interpolation::CatmullRom,
    )
    .animate(
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

/// Touchable progress bar canvas.
///
/// Layout uses `flex: 1.0` so right edges align. Background track uses
/// `OVERSIZED_W` (clipped by canvas). Fill/dot positions use `draw_w`
/// (pre-computed expected width) so the progress fraction is correct.
/// Touch `frac_x` is relative to actual layout width, so seek works.
fn progress_bar_node(media: &MediaState, draw_w: f32) -> Node {
    let pos = media.position.position_secs;
    let dur = media.position.duration_secs;
    let is_playing = media.transport == TransportState::Playing;
    let is_continuous = dur == 0;
    let progress = if dur > 0 {
        (pos as f32 / dur as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let half_track = BAR_TRACK_H / 2.0;
    let bar_height = DOT_RADIUS * 2.0 + BAR_TRACK_H;
    let mid_y = bar_height / 2.0;
    let fill_w = draw_w * progress;

    let mut draws: Vec<Draw> = Vec::new();

    if is_continuous && is_playing {
        draws.push(squiggle_path(OVERSIZED_W, mid_y));
    } else {
        // Background track: oversized, clipped by canvas
        draws.push(Draw::rect(
            0.0,
            mid_y - half_track,
            OVERSIZED_W,
            BAR_TRACK_H,
            GRAY_70,
        ));

        if is_playing && fill_w > BAR_TRACK_H {
            draws.push(squiggle_path(fill_w, mid_y));
            draws.push(Draw::rect(
                fill_w,
                0.0,
                OVERSIZED_W - fill_w + 1.0,
                bar_height,
                media.accent_bg,
            ));
            let track_x = fill_w + DOT_RADIUS;
            draws.push(Draw::rect(
                track_x,
                mid_y - half_track,
                OVERSIZED_W - track_x,
                BAR_TRACK_H,
                GRAY_70,
            ));
        } else if fill_w > 0.0 {
            draws.push(Draw::rect(
                0.0,
                mid_y - half_track,
                fill_w,
                BAR_TRACK_H,
                WHITE,
            ));
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

/// Estimate text width at 12px font for time/percentage labels.
/// Digits ~7px, punctuation/spaces ~4px, plus small padding.
fn estimate_label_w(s: &str) -> f32 {
    let w: f32 = s
        .chars()
        .map(|c| if c.is_ascii_digit() { 7.2 } else { 4.0 })
        .sum();
    w + 4.0
}

fn render_progress(media: &MediaState, avail_w: f32) -> Node {
    let time_str = progress_time_str(media);
    let label_w = estimate_label_w(&time_str);
    let bar_draw_w = (avail_w - 8.0 - label_w).max(40.0);
    row(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            progress_bar_node(media, bar_draw_w),
            text(
                time_str,
                style!(size: 12, color: GRAY_40, text_overflow: TextOverflow::Clip),
            ),
        ],
    )
}

/// Ghost Small icon-only button width (square = height = 32px).
const BTN_SM: f32 = 32.0;

fn render_controls(media: &MediaState, avail_w: f32) -> Node {
    let cd = controls_data(media);
    let actions = &media.actions;

    let vol_label_w = estimate_label_w(&cd.vol_str);
    // 6 buttons + spacer(flex:1) + vol_bar + vol_label — 8 gaps
    let fixed = BTN_SM * 6.0 + vol_label_w + 8.0 * 8.0;
    let vol_draw_w = ((avail_w - fixed) / 2.0).max(20.0);

    row(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            button!("", icon: tree::ensure_registered(&icons::solid::SKIP_BACK), style: Ghost, size: Small, disabled: !actions.can_previous),
            button!("", icon: tree::ensure_registered(cd.play_icon), style: Ghost, size: Small, disabled: cd.play_disabled),
            button!("", icon: tree::ensure_registered(&icons::solid::SKIP_FORWARD), style: Ghost, size: Small, disabled: !actions.can_next),
            spacer(1.0),
            button!("", icon: tree::ensure_registered(&icons::solid::VOLUME_DOWN), style: Ghost, size: Small),
            button!("", icon: tree::ensure_registered(&icons::solid::VOLUME_UP), style: Ghost, size: Small),
            button!("", icon: tree::ensure_registered(cd.mute_icon), style: Ghost, size: Small),
            volume_bar(cd.vol_frac, vol_draw_w),
            text(&cd.vol_str, style!(size: 12, color: GRAY_40)),
        ],
    )
}

/// Stacked controls for small layouts:
/// Row 1: prev | play | next | [progress bar (flex)] | time (fixed)
/// Row 2: vol- | vol+ | mute | [volume bar (flex)]   | % (fixed)
fn render_controls_stacked(media: &MediaState, avail_w: f32) -> Node {
    let cd = controls_data(media);
    let actions = &media.actions;

    let time_str = progress_time_str(media);
    let time_label_w = estimate_label_w(&time_str);
    let vol_label_w = estimate_label_w(&cd.vol_str);

    // Row 1: 3 buttons + bar + time_label — 4 gaps
    let prog_draw_w = (avail_w - BTN_SM * 3.0 - time_label_w - 8.0 * 4.0).max(40.0);
    // Row 2: 3 buttons + bar + vol_label — 4 gaps
    let vol_draw_w = (avail_w - BTN_SM * 3.0 - vol_label_w - 8.0 * 4.0).max(20.0);

    col(
        props!(gap: 4.0),
        [
            row(
                props!(gap: 8.0, cross_align: CrossAlign::Center),
                [
                    button!("", icon: tree::ensure_registered(&icons::solid::SKIP_BACK), style: Ghost, size: Small, disabled: !actions.can_previous),
                    button!("", icon: tree::ensure_registered(cd.play_icon), style: Ghost, size: Small, disabled: cd.play_disabled),
                    button!("", icon: tree::ensure_registered(&icons::solid::SKIP_FORWARD), style: Ghost, size: Small, disabled: !actions.can_next),
                    progress_bar_node(media, prog_draw_w),
                    text(time_str, style!(size: 12, color: GRAY_40)),
                ],
            ),
            row(
                props!(gap: 8.0, cross_align: CrossAlign::Center),
                [
                    button!("", icon: tree::ensure_registered(&icons::solid::VOLUME_DOWN), style: Ghost, size: Small),
                    button!("", icon: tree::ensure_registered(&icons::solid::VOLUME_UP), style: Ghost, size: Small),
                    button!("", icon: tree::ensure_registered(cd.mute_icon), style: Ghost, size: Small),
                    volume_bar(cd.vol_frac, vol_draw_w),
                    text(&cd.vol_str, style!(size: 12, color: GRAY_40)),
                ],
            ),
        ],
    )
}

fn volume_bar(vol_frac: f32, draw_w: f32) -> Node {
    let fill_w = draw_w * vol_frac;
    let draws = vec![
        Draw::rect(0.0, 0.0, OVERSIZED_W, BAR_TRACK_H, GRAY_70),
        Draw::rect(0.0, 0.0, fill_w, BAR_TRACK_H, GRAY_30),
    ];
    touchable("volume", props!(flex: 1.0, height: BAR_TRACK_H), draws)
}

struct ControlsData {
    play_icon: &'static Icon,
    play_disabled: bool,
    mute_icon: &'static Icon,
    vol_str: String,
    vol_frac: f32,
}

fn controls_data(media: &MediaState) -> ControlsData {
    let is_playing = media.transport == TransportState::Playing;
    let play_icon = if is_playing {
        &icons::solid::PAUSE
    } else {
        &icons::solid::PLAY
    };
    let play_disabled = if is_playing {
        !media.actions.can_pause
    } else {
        !media.actions.can_play
    };
    let mute_icon = if media.volume.muted {
        &icons::solid::VOLUME_MUTE
    } else {
        &icons::solid::VOLUME_UP
    };
    let vol_pct = (media.volume.level + 5) / 10; // permille → percent, rounded
    let vol_str = fmt!("{}%", vol_pct);
    let vol_frac = media.volume.level as f32 / 1_000.0;
    ControlsData {
        play_icon,
        play_disabled,
        mute_icon,
        vol_str,
        vol_frac,
    }
}
