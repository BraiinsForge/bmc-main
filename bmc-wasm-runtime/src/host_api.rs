// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host state and function bindings for WASM.

use std::collections::HashMap;
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

/// A completed HTTP fetch response ready for delivery to WASM.
pub struct CompletedFetch {
    pub request_id: u32,
    pub status: u32,
    pub body: Vec<u8>,
}

/// A delayed fetch waiting for its fire time.
pub struct DelayedFetch {
    pub fire_at: Instant,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub request_id: u32,
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

    /// Button clicks from last tree render (for new tree API)
    pub tree_clicks: Vec<bool>,

    /// Modal dialog states (keyed by modal_id)
    pub modal_states: HashMap<u16, ModalState>,

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

    /// Next request ID for fetch.
    pub next_request_id: u32,

    /// Receiver for completed fetch responses from background threads.
    pub fetch_rx: mpsc::Receiver<CompletedFetch>,

    /// Sender cloned into each background fetch thread.
    pub fetch_tx: mpsc::Sender<CompletedFetch>,

    /// Pending delayed fetches.
    pub delayed_fetches: Vec<DelayedFetch>,

    /// Parsed JSON documents, keyed by doc_id.
    pub json_docs: HashMap<u32, Value>,

    /// Next JSON document ID.
    pub next_json_id: u32,

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
            tree_clicks: Vec::new(),
            modal_states: HashMap::new(),
            delta_ms: 0,
            animation_states: HashMap::new(),
            transition_states: HashMap::new(),
            frame_counter: 0,
            cached_tree: None,
            animation_only_frame: false,
            next_request_id: 1,
            fetch_rx,
            fetch_tx,
            delayed_fetches: Vec::new(),
            json_docs: HashMap::new(),
            next_json_id: 1,
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
