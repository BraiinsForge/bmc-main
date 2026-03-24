// Copyright (C) 2025  Braiins Systems s.r.o.

//! Interaction state for immediate-mode UI.

use std::collections::{HashMap, VecDeque};

use super::types::{Rect, TouchEvent};

/// Manages interaction state for immediate-mode UI pattern.
#[derive(Debug)]
pub struct InteractionState {
    /// Hit regions registered this frame (cleared each frame).
    hit_regions: HashMap<String, Rect>,

    /// Pending touch events.
    event_queue: VecDeque<TouchEvent>,

    /// Element where touch started (for click detection).
    touch_down_key: Option<String>,

    /// Pending click to be consumed by matching button().
    pending_click: Option<String>,

    /// Last touch position for drag tracking.
    last_touch_pos: Option<(i32, i32)>,

    /// Accumulated drag delta for the current frame (y-axis for scrolling).
    drag_delta_y: i32,
}

impl InteractionState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            hit_regions: HashMap::new(),
            event_queue: VecDeque::new(),
            touch_down_key: None,
            pending_click: None,
            last_touch_pos: None,
            drag_delta_y: 0,
        }
    }

    /// Clear hit regions for new frame.
    pub fn begin_frame(&mut self) {
        // Reset drag delta for new frame
        self.drag_delta_y = 0;

        // Process pending events BEFORE clearing hit regions
        // (events need to hit-test against previous frame's regions)
        while let Some(event) = self.event_queue.pop_front() {
            match event {
                TouchEvent::Down { x, y } => {
                    self.touch_down_key = self.hit_test(x, y);
                    self.last_touch_pos = Some((x, y));
                }
                TouchEvent::Up { x, y } => {
                    if let Some(down_key) = &self.touch_down_key
                        && self.hit_test(x, y).as_ref() == Some(down_key)
                    {
                        // Touch up on same element = click
                        self.pending_click = Some(down_key.clone());
                    }

                    self.touch_down_key = None;
                    self.last_touch_pos = None;
                }
                TouchEvent::Move { x, y } => {
                    // Track drag delta for scrolling
                    if let Some((last_x, last_y)) = self.last_touch_pos {
                        let _ = x - last_x; // dx (unused for now)
                        let dy = y - last_y;
                        self.drag_delta_y += dy;
                    }
                    self.last_touch_pos = Some((x, y));
                }
                TouchEvent::Scroll { delta_y, .. } => {
                    // Mouse wheel scroll
                    self.drag_delta_y += delta_y;
                }
            }
        }

        // Now clear hit regions for this frame
        self.hit_regions.clear();
    }

    /// Push a touch event to be processed.
    pub fn push_event(&mut self, event: TouchEvent) {
        self.event_queue.push_back(event);
    }

    /// Register a hit region and check if it was clicked.
    /// Returns true if this element was clicked (consumes the click).
    pub fn button(&mut self, key: &str, bounds: Rect) -> bool {
        // Register hit region for future hit testing
        self.hit_regions.insert(key.to_owned(), bounds);

        // Check and consume pending click
        if self.pending_click.as_deref() == Some(key) {
            self.pending_click = None;
            return true;
        }

        false
    }

    /// Check if a button is currently pressed (touch down on it).
    #[must_use]
    pub fn is_pressed(&self, key: &str) -> bool {
        self.touch_down_key.as_deref() == Some(key)
    }

    /// Get the accumulated scroll delta (y-axis) for this frame.
    /// Returns the delta if the touch/scroll started on the specified element.
    /// Positive = scroll down (content moves up), negative = scroll up.
    #[must_use]
    pub fn get_scroll_delta(&self, key: &str) -> i32 {
        // Return scroll delta if drag is happening on this element
        if self.touch_down_key.as_deref() == Some(key) || self.drag_delta_y != 0 {
            // For mouse wheel, we don't check the key since wheel events
            // should work when mouse is over any scroll region
            self.drag_delta_y
        } else {
            0
        }
    }

    /// Get the global scroll delta (for any scrollable region).
    #[must_use]
    pub fn get_global_scroll_delta(&self) -> i32 {
        self.drag_delta_y
    }

    /// Hit test against registered regions.
    fn hit_test(&self, x: i32, y: i32) -> Option<String> {
        for (key, rect) in &self.hit_regions {
            if rect.contains(x, y) {
                return Some(key.clone());
            }
        }
        None
    }
}

impl Default for InteractionState {
    fn default() -> Self {
        Self::new()
    }
}
