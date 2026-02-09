// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host state and function bindings for WASM.

use std::collections::HashMap;

use crate::gpu::FemtoVgRenderer;
use crate::interaction::InteractionState;

/// State for a single running animation instance.
#[derive(Debug, Clone)]
pub struct AnimationState {
    pub elapsed_ms: u32,
    /// Current direction for PingPong (true = forward).
    pub forward: bool,
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
    /// Y position where drag started
    pub drag_start_y: i32,
    /// Scroll offset when drag started
    pub drag_start_offset: f32,
}

/// Host-side state accessible to WASM via host functions.
#[expect(dead_code)]
pub struct HostState {
    /// GPU renderer (FemtoVG + cosmic-text)
    pub renderer: FemtoVgRenderer,

    /// Interaction state (hit testing, pending clicks)
    pub interaction: InteractionState,

    /// Server-provided state blob
    pub state_blob: Option<Vec<u8>>,

    /// Whether `request_frame()` was called this frame
    pub frame_requested: bool,

    /// Delay from `request_frame_after(ms)`, if called
    pub frame_delay_ms: Option<u32>,

    /// Whether to request server refresh
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

    /// Cached tree data for animation-only frames (bytes, width, height).
    pub cached_tree_data: Option<(Vec<u8>, f32, f32)>,

    /// Whether the next frame only needs animation updates (no WASM execution).
    pub animation_only_frame: bool,
}

impl HostState {
    /// Create new host state with the given renderer.
    pub fn new(renderer: FemtoVgRenderer) -> Self {
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
            cached_tree_data: None,
            animation_only_frame: false,
        }
    }

    /// Reset per-frame flags.
    pub fn begin_render_frame(&mut self) {
        self.frame_requested = false;
        self.frame_delay_ms = None;
    }
}
