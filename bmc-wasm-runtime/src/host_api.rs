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

//! Host state and function bindings for WASM.
//!
//! Animation/transition draw-state (`PrevDrawValues`, `AnimationState`,
//! `TransitionState`, `ModalState`, `ScrollState`) lives in [`bmc_render`]
//! together with the renderer that owns it; this module imports those types.
//! See `bmc_render`'s crate-level docs for the rationale on `glam` being part
//! of the host-side public surface.

use chrono::{DateTime, FixedOffset};
use std::collections::{BTreeMap, HashMap, HashSet};
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
    AudioId, FetchOutcome, FetchRequestId, HttpListenerId, HttpRequestId, ImageJobId, JsonId,
    MdnsBrowseId, MdnsRegId, SocketId, SsdpSearchId, UdpBroadcastId, WebsocketId, XmlId,
};

use crate::audio_registry::AudioRegistry;
use crate::network::NetworkInfo;
use crate::runtime::{CredentialView, ParamsSnapshot};
use crate::runtime_limits::RuntimeResourceLimits;
use crate::system::SystemSnapshot;
use crate::xml::XmlDocumentIndex;
use bmc_wasm_protocol::versioned_snapshot::VersionedSnapshotCache;

/// Refused live-I/O attempts collected during a hermetic capture run.
#[derive(Default)]
pub struct HermeticRun {
    /// Each a `"<kind>: <target>"`, e.g. `"fetch: GET https://…"`.
    pub breaches: Vec<String>,
}

impl HermeticRun {
    /// Record a refused live-I/O attempt. `kind` is the egress class
    /// (`"fetch"`, `"websocket"`, …); `target` a human-readable destination.
    pub fn record(&mut self, kind: &str, target: &str) {
        self.breaches.push(format!("{kind}: {target}"));
    }
}

/// A completed HTTP fetch response ready for delivery to WASM.
pub struct CompletedFetch {
    pub request_id: FetchRequestId,
    pub status: u32,
    pub body: Vec<u8>,
}

/// The fetches a widget has outstanding, and the accounting that bounds them.
///
/// One rule holds that accounting together.
/// [`FetchState::accept`] records a request and hands back the channel that
/// settles it, and every answer travels that channel —
/// whether a transport thread produced it or the host wrote it directly,
/// as it does for a refusal.
/// A slot is released only in [`FetchState::drain_settled`],
/// so the set cannot drift from what the widget was told.
pub struct FetchState {
    /// Cloned into each background fetch thread,
    /// and used directly when the host settles a request itself.
    settle_tx: mpsc::Sender<CompletedFetch>,
    settle_rx: mpsc::Receiver<CompletedFetch>,
    /// Queued by `host_fetch_after`, still waiting for its firing time.
    delayed: Vec<DelayedFetch>,
    /// Accepted and not yet drained. Held by id, not counted: a cancel names
    /// one request, and only the ids can say whether it names a real one.
    in_flight: HashSet<FetchRequestId>,
    /// In-flight requests the widget cancelled: their settlements are
    /// rewritten to [`FetchOutcome::Aborted`], never delivered as data.
    /// In-flight only, so the fetch limit bounds this set too.
    cancelled: HashSet<FetchRequestId>,
}

/// What [`FetchState::cancel`] found to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum CancelDisposition {
    /// Removed from the queue before it ran: gone, slot freed, nothing owed.
    Stopped,
    /// Already away: its own reply settles as [`FetchOutcome::Aborted`].
    WillAbort,
    /// Neither queued nor awaiting settlement: nothing to act on.
    Unknown,
}

impl FetchState {
    fn new() -> Self {
        let (settle_tx, settle_rx) = mpsc::channel();
        Self {
            settle_tx,
            settle_rx,
            delayed: Vec::new(),
            in_flight: HashSet::new(),
            cancelled: HashSet::new(),
        }
    }

    /// Record a request the host has accepted, and hand back its settling
    /// channel. Dropping that channel without sending holds the slot until
    /// the runtime goes away.
    pub fn accept(&mut self, request_id: FetchRequestId) -> mpsc::Sender<CompletedFetch> {
        self.in_flight.insert(request_id);
        self.settle_tx.clone()
    }

    /// Settlements delivered since the last drain, each releasing its slot.
    pub fn drain_settled(&mut self) -> Vec<CompletedFetch> {
        let mut settled = Vec::new();
        while let Ok(mut response) = self.settle_rx.try_recv() {
            self.in_flight.remove(&response.request_id);
            if self.cancelled.remove(&response.request_id) {
                response.status = FetchOutcome::Aborted.to_wire();
                response.body.clear();
            }
            settled.push(response);
        }
        settled
    }

    /// Cancel a request. Freeing a queued one's slot here, not at the drain,
    /// is what lets a caller cancel and re-send within one guest call.
    pub fn cancel(&mut self, request_id: FetchRequestId) -> CancelDisposition {
        let before = self.delayed.len();
        self.delayed.retain(|fetch| fetch.request_id != request_id);
        if self.delayed.len() != before {
            return CancelDisposition::Stopped;
        }
        if self.in_flight.contains(&request_id) {
            self.cancelled.insert(request_id);
            return CancelDisposition::WillAbort;
        }
        CancelDisposition::Unknown
    }

    pub fn queue_delayed(&mut self, fetch: DelayedFetch) {
        self.delayed.push(fetch);
    }

    /// The delayed queue: fire the requests whose time has come,
    /// or drop one the widget cancelled before it ran.
    pub fn delayed_mut(&mut self) -> &mut Vec<DelayedFetch> {
        &mut self.delayed
    }

    /// Whether anything is owed: a queued request, or one awaiting settlement.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.delayed.is_empty() || !self.in_flight.is_empty()
    }

    /// Slots a widget is holding, against `max_fetches`.
    #[must_use]
    pub fn slots_used(&self) -> usize {
        self.delayed.len().saturating_add(self.in_flight.len())
    }

    /// A sender for a settlement no request is waiting on — teardown tests
    /// keep one alive to prove a dropped runtime hangs up on its threads.
    #[cfg(feature = "testing")]
    pub fn test_settle_sender(&self) -> mpsc::Sender<CompletedFetch> {
        self.settle_tx.clone()
    }
}

/// A completed off-thread image decode, ready for GPU upload and delivery.
#[derive(Debug)]
pub enum CacheWriteOutcome {
    Stored,
    Failed(String),
    Disabled,
}

pub struct CompletedImageDecode {
    pub job_id: ImageJobId,
    pub raw_tag: String,
    /// Registry tag (slot-namespaced) for the GPU texture upload.
    pub tag: String,
    pub result: Result<(Vec<u8>, u32, u32), String>,
    pub cache_write: CacheWriteOutcome,
    pub decode_us: u64,
}

#[cfg(feature = "testing")]
impl CompletedFetch {
    pub fn test_sentinel() -> Self {
        Self {
            request_id: bmc_wasm_protocol::FetchRequestId::from_wire(1)
                .expect("BUG: 1 is non-zero so from_wire returns Some"),
            status: bmc_wasm_protocol::FetchOutcome::Network.to_wire(),
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
    LedSetEndless {
        effect: u8,
        r: u8,
        g: u8,
        b: u8,
        period_ms: u32,
        scope: u8,
    },
    LedSetTemporary {
        effect: u8,
        r: u8,
        g: u8,
        b: u8,
        period_ms: u32,
        duration_ms: u32,
        scope: u8,
    },
    LedStop,
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
    /// `on_credentials_update` is on the stack.
    /// Shares [`Self::ParamsUpdate`]'s import surface,
    /// and is separate for the same reason.
    CredentialsUpdate,
    /// `on_touch` is on the stack. Fired once per Wayland drain that delivered
    /// touch activity. `request_frame` is legal (and is how the widget asks to
    /// re-render in response); submitting a tree and touch readback are not —
    /// the queued touch is consumed at the next render, not here.
    Touch,
    /// `on_network_update` is on the stack — the Deck's SSID or IP changed.
    /// Shares [`Self::Touch`]'s import surface:
    /// the widget re-reads `host_network_info` and decides via `request_frame`
    /// whether to repaint.
    NetworkUpdate,
    /// `on_sleep` is on the stack — release off-scene resources.
    Sleep,
    /// `on_wake` is on the stack — rebuild guest state; `request_frame` is legal.
    Wake,
    /// `unload` is on the stack. Synchronous cleanup only; frame requests no-op.
    Unload,
}

/// Fallback id when no compositor token is supplied (testbed/capture harness).
fn synthetic_instance_id() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    format!("dev-{}", NEXT.fetch_add(1, Ordering::Relaxed))
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
#[derive(Debug, Clone)]
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

    /// Boundary-cadence wake for host-rendered time nodes; uncapped by animations.
    pub host_frame_delay_ms: Option<u32>,

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
            host_frame_delay_ms: None,
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
        self.host_frame_delay_ms = None;
    }

    /// Request a full-WASM render after `delay_ms` (monotonic `now`).
    /// Soonest-wins: a frame already requested sooner is left alone,
    /// so a later delayed request can't postpone a pending render.
    pub fn request_frame_after(&mut self, delay_ms: u32, now: u64) {
        if self
            .widget_delay_ms
            .is_some_and(|pending| pending <= delay_ms)
        {
            return;
        }
        self.widget_delay_ms = Some(delay_ms);
        self.deferred_wasm_render_at_ms = Some(now + u64::from(delay_ms));
    }

    /// Whether anything wants the host to render a next frame.
    pub fn wants_next_frame(&self) -> bool {
        self.widget_delay_ms.is_some()
            || self.has_active_animations
            || self.interaction_pending
            || self.host_frame_delay_ms.is_some()
    }

    /// Whether the next frame can replay the cached tree without running WASM.
    pub fn is_animation_only_frame(&self) -> bool {
        (self.has_active_animations || self.host_frame_delay_ms.is_some())
            && !self.interaction_pending
            && self.widget_delay_ms.is_none()
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
        [self.widget_delay_ms, cap, self.host_frame_delay_ms]
            .into_iter()
            .flatten()
            .min()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RendererAssetGate {
    Active,
    Dormant,
}

/// Host-side state accessible to WASM via host functions.
pub(crate) struct HostState {
    /// Renderer parked by `WasmWidgetRuntime::with_renderer` for the duration of a
    /// render scope. `None` outside a render scope; host imports that read this must
    /// trap the guest with `wasmi::Error::new("renderer accessed outside render scope")`
    /// rather than panic the host.
    pub renderer_ptr: Option<NonNull<dyn Renderer>>,

    renderer_asset_gate: RendererAssetGate,

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

    pub(crate) cached_tree_asset_references: Option<bmc_render::tree::RendererAssetReferences>,

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

    pub fetches: FetchState,

    /// Receiver for completed off-thread image decodes.
    pub image_decode_rx: mpsc::Receiver<CompletedImageDecode>,

    /// Sender cloned into each background image-decode thread.
    pub image_decode_tx: mpsc::Sender<CompletedImageDecode>,

    /// Worker results awaiting a renderer-backed delivery scope.
    pub completed_image_decodes: Vec<CompletedImageDecode>,

    /// Next image-decode job id counter (for `ImageJobId::alloc`).
    pub next_image_job_id: u32,

    /// Number of image decodes currently in flight.
    pub in_flight_image_decodes: u32,

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

    /// Base-URL rewrites `(from_prefix, to_prefix)`; the first matching
    /// prefix maps the URL at the last hop, ahead of secret substitution
    /// and the egress check — dev plumbing that points a widget's
    /// hard-coded API base at a simulator. Set via `RuntimeConfig`.
    pub url_rewrites: Vec<(String, String)>,

    /// Hermetic-run state; `None` is a normal run. Set via `RuntimeConfig`.
    pub hermetic: Option<HermeticRun>,

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

    /// The Deck's own SSID + IP, read on demand by the `host_network_info` getter.
    /// Unversioned: the embedder stores the new value first, then fires
    /// `on_network_update`, which re-reads it through the getter.
    pub network_info: NetworkInfo,

    /// Third channel of the same shape as `params` and `system`.
    pub credentials: VersionedSnapshotCache<CredentialView>,

    /// The secret values behind those slots.
    ///
    /// Deliberately a bare field — no version, no cache, no encoder,
    /// because no import may hand these to the guest.
    /// The runtime spends them itself when substituting
    /// into an outbound request.
    pub credential_secrets: bmc_widget_protocol::CredentialSecrets,

    /// Per-frame timing breakdown from the last rendered frame.
    pub last_timings: FrameTimings,

    /// Fuel charged per profiling section in the current frame; drained per frame.
    /// `BTreeMap` keeps the report's section order deterministic.
    pub profile_sections: BTreeMap<String, u64>,

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

    /// Sender for widget-perspective LED requests. `None` when LED control
    /// is unavailable.
    pub led_request_sender: Option<mpsc::Sender<crate::led_request::LedRequest>>,

    /// Per-guest allocator for non-zero LED request ids.
    pub led_request_alloc: crate::led_request::LedRequestIdAllocator,

    /// Registered audio samples + tag dedup + active playback sinks.
    /// The host stores original encoded data and decodes on each play.
    pub audio: AudioRegistry,

    /// Namespaces every host-side asset tag — the compositor-minted instance
    /// token, or a synthetic `dev-N` for the testbed/capture harness.
    pub instance_id: String,

    /// Per-instance asset cache, curried to this widget's bucket.
    pub asset_cache: Option<crate::disk_cache::DiskCache>,

    /// Per-widget immutable source for package-backed assets.
    pub package_assets: Option<crate::PackageAssetStore>,

    pub(crate) renderer_assets: crate::renderer_assets::RendererAssetLedger,

    pub(crate) renderer_asset_failure: Option<String>,

    pub(crate) last_asset_restoration: Option<crate::runtime::RendererAssetRestorationObservation>,

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
    pub(crate) fn renderer_asset_registration_matches(
        &self,
        raw_tag: &str,
        kind: crate::renderer_assets::RendererAssetKind,
        backing: &crate::renderer_assets::AssetBacking,
    ) -> bool {
        let Some(existing) = self.renderer_assets.get(raw_tag) else {
            return true;
        };
        if existing.kind == kind && existing.backing.can_transition_to(backing) {
            return true;
        }
        tracing::warn!(
            instance_id = %self.instance_id,
            tag = %raw_tag,
            requested_kind = ?kind,
            requested_backing = ?backing,
            recorded_kind = ?existing.kind,
            recorded_backing = ?existing.backing,
            "renderer asset registration rejected: tag already has an incompatible registration"
        );
        false
    }

    pub(crate) fn record_renderer_asset(
        &mut self,
        raw_tag: String,
        kind: crate::renderer_assets::RendererAssetKind,
        id: crate::renderer_assets::RendererAssetId,
        backing: crate::renderer_assets::AssetBacking,
    ) -> bool {
        self.renderer_assets
            .record(
                raw_tag,
                crate::renderer_assets::RendererAssetRecord {
                    kind,
                    id,
                    demand_restoration: if backing.is_restorable() {
                        crate::renderer_assets::DemandRestoration::Pending
                    } else {
                        crate::renderer_assets::DemandRestoration::Unavailable
                    },
                    backing,
                },
            )
            .is_ok()
    }

    pub(crate) fn mark_renderer_assets_dormant(&mut self) {
        self.renderer_asset_gate = RendererAssetGate::Dormant;
    }

    pub(crate) fn mark_renderer_assets_active(&mut self) {
        self.renderer_asset_gate = RendererAssetGate::Active;
    }

    pub(crate) fn renderer_assets_are_dormant(&self) -> bool {
        self.renderer_asset_gate != RendererAssetGate::Active
    }

    /// In a hermetic run, record a breach and return `true`; else `false`.
    /// Call sites: `if state.refuse_live_io(kind, target) { return reject; }`.
    pub(crate) fn refuse_live_io(&mut self, kind: &str, target: &str) -> bool {
        self.hermetic
            .as_mut()
            .map(|run| run.record(kind, target))
            .is_some()
    }

    /// Create new host state with default `system` / `params` snapshots
    /// (version 0). The runtime constructor stages the operator-supplied
    /// initial snapshots via `.replace(...)` to bump both to version 1
    /// before `init()` runs.
    ///
    /// The renderer is owned by the caller of [`crate::WasmWidgetRuntime::new`]
    /// and installed on `renderer_ptr` per-frame via
    /// `WasmWidgetRuntime::with_renderer`.
    #[expect(
        clippy::too_many_lines,
        reason = "constructor initializes every independent host service and runtime registry"
    )]
    pub fn new(resource_limits: RuntimeResourceLimits, system_time: DateTime<FixedOffset>) -> Self {
        let (image_decode_tx, image_decode_rx) = mpsc::channel();
        Self {
            renderer_ptr: None,
            renderer_asset_gate: RendererAssetGate::Active,
            renderer_assets: crate::renderer_assets::RendererAssetLedger::default(),
            renderer_asset_failure: None,
            last_asset_restoration: None,
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
            cached_tree_asset_references: None,
            system_time,
            monotonic_ms: 0,
            params: VersionedSnapshotCache::new(ParamsSnapshot::new(BTreeMap::new())),
            current_lifecycle: Lifecycle::Idle,
            next_request_id: 1,
            fetches: FetchState::new(),
            image_decode_rx,
            image_decode_tx,
            completed_image_decodes: Vec::new(),
            next_image_job_id: 1,
            in_flight_image_decodes: 0,
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
            url_rewrites: Vec::new(),
            hermetic: None,
            fetch_observer: None,
            fetch_keys: HashMap::new(),
            fetch_agent: crate::runtime::build_fetch_agent(),
            record_events: false,
            event_fixtures: None,
            recorded_events: Vec::new(),
            system: VersionedSnapshotCache::new(SystemSnapshot::default()),
            network_info: NetworkInfo::default(),
            credentials: VersionedSnapshotCache::new(CredentialView::default()),
            credential_secrets: bmc_widget_protocol::CredentialSecrets::default(),
            last_timings: FrameTimings::default(),
            profile_sections: BTreeMap::new(),
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
            led_request_sender: None,
            led_request_alloc: crate::led_request::LedRequestIdAllocator::new(),
            audio: AudioRegistry::new(),
            instance_id: synthetic_instance_id(),
            asset_cache: None,
            package_assets: None,
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
        self.fetches.slots_used()
    }

    /// Accumulate `us` wall-clock micros into the named profile section. The
    /// `_us` name marks the unit apart from the guest fuel sections.
    pub fn add_profile_us(&mut self, name: &str, us: u64) {
        *self.profile_sections.entry(name.to_owned()).or_default() += us;
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

    /// Wrap a guest-supplied tag with this instance's id prefix.
    /// Every host-side asset registration and eviction goes through this
    /// helper, so two `WasmWidgetRuntime`s for the same widget can use the
    /// same slot names without collision.
    #[must_use]
    pub fn namespaced_tag(&self, tag: &str) -> String {
        format!("{}:{tag}", self.instance_id)
    }

    /// The bare namespace root (`instance_id`) every asset tag lives under.
    /// `evict_prefix(instance_namespace())` sweeps the whole bucket.
    #[must_use]
    pub fn instance_namespace(&self) -> &str {
        &self.instance_id
    }

    /// Evict every host-side audio asset belonging to this widget instance.
    /// Returns the number of evicted entries.
    /// Used as the audio-side safety sweep in `WasmWidgetRuntime`'s `Drop`;
    /// renderer-side assets are reclaimed when the caller drops their
    /// caller-owned `FemtoVgRenderer`.
    pub fn evict_widget(&mut self) -> usize {
        let prefix = self.instance_id.clone();
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
    use super::{
        CancelDisposition, DelayedFetch, FetchState, FrameScheduleState, HermeticRun, HostState,
        RendererAssetGate,
    };
    use crate::runtime_limits::RuntimeResourceLimits;
    use bmc_wasm_protocol::FetchRequestId;

    /// A cancel names one request, and an id naming none can never settle.
    /// Remembering it would grow `cancelled` for as long as the widget runs,
    /// unnoticed by the fetch limit, which counts slots rather than cancels.
    #[test]
    fn cancelling_an_unknown_id_is_refused_and_remembers_nothing() {
        // Ids start at 1; zero is the wire's "no request".
        let mut counter = 1;
        let mut fetches = FetchState::new();
        let real = FetchRequestId::alloc(&mut counter);
        let _settle = fetches.accept(real);

        let unknown = FetchRequestId::alloc(&mut counter);
        assert_eq!(
            fetches.cancel(unknown),
            CancelDisposition::Unknown,
            "an id naming no request reports nothing to act on"
        );
        assert!(
            fetches.cancelled.is_empty(),
            "and leaves nothing behind to remember it by"
        );
        assert_eq!(fetches.slots_used(), 1, "nor does it take a fetch slot");

        assert_eq!(
            fetches.cancel(real),
            CancelDisposition::WillAbort,
            "an in-flight request cannot be stopped, only rewritten"
        );
        assert_eq!(
            fetches.cancelled.len(),
            1,
            "and it is remembered, since its own reply is still coming"
        );
    }

    /// The slot must free within the very call — a cancelling caller
    /// re-sends before the host can drain.
    #[test]
    fn cancelling_a_queued_fetch_frees_its_slot_and_owes_nothing() {
        let mut counter = 1;
        let mut fetches = FetchState::new();
        let queued = FetchRequestId::alloc(&mut counter);
        fetches.queue_delayed(DelayedFetch {
            fire_at_ms: 1_000,
            method: "GET".to_owned(),
            url: "https://example.test/delayed".to_owned(),
            headers: Vec::new(),
            body: None,
            timeout: std::time::Duration::from_secs(10),
            request_id: queued,
        });
        assert_eq!(fetches.slots_used(), 1, "a queued request reserves a slot");

        assert_eq!(fetches.cancel(queued), CancelDisposition::Stopped);
        assert_eq!(fetches.slots_used(), 0, "the reservation frees immediately");
        assert!(
            fetches.cancelled.is_empty(),
            "nothing will settle, so nothing is owed or remembered"
        );
        assert!(
            fetches.drain_settled().is_empty(),
            "and no settlement was manufactured for it"
        );
    }

    #[test]
    fn renderer_asset_gate_tracks_committed_renderability() {
        let mut state = HostState::new(
            RuntimeResourceLimits::default(),
            chrono::Local::now().fixed_offset(),
        );

        state.mark_renderer_assets_dormant();
        assert!(state.renderer_assets_are_dormant());
        assert_eq!(state.renderer_asset_gate, RendererAssetGate::Dormant);

        state.mark_renderer_assets_active();
        assert!(!state.renderer_assets_are_dormant());
        assert_eq!(state.renderer_asset_gate, RendererAssetGate::Active);
    }

    #[test]
    fn refuse_live_io_records_only_in_a_hermetic_run() {
        let mut state = HostState::new(
            RuntimeResourceLimits::default(),
            chrono::Local::now().fixed_offset(),
        );
        // Not hermetic: the call is a no-op and reports "proceed".
        assert!(!state.refuse_live_io("fetch", "GET https://x/y"));
        assert!(state.hermetic.is_none());

        // Hermetic: the call refuses and records the breach with kind + target.
        state.hermetic = Some(HermeticRun::default());
        assert!(state.refuse_live_io("fetch", "GET https://x/y"));
        assert!(state.refuse_live_io("websocket", "wss://z"));
        assert_eq!(
            state.hermetic.as_ref().expect("BUG: set above").breaches,
            ["fetch: GET https://x/y", "websocket: wss://z"]
        );
    }

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
    fn request_frame_after_is_soonest_wins() {
        // A tap's pending immediate frame is not downgraded
        // by a poll's later, delayed request.
        let mut s = FrameScheduleState::new();
        s.widget_delay_ms = Some(0);
        s.request_frame_after(500, 1_000);
        assert_eq!(s.widget_delay_ms, Some(0));

        let mut s = FrameScheduleState::new();
        s.request_frame_after(500, 1_000);
        assert_eq!(s.widget_delay_ms, Some(500));
        assert_eq!(s.deferred_wasm_render_at_ms, Some(1_500));
        s.request_frame_after(100, 2_000);
        assert_eq!(s.widget_delay_ms, Some(100));
        assert_eq!(s.deferred_wasm_render_at_ms, Some(2_100));
        s.request_frame_after(900, 3_000);
        assert_eq!(s.widget_delay_ms, Some(100), "a later request is ignored");
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

    #[test]
    fn host_frame_delay_wakes_at_its_boundary_uncapped() {
        let mut s = schedule(33);
        s.host_frame_delay_ms = Some(1_000);
        assert_eq!(s.effective_delay_ms(), Some(1_000));
        assert!(s.wants_next_frame());
        assert!(
            s.is_animation_only_frame(),
            "a time node replays the cached tree"
        );
    }

    #[test]
    fn animation_cadence_wins_over_host_frame_delay_when_both_present() {
        let mut s = schedule(33);
        s.host_frame_delay_ms = Some(1_000);
        s.has_active_animations = true;
        assert_eq!(s.effective_delay_ms(), Some(33));
    }
}
