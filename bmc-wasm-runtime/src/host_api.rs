// Copyright (C) 2025  Braiins Systems s.r.o.

//! Host state and function bindings for WASM.

use anyhow::Result;
use std::collections::HashMap;

use crate::drawing::text::ShapedTextCache;
use crate::interaction::InteractionState;

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
    /// RGBA8 overlay buffer
    pub pixmap: tiny_skia::Pixmap,

    /// Font system for text rendering
    pub font_system: cosmic_text::FontSystem,

    /// Swash cache for glyph rasterization
    pub swash_cache: cosmic_text::SwashCache,

    /// Interaction state (hit testing, pending clicks)
    pub interaction: InteractionState,

    /// Server-provided state blob
    pub state_blob: Option<Vec<u8>>,

    /// Registered images from state blob
    pub images: HashMap<u32, tiny_skia::Pixmap>,

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

    /// Cache for shaped text buffers
    pub text_cache: ShapedTextCache,
}

impl HostState {
    /// Create new host state with given dimensions.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| anyhow::anyhow!("Failed to create pixmap"))?;

        let font_system = cosmic_text::FontSystem::new();
        let swash_cache = cosmic_text::SwashCache::new();

        Ok(Self {
            pixmap,
            font_system,
            swash_cache,
            interaction: InteractionState::new(),
            state_blob: None,
            images: HashMap::new(),
            frame_requested: false,
            frame_delay_ms: None,
            refresh_requested: false,
            tree_clicks: Vec::new(),
            modal_states: HashMap::new(),
            delta_ms: 0,
            text_cache: ShapedTextCache::new(256), // Cache up to 256 shaped text entries
        })
    }

    /// Clear the overlay to transparent.
    pub fn clear_overlay(&mut self) {
        self.pixmap.fill(tiny_skia::Color::TRANSPARENT);
        // Note: begin_frame() is called separately in runtime.rs before this
        self.frame_requested = false;
        self.frame_delay_ms = None;
    }
}
