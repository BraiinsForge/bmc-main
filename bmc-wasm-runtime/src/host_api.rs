// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host state and function bindings for WASM.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

use serde_json::Value;
use taffy::prelude::*;

use bmc_wasm_protocol::FormatPreferences;

use crate::gpu::FemtoVgRenderer;
use crate::interaction::InteractionState;
use crate::tree::NodeContext;

/// State for a single running animation instance.
#[derive(Debug, Clone)]
pub struct AnimationState {
    pub elapsed_ms: u32,
    /// Frame counter when last seen (for GC).
    pub last_seen_frame: u64,
}

/// Captured static values of a draw command for transition interpolation.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PrevDrawValues {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: u32,
    /// Orbit angle (for `Orbit` draw commands).
    pub angle: f32,
    pub radius: f32,
    /// Rotation angle (for `Rotated` draw commands).
    pub rotation: f32,
    /// Sphere camera center latitude (degrees).
    pub center_lat: f32,
    /// Sphere camera center longitude (degrees).
    pub center_lon: f32,
    /// Sphere camera distance (unitless, in sphere radii).
    pub zoom: f32,
    /// Light direction latitude (degrees).
    pub light_lat: f32,
    /// Light direction longitude (degrees).
    pub light_lon: f32,
}

/// State for a single transition instance.
#[derive(Debug, Clone)]
pub struct TransitionState {
    /// Values we are interpolating from.
    pub from: PrevDrawValues,
    /// Previous target values (to detect changes).
    pub target: PrevDrawValues,
    pub elapsed_ms: u32,
    pub last_seen_frame: u64,
}

/// State for a modal dialog (animation, scroll)
#[derive(Debug, Default)]
pub struct ModalState {
    /// Current open state (tracked for transition detection)
    pub is_open: bool,
    /// Animation progress: 0.0 = closed, 1.0 = fully open
    pub animation_progress: f32,
    /// Current scroll offset in the modal body
    pub scroll_offset: f32,
    /// Total content height (for scroll bounds)
    pub content_height: f32,
    /// Viewport height (for scroll bounds)
    pub viewport_height: f32,
    /// Whether currently dragging to scroll
    pub is_dragging: bool,
}

/// State for a scroll container
#[derive(Debug, Default)]
pub struct ScrollState {
    /// Current scroll offset (pixels from top)
    pub scroll_offset: f32,
}

/// A completed HTTP fetch response ready for delivery to WASM.
pub struct CompletedFetch {
    pub request_id: u32,
    pub status: u32,
    pub body: Vec<u8>,
}

/// A delayed fetch waiting for its fire time.
pub struct DelayedFetch {
    pub fire_at: Instant,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub request_id: u32,
}

/// A WebSocket event queued for delivery to WASM.
pub enum WsEvent {
    /// Connection successfully opened.
    Open,
    /// A text message was received.
    Message(Vec<u8>),
    /// Connection closed with a status code.
    Close(u16),
}

/// An active WebSocket connection managed by a background thread.
pub struct ActiveWebSocket {
    /// Channel to send outbound messages to the background write loop.
    pub msg_tx: mpsc::Sender<WsOutbound>,
    /// Channel to receive inbound events from the background read loop.
    pub event_rx: mpsc::Receiver<WsEvent>,
}

/// Outbound message or control signal for a WebSocket background thread.
pub enum WsOutbound {
    /// Send a text message.
    Text(String),
    /// Close the connection.
    Close,
}

/// A TLS socket event queued for delivery to WASM.
pub enum SocketEvent {
    /// Connection successfully established.
    Connected,
    /// Data received from the remote end.
    Data(Vec<u8>),
    /// Connection closed (0 = normal, non-zero = error).
    Closed(u32),
}

/// An active TLS socket connection managed by a background thread.
pub struct ActiveSocket {
    /// Channel to send outbound data to the background write loop.
    pub write_tx: mpsc::Sender<SocketOutbound>,
    /// Channel to receive inbound events from the background read loop.
    pub event_rx: mpsc::Receiver<SocketEvent>,
}

/// Outbound data or control signal for a socket background thread.
pub enum SocketOutbound {
    /// Write data bytes.
    Data(Vec<u8>),
    /// Close the connection.
    Close,
}

/// An mDNS event queued for delivery to WASM.
pub enum MdnsEvent {
    /// Service found/resolved — carries JSON with service details.
    Found(String),
    /// Service removed — carries the service full name.
    Removed(String),
}

/// An active mDNS browse session managed by a background thread.
pub struct ActiveMdnsBrowse {
    /// Channel to receive mDNS events from the background thread.
    pub event_rx: mpsc::Receiver<MdnsEvent>,
    /// Signal the background thread to stop.
    pub stop_tx: mpsc::Sender<()>,
}

/// An SSDP event queued for delivery to WASM.
pub enum SsdpEvent {
    /// Device found — carries JSON with device details.
    Found(String),
    /// Device removed — carries the USN string (from SSDP NOTIFY byebye).
    Removed(String),
}

/// An active SSDP search session managed by a background thread.
pub struct ActiveSsdpSearch {
    /// Channel to receive SSDP events from the background thread.
    pub event_rx: mpsc::Receiver<SsdpEvent>,
    /// Signal the background thread to stop.
    pub stop_tx: mpsc::Sender<()>,
}

/// A UDP broadcast event queued for delivery to WASM.
pub enum UdpBroadcastEvent {
    /// Response received — carries (response_data, source_address).
    Response(String, String),
}

/// An active UDP broadcast session managed by a background thread.
pub struct ActiveUdpBroadcast {
    /// Channel to receive events from the background thread.
    pub event_rx: mpsc::Receiver<UdpBroadcastEvent>,
    /// Signal the background thread to stop.
    pub stop_tx: mpsc::Sender<()>,
}

/// An active mDNS service registration.
pub struct ActiveMdnsRegistration {
    /// The daemon owning this registration.
    pub daemon: mdns_sd::ServiceDaemon,
    /// Full service name (for unregistration).
    pub fullname: String,
}

/// An inbound HTTP request queued for delivery to WASM.
pub struct HttpInboundRequest {
    pub request_id: u32,
    pub method: String,
    pub path: String,
    pub headers: String,
    pub body: Vec<u8>,
    /// Channel for the background thread to receive the WASM response.
    pub response_tx: mpsc::Sender<HttpListenerResponse>,
}

/// Response data sent from WASM back to the HTTP listener background thread.
pub struct HttpListenerResponse {
    pub status: u16,
    pub headers: String,
    pub body: Vec<u8>,
}

/// An active HTTP listener managed by a background thread.
pub struct ActiveHttpListener {
    /// Receiver for inbound requests from the background thread.
    pub request_rx: mpsc::Receiver<HttpInboundRequest>,
    /// Signal to stop the listener.
    pub stop_tx: mpsc::Sender<()>,
    /// Actual bound port (useful when port=0 for ephemeral).
    pub port: u16,
}

/// Per-frame timing breakdown (microseconds).
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameTimings {
    /// Total WASM interpreter time (outer envelope, includes tree processing).
    pub wasm_us: u32,
    /// Tree binary deserialization.
    pub deserialize_us: u32,
    /// Taffy tree build + layout computation.
    pub layout_us: u32,
    /// render_taffy_node + modal rendering.
    pub render_us: u32,
    /// FemtoVG canvas.flush().
    pub flush_us: u32,
}

/// Host-side state accessible to WASM via host functions.
pub struct HostState {
    /// GPU renderer (FemtoVG + cosmic-text)
    pub renderer: FemtoVgRenderer,

    /// Interaction state (hit testing, pending clicks)
    pub interaction: InteractionState,

    /// Server-provided state blob (read by the host runtime, not testbed)
    #[expect(dead_code)]
    pub state_blob: Option<Vec<u8>>,

    /// Whether `request_frame()` was called this frame
    pub frame_requested: bool,

    /// Delay from `request_frame_after(ms)`, if called
    pub frame_delay_ms: Option<u32>,

    /// Whether to request server refresh (read by the host runtime, not testbed)
    #[expect(dead_code)]
    pub refresh_requested: bool,

    /// One-shot clicks on buttons and interactive canvases (on finger-up)
    pub tree_clicks: HashMap<String, crate::tree::TouchHit>,

    /// Active drag positions on interactive canvases (while finger is down)
    pub tree_drags: HashMap<String, crate::tree::TouchHit>,

    /// Modal dialog states (keyed by modal_id string)
    pub modal_states: HashMap<String, ModalState>,

    /// Scroll container states (keyed by scroll_id)
    pub scroll_states: HashMap<u16, ScrollState>,

    /// Delta time since last frame (for animations)
    pub delta_ms: u32,

    /// Running animation states, keyed by content hash.
    pub animation_states: HashMap<u64, AnimationState>,

    /// Running transition states, keyed by (canvas_index, draw_index).
    pub transition_states: HashMap<(u16, u16), TransitionState>,

    /// Monotonic frame counter for GC.
    pub frame_counter: u64,

    /// Cached deserialized tree for animation-only frames (tree, width, height).
    pub cached_tree: Option<(crate::tree::TreeNode, f32, f32)>,

    /// Whether the next frame only needs animation updates (no WASM execution).
    pub animation_only_frame: bool,

    /// Wall-clock deadline for the next forced WASM render (from
    /// `request_frame_after`). Uses `Instant` instead of counting down by
    /// `delta_ms` because sub-millisecond frames truncate to 0 and stall
    /// countdown timers.
    pub deferred_wasm_render_at: Option<Instant>,

    /// Wall-clock time of the last full WASM render. Used to compute the
    /// real elapsed delta for WASM (not just the animation frame's ~16ms).
    pub last_wasm_render_at: Instant,

    /// Next request ID for fetch.
    pub next_request_id: u32,

    /// Receiver for completed fetch responses from background threads.
    pub fetch_rx: mpsc::Receiver<CompletedFetch>,

    /// Sender cloned into each background fetch thread.
    pub fetch_tx: mpsc::Sender<CompletedFetch>,

    /// Pending delayed fetches.
    pub delayed_fetches: Vec<DelayedFetch>,

    /// Number of HTTP fetches currently in flight (spawned but not yet completed).
    pub in_flight_fetches: u32,

    /// Parsed JSON documents, keyed by doc_id.
    pub json_docs: HashMap<u32, Value>,

    /// Next JSON document ID.
    pub next_json_id: u32,

    /// Parsed XML documents (stored as raw strings for roxmltree re-parsing).
    pub xml_docs: HashMap<u32, String>,

    /// Next XML document ID.
    pub next_xml_id: u32,

    /// Active WebSocket connections, keyed by ws_id.
    pub websockets: HashMap<u32, ActiveWebSocket>,

    /// Next WebSocket connection ID.
    pub next_ws_id: u32,

    /// Active TLS socket connections, keyed by socket_id.
    pub sockets: HashMap<u32, ActiveSocket>,

    /// Next TLS socket connection ID.
    pub next_socket_id: u32,

    /// Active mDNS browse sessions, keyed by browse_id.
    pub mdns_browses: HashMap<u32, ActiveMdnsBrowse>,

    /// Next mDNS browse ID.
    pub next_mdns_browse_id: u32,

    /// Active mDNS service registrations, keyed by reg_id.
    pub mdns_registrations: HashMap<u32, ActiveMdnsRegistration>,

    /// Next mDNS registration ID.
    pub next_mdns_reg_id: u32,

    /// Active SSDP search sessions, keyed by search_id.
    pub ssdp_searches: HashMap<u32, ActiveSsdpSearch>,

    /// Next SSDP search ID.
    pub next_ssdp_search_id: u32,

    /// Active UDP broadcast sessions, keyed by broadcast_id.
    pub udp_broadcasts: HashMap<u32, ActiveUdpBroadcast>,

    /// Next UDP broadcast ID.
    pub next_udp_broadcast_id: u32,

    /// Per-widget key-value storage directory (None = persistence disabled).
    pub kv_store_path: Option<PathBuf>,

    /// In-memory cache of KV data (lazy-loaded from disk on first access).
    pub kv_cache: HashMap<String, Vec<u8>>,

    /// Active HTTP listeners, keyed by listener_id.
    pub http_listeners: HashMap<u32, ActiveHttpListener>,

    /// Next HTTP listener ID.
    pub next_http_listener_id: u32,

    /// Per-request response senders: request_id → sender for the response.
    pub http_response_txs: HashMap<u32, mpsc::Sender<HttpListenerResponse>>,

    /// User formatting preferences (number format, unit system, temperature unit).
    pub prefs: FormatPreferences,

    /// Per-frame timing breakdown from the last rendered frame.
    pub last_timings: FrameTimings,

    /// Reusable Taffy layout tree (cleared each frame, keeps allocations).
    pub taffy: TaffyTree<NodeContext>,
}

impl HostState {
    /// Create new host state with the given renderer and formatting preferences.
    pub fn new(renderer: FemtoVgRenderer, prefs: FormatPreferences) -> Self {
        let (fetch_tx, fetch_rx) = mpsc::channel();
        Self {
            renderer,
            interaction: InteractionState::new(),
            state_blob: None,
            frame_requested: false,
            frame_delay_ms: None,
            refresh_requested: false,
            tree_clicks: HashMap::new(),
            tree_drags: HashMap::new(),
            modal_states: HashMap::new(),
            scroll_states: HashMap::new(),
            delta_ms: 0,
            animation_states: HashMap::new(),
            transition_states: HashMap::new(),
            frame_counter: 0,
            cached_tree: None,
            animation_only_frame: false,
            deferred_wasm_render_at: None,
            last_wasm_render_at: Instant::now(),
            next_request_id: 1,
            fetch_rx,
            fetch_tx,
            delayed_fetches: Vec::new(),
            in_flight_fetches: 0,
            json_docs: HashMap::new(),
            next_json_id: 1,
            xml_docs: HashMap::new(),
            next_xml_id: 1,
            websockets: HashMap::new(),
            next_ws_id: 1,
            sockets: HashMap::new(),
            next_socket_id: 1,
            mdns_browses: HashMap::new(),
            next_mdns_browse_id: 1,
            mdns_registrations: HashMap::new(),
            next_mdns_reg_id: 1,
            ssdp_searches: HashMap::new(),
            next_ssdp_search_id: 1,
            udp_broadcasts: HashMap::new(),
            next_udp_broadcast_id: 1,
            kv_store_path: None,
            kv_cache: HashMap::new(),
            http_listeners: HashMap::new(),
            next_http_listener_id: 1,
            http_response_txs: HashMap::new(),
            prefs,
            last_timings: FrameTimings::default(),
            taffy: TaffyTree::with_capacity(64),
        }
    }

    /// Reset per-frame flags.
    pub fn begin_render_frame(&mut self) {
        self.frame_requested = false;
        self.frame_delay_ms = None;
    }
}
