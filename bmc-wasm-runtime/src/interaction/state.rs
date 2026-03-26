// Copyright (C) 2026  Braiins Systems s.r.o.

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

    /// Click position (absolute) for the pending click.
    pending_click_pos: Option<(f32, f32)>,

    /// Last touch position for drag tracking.
    last_touch_pos: Option<(f32, f32)>,

    /// Accumulated drag delta for the current frame (y-axis for scrolling).
    drag_delta_y: f32,
}

impl InteractionState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            hit_regions: HashMap::new(),
            event_queue: VecDeque::new(),
            touch_down_key: None,
            pending_click: None,
            pending_click_pos: None,
            last_touch_pos: None,
            drag_delta_y: 0.0,
        }
    }

    /// Clear hit regions for new frame.
    pub fn begin_frame(&mut self) {
        // Reset drag delta for new frame
        self.drag_delta_y = 0.0;

        // Process pending events BEFORE clearing hit regions
        // (events need to hit-test against previous frame's regions)
        while let Some(event) = self.event_queue.pop_front() {
            match event {
                TouchEvent::Down { x, y } => {
                    self.touch_down_key = self.hit_test(x, y);
                    self.last_touch_pos = Some((x, y));
                }
                TouchEvent::Up => {
                    // Use last known position for hit-testing the release.
                    // wl_touch::up does not carry coordinates.
                    if let Some(down_key) = &self.touch_down_key
                        && let Some((ux, uy)) = self.last_touch_pos
                        && self.hit_test(ux, uy).as_ref() == Some(down_key)
                    {
                        self.pending_click = Some(down_key.clone());
                        self.pending_click_pos = Some((ux, uy));
                    }

                    self.touch_down_key = None;
                    self.last_touch_pos = None;
                }
                TouchEvent::Cancel => {
                    self.touch_down_key = None;
                    self.last_touch_pos = None;
                }
                TouchEvent::Move { x, y } => {
                    // Track drag delta for scrolling
                    if let Some((_last_x, last_y)) = self.last_touch_pos {
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

    /// Check if there are pending touch events to process.
    #[must_use]
    pub fn has_pending_events(&self) -> bool {
        !self.event_queue.is_empty()
    }

    /// Register a hit region and check if it was clicked.
    /// Returns true if this element was clicked (consumes the click).
    pub fn button(&mut self, key: &str, bounds: Rect) -> bool {
        self.button_with_pos(key, bounds).0
    }

    /// Register a hit region and check if it was clicked, returning the
    /// click position relative to `bounds` as `(local_x, local_y)`.
    pub fn button_with_pos(&mut self, key: &str, bounds: Rect) -> (bool, Option<(f32, f32)>) {
        // Register hit region for future hit testing
        self.hit_regions.insert(key.to_owned(), bounds);

        // Check and consume pending click
        if self.pending_click.as_deref() == Some(key) {
            self.pending_click = None;
            let pos = self
                .pending_click_pos
                .take()
                .map(|(cx, cy)| (cx - bounds.x, cy - bounds.y));
            return (true, pos);
        }

        (false, None)
    }

    /// Check if a button is currently pressed (touch down on it).
    #[must_use]
    pub fn is_pressed(&self, key: &str) -> bool {
        self.touch_down_key.as_deref() == Some(key)
    }

    /// Get the current drag position for an element (local to `bounds`).
    ///
    /// Returns `Some((local_x, local_y))` if the user is actively touching
    /// (finger down + moved) on the element identified by `key`.
    #[must_use]
    pub fn get_drag_pos(&self, key: &str, bounds: Rect) -> Option<(f32, f32)> {
        if self.touch_down_key.as_deref() == Some(key) {
            self.last_touch_pos
                .map(|(x, y)| (x - bounds.x, y - bounds.y))
        } else {
            None
        }
    }

    /// Get the accumulated scroll delta (y-axis) for this frame.
    /// Returns the delta if the touch/scroll started on the specified element.
    /// Positive = scroll down (content moves up), negative = scroll up.
    #[must_use]
    pub fn get_scroll_delta(&self, key: &str) -> f32 {
        // Return scroll delta if drag is happening on this element
        if self.touch_down_key.as_deref() == Some(key) || self.drag_delta_y != 0.0 {
            // For mouse wheel, we don't check the key since wheel events
            // should work when mouse is over any scroll region
            self.drag_delta_y
        } else {
            0.0
        }
    }

    /// Get the global scroll delta (for any scrollable region).
    #[must_use]
    pub fn get_global_scroll_delta(&self) -> f32 {
        self.drag_delta_y
    }

    /// Look up the bounds of a registered element by its string ID.
    ///
    /// Returns `None` if no element with that ID was registered this frame.
    /// Note: hit regions are cleared at the start of each frame, so this must
    /// be called after a render pass.
    #[must_use]
    pub fn element_bounds(&self, id: &str) -> Option<Rect> {
        self.hit_regions.get(id).copied()
    }

    /// Return all registered hit region element IDs (sorted).
    #[must_use]
    pub fn element_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.hit_regions.keys().map(String::as_str).collect();
        ids.sort_unstable();
        ids
    }

    /// Hit test against registered regions.
    ///
    /// Returns the smallest (most specific) region containing the point.
    /// This ensures buttons inside scroll containers win over the scroll
    /// container's own hit region.
    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> Option<String> {
        let mut best: Option<(&str, f32)> = None;
        for (key, rect) in &self.hit_regions {
            if rect.contains(x, y) {
                let area = rect.area();
                if best.as_ref().is_none_or(|(_, best_area)| area < *best_area) {
                    best = Some((key, area));
                }
            }
        }
        best.map(|(key, _)| key.to_owned())
    }
}

impl Default for InteractionState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::InteractionState;
    use crate::interaction::{Rect, TouchEvent};

    #[test]
    fn coordinate_less_up_uses_last_touch_position_for_click() {
        let bounds = Rect::new(50.0, 20.0, 100.0, 40.0);
        let mut state = InteractionState::new();

        assert!(!state.button("btn", bounds));

        state.push_event(TouchEvent::Down { x: 80.0, y: 35.0 });
        state.push_event(TouchEvent::Move { x: 90.0, y: 40.0 });
        state.push_event(TouchEvent::Up);
        state.begin_frame();

        let (clicked, pos) = state.button_with_pos("btn", bounds);
        assert!(clicked);
        assert_eq!(pos, Some((40.0, 20.0)));
    }

    #[test]
    fn cancel_clears_pressed_state_without_emitting_click() {
        let bounds = Rect::new(50.0, 20.0, 100.0, 40.0);
        let mut state = InteractionState::new();

        assert!(!state.button("btn", bounds));

        state.push_event(TouchEvent::Down { x: 80.0, y: 35.0 });
        state.push_event(TouchEvent::Move { x: 90.0, y: 40.0 });
        state.push_event(TouchEvent::Cancel);
        state.begin_frame();

        assert!(!state.is_pressed("btn"));
        assert_eq!(state.get_drag_pos("btn", bounds), None);

        let (clicked, pos) = state.button_with_pos("btn", bounds);
        assert!(!clicked);
        assert_eq!(pos, None);
    }
}
