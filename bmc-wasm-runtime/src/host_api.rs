// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host state and function bindings for WASM.
//!
//! Animation/transition draw-state (`PrevDrawValues`, `AnimationState`,
//! `TransitionState`, `ModalState`, `ScrollState`) lives in [`bmc_render`]
//! together with the renderer that owns it; this module imports those types.
//! See `bmc_render`'s crate-level docs for the rationale on `glam` being part
//! of the host-side public surface.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;
use taffy::prelude::*;

use bmc_render::interaction::InteractionState;
use bmc_render::renderer::Renderer;
use bmc_render::tree::NodeContext;
use bmc_render::{AnimationState, ModalState, ScrollState, TransitionState, TransitionStateKey};
use bmc_wasm_protocol::{
    AudioId, FetchRequestId, HttpListenerId, HttpRequestId, JsonId, MdnsBrowseId, MdnsRegId,
    SocketId, SsdpSearchId, UdpBroadcastId, WebsocketId, XmlId,
};

use crate::audio_registry::AudioRegistry;
use crate::runtime::ParamsSnapshot;
use crate::runtime_limits::RuntimeResourceLimits;
use crate::system::SystemSnapshot;
use crate::xml::XmlDocumentIndex;
use bmc_wasm_protocol::versioned_snapshot::VersionedSnapshotCache;

/// A completed HTTP fetch response ready for delivery to WASM.
pub struct CompletedFetch {
    pub request_id: FetchRequestId,
    pub status: u32,
    pub body: Vec<u8>,
}

#[cfg(feature = "testing")]
impl CompletedFetch {
    pub fn test_sentinel() -> Self {
        Self {
            request_id: bmc_wasm_protocol::FetchRequestId::from_wire(1)
                .expect("BUG: 1 is non-zero so from_wire returns Some"),
            status: 0,
            body: Vec::new(),
        }
    }
}

/// A delayed fetch waiting for its fire time.
pub struct DelayedFetch {
    pub fire_at_ms: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub request_id: FetchRequestId,
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
#[derive(Debug, PartialEq)]
pub enum FixtureEventKind {
    SsdpFound {
        search_id: SsdpSearchId,
        data: String,
    },
    SsdpRemoved {
        search_id: SsdpSearchId,
        data: String,
    },
    MdnsFound {
        browse_id: MdnsBrowseId,
        data: String,
    },
    MdnsRemoved {
        browse_id: MdnsBrowseId,
        data: String,
    },
    WsOpen {
        ws_id: WebsocketId,
    },
    WsMessage {
        ws_id: WebsocketId,
        data: Vec<u8>,
    },
    WsClose {
        ws_id: WebsocketId,
        code: u16,
    },
    SocketConnected {
        socket_id: SocketId,
    },
    SocketData {
        socket_id: SocketId,
        data: Vec<u8>,
    },
    SocketClosed {
        socket_id: SocketId,
        code: u32,
    },
    UdpResponse {
        broadcast_id: UdpBroadcastId,
        data: String,
        source: String,
    },
    AudioPlay {
        sound_id: AudioId,
        volume: u32,
        name: String,
        duration_ms: u32,
    },
    LedSetEffect {
        effect: u8,
        r: u8,
        g: u8,
        b: u8,
        period_ms: u32,
        duration_ms: u32,
    },
    LedSetBrightness {
        brightness: f32,
    },
    LedEnable,
    LedDisable,
}

/// Phase of the guest lifecycle the runtime is currently executing.
///
/// Set immediately before each guest-call site in `WasmWidgetRuntime` and reset to [`Self::Idle`]
/// when the call returns. Read by guarded host imports to decide whether the call is allowed
/// in the current phase (the matrix lives in `runtime/imports/guards.rs`).
///
/// Single-threaded guest + host-serialised calls = at most one non-`Idle` value at any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    /// Outside any guest call. Most imports are dormant; assets registered here would have no
    /// frame to bind to and trap (pre-existing behaviour).
    Idle,
    /// `init` is on the stack. Setup work — assets, KV reads, request_frame, etc.
    Init,
    /// `render` is on the stack. `host_submit_tree`, touch readbacks, and frame requests are
    /// all legal here.
    Render,
    /// `on_params_update` is on the stack. State mutation + `request_frame` are legal;
    /// submitting a tree is not (the next render is the rendering opportunity).
    ParamsUpdate,
    /// `on_system_update` is on the stack. Same import surface as [`Self::ParamsUpdate`],
    /// but a separate phase so traps and logs name the right hook.
    SystemUpdate,
    /// `on_touch` is on the stack. Fired once per Wayland drain that delivered
    /// touch activity. `request_frame` is legal (and is how the widget asks to
    /// re-render in response); submitting a tree and touch readback are not —
    /// the queued touch is consumed at the next render, not here.
    Touch,
    /// `unload` is on the stack. Synchronous cleanup only; frame requests no-op.
    Unload,
}

/// Identifies a single `WasmWidgetRuntime` instance.
///
/// Minted from a process-wide monotonic counter at HostState construction.
/// Used as the leading component of every asset tag the host stores,
/// so two instances of the same widget can't collide on slot names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GuestId(u32);

impl GuestId {
    fn alloc() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl std::fmt::Display for GuestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A registered audio sample with metadata.
#[derive(Debug)]
pub struct AudioSample {
    /// Raw encoded audio data (WAV/OGG/MP3).
    pub data: Arc<[u8]>,
    /// Human-readable name (from widget registration).
    pub name: String,
    /// Duration in milliseconds (computed at registration by decoding).
    pub duration_ms: u32,
}

/// State for event fixture replay — holds sorted events and stub channel senders.
#[derive(Debug)]
pub struct FixtureEventState {
    /// Events sorted by `at_ms` (ascending).
    pub events: Vec<FixtureEvent>,
    /// Next event index to replay.
    pub cursor: usize,
    /// Senders for injecting WebSocket events into stub connections.
    pub ws_event_txs: HashMap<WebsocketId, mpsc::Sender<WsEvent>>,
    /// Senders for injecting socket events into stub connections.
    pub socket_event_txs: HashMap<SocketId, mpsc::Sender<SocketEvent>>,
    /// Senders for injecting mDNS events into stub browse sessions.
    pub mdns_event_txs: HashMap<MdnsBrowseId, mpsc::Sender<MdnsEvent>>,
    /// Senders for injecting SSDP events into stub search sessions.
    pub ssdp_event_txs: HashMap<SsdpSearchId, mpsc::Sender<SsdpEvent>>,
    /// Senders for injecting UDP broadcast events into stub sessions.
    pub udp_event_txs: HashMap<UdpBroadcastId, mpsc::Sender<UdpBroadcastEvent>>,
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

/// A decoded mDNS Found event accumulated for testing inspection.
#[cfg(feature = "testing")]
#[derive(Debug)]
pub struct CapturedMdnsEvent {
    pub fullname: String,
}

/// An active mDNS service registration.
pub struct ActiveMdnsRegistration {
    /// The daemon owning this registration.
    pub daemon: mdns_sd::ServiceDaemon,
    /// Full service name (for unregistration).
    pub fullname: String,
}

/// An inbound HTTP request queued for delivery to WASM.
///
/// The `request_id` is assigned host-side at delivery time (so it can come
/// from a single shared counter), not in the listener thread.
pub struct HttpInboundRequest {
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

pub use bmc_render::FrameTimings;

/// Frame-scheduler state.
///
/// Tracks the three independent inputs that decide what the host does next:
///
/// * [`Self::widget_delay_ms`] — what the widget asked for via
///   `request_frame()` (= `Some(0)`) or `request_frame_after(N)`.
/// * [`Self::has_active_animations`] — whether the last submitted tree still
///   has running animations or transitions.
/// * [`Self::interaction_pending`] — whether clicks/drags were delivered this
///   frame and the widget has not yet had a chance to render its reaction.
///
/// Whether to render at all ([`Self::wants_next_frame`]), how long to wait
/// ([`Self::effective_delay_ms`]) and whether the next frame can skip WASM
/// ([`Self::is_animation_only_frame`]) are all *derived* on query rather
/// than stored. Storing them was the source of a last-writer-wins bug where
/// the widget's `request_frame_after` could clobber a runtime-imposed
/// clamp set earlier in the same frame.
pub(crate) struct FrameScheduleState {
    /// Widget's requested delay before the next render. `Some(0)` ≡
    /// `request_frame()`; `Some(n)` ≡ `request_frame_after(n)`; `None` means
    /// no widget request this frame.
    pub widget_delay_ms: Option<u32>,

    /// Whether the last submitted tree still has active host-side animations
    /// or transitions that require further frames.
    pub has_active_animations: bool,

    /// Whether clicks/drags were delivered this frame. Forces the next frame
    /// to be an immediate full-WASM render so the widget can react — even if
    /// the widget then asked for a longer `request_frame_after` after
    /// consuming the interaction.
    pub interaction_pending: bool,

    /// Frame poll cadence (ms) capping the effective wake while animations
    /// are active. Set from `RuntimeConfig::animation_frame_delay_ms`.
    pub animation_frame_delay_ms: u32,

    /// Monotonic deadline (ms) for the next forced full WASM render requested
    /// by `request_frame_after`. Kept separate from the effective host wake,
    /// which may be clamped earlier while animations are active.
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
            widget_delay_ms: None,
            has_active_animations: false,
            interaction_pending: false,
            animation_frame_delay_ms: crate::RuntimeConfig::DEFAULT_ANIMATION_FRAME_DELAY_MS,
            deferred_wasm_render_at_ms: None,
            last_wasm_render_at_ms: 0,
        }
    }

    /// Reset per-frame inputs before a new host render pass begins.
    pub fn begin_render_frame(&mut self) {
        self.widget_delay_ms = None;
        self.has_active_animations = false;
        self.interaction_pending = false;
    }

    /// Whether anything wants the host to render a next frame.
    pub fn wants_next_frame(&self) -> bool {
        self.widget_delay_ms.is_some() || self.has_active_animations || self.interaction_pending
    }

    /// Whether the next frame can replay the cached tree without running
    /// WASM. Only when animations are active and neither the widget nor a
    /// pending interaction needs WASM to run.
    pub fn is_animation_only_frame(&self) -> bool {
        self.has_active_animations && !self.interaction_pending && self.widget_delay_ms.is_none()
    }

    /// Effective delay before the host should wake for the next render —
    /// the min of all active constraints.
    pub fn effective_delay_ms(&self) -> Option<u32> {
        if self.interaction_pending {
            return Some(0);
        }
        let cap = self
            .has_active_animations
            .then_some(self.animation_frame_delay_ms);
        match (self.widget_delay_ms, cap) {
            (Some(d), Some(c)) => Some(d.min(c)),
            (Some(d), None) => Some(d),
            (None, Some(c)) => Some(c),
            (None, None) => None,
        }
    }
}

/// Host-side state accessible to WASM via host functions.
pub(crate) struct HostState {
    /// Renderer parked by `WasmWidgetRuntime::with_renderer` for the duration of a
    /// render scope. `None` outside a render scope; host imports that read this must
    /// trap the guest with `wasmi::Error::new("renderer accessed outside render scope")`
    /// rather than panic the host.
    pub renderer_ptr: Option<NonNull<dyn Renderer>>,

    /// Interaction state (hit testing, pending clicks)
    pub interaction: InteractionState,

    /// Scheduler state for full WASM reruns and cached-tree animation wakes.
    pub frame_schedule: FrameScheduleState,

    /// One-shot clicks on buttons and interactive canvases (on finger-up)
    pub tree_clicks: HashMap<String, bmc_render::tree::TouchHit>,

    /// Active drag positions on interactive canvases (while finger is down)
    pub tree_drags: HashMap<String, bmc_render::tree::TouchHit>,

    /// Modal dialog states (keyed by modal_id string)
    pub modal_states: HashMap<String, ModalState>,

    /// Scroll container states (keyed by scroll_key string)
    pub scroll_states: HashMap<String, ScrollState>,

    /// Delta time since last frame (for animations)
    pub delta_ms: u32,

    /// Running animation states, keyed by content hash.
    pub animation_states: HashMap<u64, AnimationState>,

    /// Running transition states, keyed by (canvas_index, draw_index).
    pub transition_states: HashMap<TransitionStateKey, TransitionState>,

    /// Monotonic frame counter for GC.
    pub frame_counter: u64,

    /// Cached deserialized tree for animation-only frames (tree, width, height).
    pub cached_tree: Option<(bmc_render::tree::TreeNode, f32, f32)>,

    /// Current wall-clock time, set by the host before each render().
    /// Used by `host_get_system_time()` — the runtime never calls `Local::now()`.
    pub system_time: chrono::DateTime<chrono::FixedOffset>,

    /// Monotonic clock in ms, set by the host before each render().
    /// Used for deferred timer checks and wasm_delta computation.
    pub monotonic_ms: u64,

    /// Per-instance widget parameters, materialised from the wayland `deck_widget_v1.params`
    /// event (compositor) or applied directly from the manifest defaults (testbed).
    ///
    /// Order is alphabetical-by-key (the inner `BTreeMap` inside [`ParamsSnapshot`]) so the
    /// on-wire serialisation is deterministic and snapshot byte-equality is meaningful.
    ///
    /// The wrapping [`VersionedSnapshotCache`] is the encapsulation:
    /// it folds the source-of-truth value, the change marker the SDK reads
    /// via `host_params_version`, and the lazily-encoded bytes `host_params_snapshot`
    /// writes to guest memory into one unit. `replace()` is the only mutation path
    /// — version-bump and cache-invalidation invariants live inside the cache,
    /// so the field is `pub` here without compromising them.
    pub params: bmc_wasm_protocol::versioned_snapshot::VersionedSnapshotCache<ParamsSnapshot>,

    /// Guest-lifecycle phase the runtime is currently in. See [`Lifecycle`].
    /// Single-threaded guest, so this is a plain field — only one phase can be active at a time.
    pub current_lifecycle: Lifecycle,

    /// Next fetch request ID counter (for `FetchRequestId::alloc`).
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
    pub json_docs: HashMap<JsonId, Value>,

    /// Next JSON document ID counter (for `JsonId::alloc`).
    pub next_json_id: u32,

    /// Owned XML lookup indices built once at `host_xml_parse` time.
    pub xml_indices: HashMap<XmlId, XmlDocumentIndex>,

    /// Next XML document ID counter (for `XmlId::alloc`).
    pub next_xml_id: u32,

    /// Active WebSocket connections, keyed by ws_id.
    pub websockets: HashMap<WebsocketId, ActiveWebSocket>,

    /// Next WebSocket ID counter (for `WebsocketId::alloc`).
    pub next_ws_id: u32,

    /// Active TLS socket connections, keyed by socket_id.
    pub sockets: HashMap<SocketId, ActiveSocket>,

    /// Next socket ID counter (for `SocketId::alloc`).
    pub next_socket_id: u32,

    /// Active mDNS browse sessions, keyed by browse_id.
    pub mdns_browses: HashMap<MdnsBrowseId, ActiveMdnsBrowse>,

    /// Next mDNS browse ID counter (for `MdnsBrowseId::alloc`).
    pub next_mdns_browse_id: u32,

    /// Active mDNS service registrations, keyed by reg_id.
    pub mdns_registrations: HashMap<MdnsRegId, ActiveMdnsRegistration>,

    /// Next mDNS registration ID counter (for `MdnsRegId::alloc`).
    pub next_mdns_reg_id: u32,

    /// Active SSDP search sessions, keyed by search_id.
    pub ssdp_searches: HashMap<SsdpSearchId, ActiveSsdpSearch>,

    /// Next SSDP search ID counter (for `SsdpSearchId::alloc`).
    pub next_ssdp_search_id: u32,

    /// Active UDP broadcast sessions, keyed by broadcast_id.
    pub udp_broadcasts: HashMap<UdpBroadcastId, ActiveUdpBroadcast>,

    /// Next UDP broadcast ID counter (for `UdpBroadcastId::alloc`).
    pub next_udp_broadcast_id: u32,

    /// Per-widget key-value storage directory (None = persistence disabled).
    pub kv_store_path: Option<PathBuf>,

    /// In-memory cache of KV data (lazy-loaded from disk on first access).
    pub kv_cache: HashMap<String, Vec<u8>>,

    /// Active HTTP listeners, keyed by listener_id.
    pub http_listeners: HashMap<HttpListenerId, ActiveHttpListener>,

    /// Next HTTP listener ID counter (for `HttpListenerId::alloc`).
    pub next_http_listener_id: u32,

    /// Per-request response senders: request_id → sender for the response.
    pub http_response_txs: HashMap<HttpRequestId, mpsc::Sender<HttpListenerResponse>>,

    /// Next HTTP request ID counter (for `HttpRequestId::alloc`). Shared
    /// across all HTTP listeners so two concurrent listeners never emit the
    /// same `HttpRequestId` and overwrite each other in `http_response_txs`.
    pub next_http_request_id: u32,

    /// Optional host-provided interceptor for fetch requests.
    /// Called with `(method, url)` before hitting the network.
    /// Return `Some((status, body))` to short-circuit, `None` to proceed normally.
    pub fetch_interceptor: Option<crate::runtime::FetchInterceptor>,

    /// Called when a fetch response is delivered. Use for recording/logging.
    pub fetch_observer: Option<crate::runtime::FetchObserver>,

    /// Maps `request_id` → fixture key (e.g. "GET https://...") for the observer.
    pub fetch_keys: HashMap<FetchRequestId, String>,

    /// Shared `ureq::Agent` cloned into every background fetch thread for
    /// connection-pool reuse. The timeout is set per request by `do_fetch`.
    pub fetch_agent: ureq::Agent,

    /// Whether to record network events for fixture generation.
    pub record_events: bool,

    /// Event fixture replay state (SSDP, mDNS, WebSocket, etc.).
    /// When present, host functions create stub channels instead of real connections.
    pub event_fixtures: Option<FixtureEventState>,

    /// Buffer for recording live events when `record_events` is enabled.
    /// Populated by `deliver_*` methods; drained via `take_recorded_events()`.
    pub recorded_events: Vec<FixtureEvent>,

    /// Deck-wide system state delivered to widgets — operator-controlled settings
    /// (timezone, time/date/number/temperature/unit formats, week start) and the
    /// resolved next-alarm entry. The runtime's `host_format_*` imports read
    /// the relevant fields directly out of `system.snapshot().settings` so a
    /// settings change is observable to widgets without any extra plumbing.
    pub system: VersionedSnapshotCache<SystemSnapshot>,

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

    pub(crate) shut_down: bool,
    #[cfg(feature = "testing")]
    pub(crate) unload_ran: bool,
    #[cfg(feature = "testing")]
    pub(crate) delivered_events: u64,
    #[cfg(feature = "testing")]
    pub mdns_captured_events: Vec<CapturedMdnsEvent>,

    /// Sender for LED commands. `None` when LED control is unavailable.
    pub led_command_sender: Option<mpsc::Sender<bmc_led::data::LedCommand>>,

    /// Registered audio samples + tag dedup + active playback sinks.
    /// The host stores original encoded data and decodes on each play.
    pub audio: AudioRegistry,

    /// Per-instance identity used to namespace every asset tag the host stores.
    /// Two `WasmWidgetRuntime`s for the same widget get different `guest_id`s,
    /// so their `<guest_id>:<tag>` registrations and prefix evictions can't collide.
    pub guest_id: GuestId,

    /// Audio output stream — must stay alive for the entire session.
    /// `None` if audio output is unavailable (headless, no ALSA, etc.).
    #[cfg(feature = "audio")]
    pub audio_stream: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>,

    /// Logical viewport dimensions the widget renders into.
    /// Set by `WasmWidgetRuntime::new` before the guest's
    /// `init` runs and never mutated thereafter.
    ///
    /// Read via the `host_widget_size` import
    /// (SDK `widget_size()` free function).
    pub widget_width: u32,
    pub widget_height: u32,
    pub viewport_shape: bmc_wasm_protocol::ViewportShape,
    pub display_width: u32,
    pub display_height: u32,
    pub display_shape: bmc_wasm_protocol::DisplayShape,
    pub display_dpi: u32,
}

impl HostState {
    /// Create new host state with default `system` / `params` snapshots
    /// (version 0). The runtime constructor stages the operator-supplied
    /// initial snapshots via `.replace(...)` to bump both to version 1
    /// before `init()` runs.
    ///
    /// The renderer is owned by the caller of [`crate::WasmWidgetRuntime::new`]
    /// and installed on `renderer_ptr` per-frame via
    /// `WasmWidgetRuntime::with_renderer`.
    pub fn new(resource_limits: RuntimeResourceLimits) -> Self {
        let (fetch_tx, fetch_rx) = mpsc::channel();
        Self {
            renderer_ptr: None,
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
            params: VersionedSnapshotCache::new(ParamsSnapshot::new(BTreeMap::new())),
            current_lifecycle: Lifecycle::Idle,
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
            next_http_request_id: 1,
            fetch_interceptor: None,
            fetch_observer: None,
            fetch_keys: HashMap::new(),
            fetch_agent: crate::runtime::build_fetch_agent(),
            record_events: false,
            event_fixtures: None,
            recorded_events: Vec::new(),
            system: VersionedSnapshotCache::new(SystemSnapshot::default()),
            last_timings: FrameTimings::default(),
            taffy: TaffyTree::with_capacity(64),
            resource_limits,
            rng_state: None, // None = auto-seed on first use (from monotonic_ms)
            shut_down: false,
            #[cfg(feature = "testing")]
            unload_ran: false,
            #[cfg(feature = "testing")]
            delivered_events: 0,
            #[cfg(feature = "testing")]
            mdns_captured_events: Vec::new(),
            led_command_sender: None,
            audio: AudioRegistry::new(),
            guest_id: GuestId::alloc(),
            #[cfg(feature = "audio")]
            audio_stream: {
                match rodio::OutputStream::try_default() {
                    Ok((stream, handle)) => Some((stream, handle)),
                    Err(e) => {
                        tracing::warn!(
                            "audio output unavailable: {e} — audio_play will be a no-op"
                        );
                        None
                    }
                }
            },
            widget_width: 0,
            widget_height: 0,
            viewport_shape: bmc_wasm_protocol::ViewportShape::Rectangular,
            display_width: 0,
            display_height: 0,
            display_shape: bmc_wasm_protocol::DisplayShape::Rectangular,
            display_dpi: 0,
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

    /// Evict every host-side audio asset (sample + matching playback sinks)
    /// whose tag starts with `prefix`. Returns the number of evicted entries.
    ///
    /// Renderer-side eviction is the caller's responsibility — the host
    /// import (`host_evict_prefix`) reaches the `FemtoVgRenderer` through
    /// `WasmWidgetRuntime::with_renderer` and adds its count on top of
    /// this one.
    pub fn evict_audio_prefix(&mut self, prefix: &str) -> usize {
        self.audio.evict_prefix(prefix)
    }

    /// Wrap a guest-supplied tag with this instance's `GuestId` prefix.
    /// Every host-side asset registration and eviction goes through this
    /// helper, so two `WasmWidgetRuntime`s for the same widget can use the
    /// same slot names without collision.
    #[must_use]
    pub fn namespaced_tag(&self, tag: &str) -> String {
        format!("{}:{tag}", self.guest_id)
    }

    /// Evict every host-side audio asset belonging to this widget instance.
    /// Returns the number of evicted entries.
    /// Used as the audio-side safety sweep in `WasmWidgetRuntime`'s `Drop`;
    /// renderer-side assets are reclaimed when the caller drops their
    /// caller-owned `FemtoVgRenderer`.
    pub fn evict_widget(&mut self) -> usize {
        let prefix = self.guest_id.to_string();
        self.evict_audio_prefix(&prefix)
    }

    pub(crate) fn shutdown_workers(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;

        for browse in self.mdns_browses.values() {
            let _ = browse.stop_tx.send(());
        }
        for search in self.ssdp_searches.values() {
            let _ = search.stop_tx.send(());
        }
        for broadcast in self.udp_broadcasts.values() {
            let _ = broadcast.stop_tx.send(());
        }
        for listener in self.http_listeners.values() {
            let _ = listener.stop_tx.send(());
        }

        self.mdns_browses.clear();
        self.ssdp_searches.clear();
        self.udp_broadcasts.clear();
        self.http_listeners.clear();
        self.websockets.clear();
        self.sockets.clear();
        self.http_response_txs.clear();
        self.mdns_registrations.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::FrameScheduleState;

    fn schedule(animation_cadence_ms: u32) -> FrameScheduleState {
        let mut s = FrameScheduleState::new();
        s.animation_frame_delay_ms = animation_cadence_ms;
        s
    }

    #[test]
    fn pending_interaction_forces_immediate_wake() {
        let mut s = schedule(33);
        s.widget_delay_ms = Some(1_000);
        s.has_active_animations = false;
        s.interaction_pending = true;
        assert_eq!(s.effective_delay_ms(), Some(0));
    }

    #[test]
    fn pending_interaction_outranks_animation_cadence() {
        let mut s = schedule(33);
        s.has_active_animations = true;
        s.interaction_pending = true;
        assert_eq!(s.effective_delay_ms(), Some(0));
    }

    #[test]
    fn animation_caps_widget_delay() {
        let mut s = schedule(33);
        s.widget_delay_ms = Some(1_000);
        s.has_active_animations = true;
        assert_eq!(s.effective_delay_ms(), Some(33));
    }

    #[test]
    fn shorter_widget_delay_wins_over_animation_cap() {
        let mut s = schedule(33);
        s.widget_delay_ms = Some(16);
        s.has_active_animations = true;
        assert_eq!(s.effective_delay_ms(), Some(16));
    }

    #[test]
    fn animation_alone_uses_cadence() {
        let mut s = schedule(33);
        s.has_active_animations = true;
        assert_eq!(s.effective_delay_ms(), Some(33));
    }

    #[test]
    fn idle_widget_request_passes_through() {
        let mut s = schedule(33);
        s.widget_delay_ms = Some(1_000);
        assert_eq!(s.effective_delay_ms(), Some(1_000));
    }

    #[test]
    fn no_constraints_returns_none() {
        let s = schedule(33);
        assert_eq!(s.effective_delay_ms(), None);
    }

    #[test]
    fn wants_next_frame_reflects_any_input() {
        let s = schedule(33);
        assert!(!s.wants_next_frame(), "no inputs → no frame wanted");

        let mut s = schedule(33);
        s.widget_delay_ms = Some(100);
        assert!(s.wants_next_frame());

        let mut s = schedule(33);
        s.has_active_animations = true;
        assert!(s.wants_next_frame());

        let mut s = schedule(33);
        s.interaction_pending = true;
        assert!(s.wants_next_frame());
    }

    #[test]
    fn animation_only_requires_animations_and_no_widget_or_interaction() {
        let mut s = schedule(33);
        s.has_active_animations = true;
        assert!(s.is_animation_only_frame());

        let mut s = schedule(33);
        s.has_active_animations = true;
        s.interaction_pending = true;
        assert!(!s.is_animation_only_frame(), "interaction needs full WASM");

        let mut s = schedule(33);
        s.has_active_animations = true;
        s.widget_delay_ms = Some(100);
        assert!(
            !s.is_animation_only_frame(),
            "widget request needs full WASM"
        );

        let s = schedule(33);
        assert!(
            !s.is_animation_only_frame(),
            "no animations → not even animatable"
        );
    }
}
