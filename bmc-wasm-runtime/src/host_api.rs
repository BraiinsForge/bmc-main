// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host state and function bindings for WASM.
//!
//! # `glam` is intentionally part of this crate's public surface.
//!
//! `PrevDrawValues` embeds `glam::Quat` and `glam::Vec3` directly, and this
//! module re-exports both types. Anything that consumes host-side draw state
//! transitively depends on `glam`. That is deliberate: the host's animation
//! and transition pipeline is built around glam math (see `compute_mvp`,
//! `quat_to_mat3`, etc.), so swapping in a private wrapper would only push
//! the dependency one level outward without buying anything.
//!
//! The guest-side SDK keeps glam at arm's length — `bmc_wasm_sdk::Orientation`
//! is the public guest type and converts to/from `glam::Quat` only behind
//! `From` impls. Don't try to mirror that pattern here; the host is glam-bound
//! by design.

use std::collections::HashMap;
use std::path::PathBuf;

pub use glam::{Quat, Vec3};
use std::sync::mpsc;

use serde_json::Value;
use taffy::prelude::*;

use bmc_wasm_protocol::FormatPreferences;

use crate::gpu::FemtoVgRenderer;
use crate::interaction::InteractionState;
use crate::runtime_limits::RuntimeResourceLimits;
use crate::tree::NodeContext;
use crate::xml::XmlDocumentIndex;

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
    // -- Mesh fields (for 3D mesh transitions) --
    /// Orientation quaternion [x, y, z, w].
    pub orientation: Quat,
    /// Camera field of view (degrees).
    pub fov: f32,
    /// Camera distance from origin.
    pub distance: f32,
    /// Uniform scale factor.
    pub mesh_scale: f32,
    /// Position offset [x, y, z].
    pub position: Vec3,
    /// Light direction pitch (degrees).
    pub light_pitch: f32,
    /// Light direction yaw (degrees).
    pub light_yaw: f32,
    /// Ambient light level (0.0–1.0).
    pub ambient: f32,
    /// Specular highlight strength (0.0–1.0).
    pub specular: f32,
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
    pub fire_at_ms: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub request_id: u32,
}

/// A timestamped event for fixture replay (SSDP, mDNS, WebSocket, etc.).
#[derive(Debug)]
pub struct FixtureEvent {
    /// Virtual time (monotonic ms) when this event should fire.
    pub at_ms: u64,
    /// The event payload.
    pub kind: FixtureEventKind,
}

/// Payload for a single fixture event.
#[derive(Debug)]
pub enum FixtureEventKind {
    SsdpFound {
        search_id: u32,
        data: String,
    },
    SsdpRemoved {
        search_id: u32,
        data: String,
    },
    MdnsFound {
        browse_id: u32,
        data: String,
    },
    MdnsRemoved {
        browse_id: u32,
        data: String,
    },
    WsOpen {
        ws_id: u32,
    },
    WsMessage {
        ws_id: u32,
        data: Vec<u8>,
    },
    WsClose {
        ws_id: u32,
        code: u16,
    },
    SocketConnected {
        socket_id: u32,
    },
    SocketData {
        socket_id: u32,
        data: Vec<u8>,
    },
    SocketClosed {
        socket_id: u32,
        code: u32,
    },
    UdpResponse {
        broadcast_id: u32,
        data: String,
        source: String,
    },
}

/// State for event fixture replay — holds sorted events and stub channel senders.
#[derive(Debug)]
pub struct FixtureEventState {
    /// Events sorted by `at_ms` (ascending).
    pub events: Vec<FixtureEvent>,
    /// Next event index to replay.
    pub cursor: usize,
    /// Senders for injecting WebSocket events into stub connections.
    pub ws_event_txs: HashMap<u32, mpsc::Sender<WsEvent>>,
    /// Senders for injecting socket events into stub connections.
    pub socket_event_txs: HashMap<u32, mpsc::Sender<SocketEvent>>,
    /// Senders for injecting mDNS events into stub browse sessions.
    pub mdns_event_txs: HashMap<u32, mpsc::Sender<MdnsEvent>>,
    /// Senders for injecting SSDP events into stub search sessions.
    pub ssdp_event_txs: HashMap<u32, mpsc::Sender<SsdpEvent>>,
    /// Senders for injecting UDP broadcast events into stub sessions.
    pub udp_event_txs: HashMap<u32, mpsc::Sender<UdpBroadcastEvent>>,
}

/// A WebSocket event queued for delivery to WASM.
#[derive(Debug)]
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
#[derive(Debug)]
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
#[derive(Debug)]
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
#[derive(Debug)]
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
#[derive(Debug)]
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

/// Frame-scheduler state shared by WASM wakeups and cached-tree animation replays.
pub(crate) struct FrameScheduleState {
    /// Whether `request_frame()` was called this frame
    pub frame_requested: bool,

    /// Delay before the next host wake, if another frame was requested.
    ///
    /// This may be shorter than the widget's requested
    /// `request_frame_after(ms)` delay when host-side cached-tree animations
    /// need intermediate frames between full WASM renders.
    pub frame_delay_ms: Option<u32>,

    /// Whether the next frame only needs animation updates (no WASM execution).
    pub animation_only_frame: bool,

    /// Whether the last successfully submitted/rendered tree still has active
    /// host-side animations or transitions that require cached-tree updates.
    pub has_active_animations: bool,

    /// Frame poll cadence (ms) used to clamp `frame_delay_ms` while host-side
    /// animations are active. Set from `RuntimeConfig::animation_frame_delay_ms`.
    pub animation_frame_delay_ms: u32,

    /// Monotonic deadline (ms) for the next forced full WASM render requested
    /// by `request_frame_after`. This keeps the widget's semantic wakeup time
    /// separate from `frame_delay_ms`, which the host may clamp earlier while
    /// cached-tree animations are active.
    ///
    /// Uses monotonic_ms instead of counting down by `delta_ms` because
    /// sub-millisecond frames truncate to 0 and stall countdown timers.
    pub deferred_wasm_render_at_ms: Option<u64>,

    /// Monotonic ms at the last full WASM render. Used to compute the
    /// real elapsed delta for WASM (not just the animation frame's ~16ms).
    pub last_wasm_render_at_ms: u64,
}

impl FrameScheduleState {
    fn new() -> Self {
        Self {
            frame_requested: false,
            frame_delay_ms: None,
            animation_only_frame: false,
            has_active_animations: false,
            animation_frame_delay_ms: crate::RuntimeConfig::DEFAULT_ANIMATION_FRAME_DELAY_MS,
            deferred_wasm_render_at_ms: None,
            last_wasm_render_at_ms: 0,
        }
    }

    /// Reset per-frame wakeup state before a new host render pass begins.
    pub fn begin_render_frame(&mut self) {
        self.frame_requested = false;
        self.frame_delay_ms = None;
        self.has_active_animations = false;
    }
}

/// Host-side state accessible to WASM via host functions.
pub(crate) struct HostState {
    /// GPU renderer (FemtoVG + cosmic-text)
    pub renderer: FemtoVgRenderer,

    /// Interaction state (hit testing, pending clicks)
    pub interaction: InteractionState,

    /// Scheduler state for full WASM reruns and cached-tree animation wakes.
    pub frame_schedule: FrameScheduleState,

    /// One-shot clicks on buttons and interactive canvases (on finger-up)
    pub tree_clicks: HashMap<String, crate::tree::TouchHit>,

    /// Active drag positions on interactive canvases (while finger is down)
    pub tree_drags: HashMap<String, crate::tree::TouchHit>,

    /// Modal dialog states (keyed by modal_id string)
    pub modal_states: HashMap<String, ModalState>,

    /// Scroll container states (keyed by scroll_key string)
    pub scroll_states: HashMap<String, ScrollState>,

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

    /// Current wall-clock time, set by the host before each render().
    /// Used by `host_get_system_time()` — the runtime never calls `Local::now()`.
    pub system_time: chrono::DateTime<chrono::FixedOffset>,

    /// Monotonic clock in ms, set by the host before each render().
    /// Used for deferred timer checks and wasm_delta computation.
    pub monotonic_ms: u64,

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

    /// Owned XML lookup indices built once at `host_xml_parse` time.
    pub xml_indices: HashMap<u32, XmlDocumentIndex>,

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

    /// Optional host-provided interceptor for fetch requests.
    /// Called with `(method, url)` before hitting the network.
    /// Return `Some((status, body))` to short-circuit, `None` to proceed normally.
    pub fetch_interceptor: Option<crate::runtime::FetchInterceptor>,

    /// Called when a fetch response is delivered. Use for recording/logging.
    pub fetch_observer: Option<crate::runtime::FetchObserver>,

    /// Maps `request_id` → fixture key (e.g. "GET https://...") for the observer.
    pub fetch_keys: HashMap<u32, String>,

    /// Whether to record network events for fixture generation.
    pub record_events: bool,

    /// Event fixture replay state (SSDP, mDNS, WebSocket, etc.).
    /// When present, host functions create stub channels instead of real connections.
    pub event_fixtures: Option<FixtureEventState>,

    /// Buffer for recording live events when `record_events` is enabled.
    /// Populated by `deliver_*` methods; drained via `take_recorded_events()`.
    pub recorded_events: Vec<FixtureEvent>,

    /// User formatting preferences (number format, unit system, temperature unit).
    pub prefs: FormatPreferences,

    /// Per-frame timing breakdown from the last rendered frame.
    pub last_timings: FrameTimings,

    /// Reusable Taffy layout tree (cleared each frame, keeps allocations).
    pub taffy: TaffyTree<NodeContext>,

    /// Per-runtime caps for host-side resources.
    pub resource_limits: RuntimeResourceLimits,
    /// xorshift64 PRNG state. `None` means "not yet seeded" — the next
    /// `host_random_u32` call lazy-seeds from `monotonic_ms`. `Some(s)` is
    /// the live xorshift state, including any deterministic seed forwarded
    /// from `RuntimeConfig::rng_seed`.
    pub rng_state: Option<u64>,
}

impl HostState {
    /// Create new host state with the given renderer and formatting preferences.
    pub fn new(
        renderer: FemtoVgRenderer,
        prefs: FormatPreferences,
        resource_limits: RuntimeResourceLimits,
    ) -> Self {
        let (fetch_tx, fetch_rx) = mpsc::channel();
        Self {
            renderer,
            interaction: InteractionState::new(),
            frame_schedule: FrameScheduleState::new(),
            tree_clicks: HashMap::new(),
            tree_drags: HashMap::new(),
            modal_states: HashMap::new(),
            scroll_states: HashMap::new(),
            delta_ms: 0,
            animation_states: HashMap::new(),
            transition_states: HashMap::new(),
            frame_counter: 0,
            cached_tree: None,
            system_time: chrono::Local::now().fixed_offset(),
            monotonic_ms: 0,
            next_request_id: 1,
            fetch_rx,
            fetch_tx,
            delayed_fetches: Vec::new(),
            in_flight_fetches: 0,
            json_docs: HashMap::new(),
            next_json_id: 1,
            xml_indices: HashMap::new(),
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
            fetch_interceptor: None,
            fetch_observer: None,
            fetch_keys: HashMap::new(),
            record_events: false,
            event_fixtures: None,
            recorded_events: Vec::new(),
            prefs,
            last_timings: FrameTimings::default(),
            taffy: TaffyTree::with_capacity(64),
            resource_limits,
            rng_state: None, // None = auto-seed on first use (from monotonic_ms)
        }
    }

    /// Reset per-frame flags.
    pub fn begin_render_frame(&mut self) {
        self.frame_schedule.begin_render_frame();
    }

    #[must_use]
    pub fn fetch_slots_used(&self) -> usize {
        self.delayed_fetches
            .len()
            .saturating_add(self.in_flight_fetches as usize)
    }
}
