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

//! Interaction state for immediate-mode UI.

use std::collections::{HashMap, VecDeque};

use super::types::{ActionEvent, Rect, TouchEvent};

/// Upper bound for queued hosted touch events while a widget is not rendering.
///
/// The queue lives inside the WASM runtime, downstream of the compositor. When
/// widgets stall, we preserve control edges (`Down`/`Up`/`Cancel`) and coalesce
/// or evict older motion events before dropping edges as a last resort.
const MAX_PENDING_TOUCH_EVENTS: usize = 64;

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

    /// Position of the last scroll event (for hit-testing which container to scroll).
    scroll_pos: Option<(f32, f32)>,

    /// High-level interaction events consumed this frame (clicks, scrolls).
    /// Populated during render when widgets consume input. Cleared each frame.
    pub action_log: Vec<ActionEvent>,
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
            scroll_pos: None,
            action_log: Vec::new(),
        }
    }

    /// Clear hit regions for new frame.
    pub fn begin_frame(&mut self) {
        // Reset per-frame state
        self.drag_delta_y = 0.0;
        self.scroll_pos = None;
        self.action_log.clear();

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
                TouchEvent::Scroll { x, y, delta_y } => {
                    // Mouse wheel scroll — store position for hit-testing.
                    self.drag_delta_y += delta_y;
                    self.scroll_pos = Some((x, y));
                }
            }
        }

        // Now clear hit regions for this frame
        self.hit_regions.clear();
    }

    /// Push a touch event to be processed.
    pub fn push_event(&mut self, event: TouchEvent) {
        if self.try_coalesce_tail(event) {
            return;
        }

        if self.event_queue.len() >= MAX_PENDING_TOUCH_EVENTS {
            self.make_room_for(event);
        }

        self.event_queue.push_back(event);
    }

    /// Cancel the active gesture and discard input that has not been processed.
    pub fn cancel_touch(&mut self) {
        self.event_queue.clear();
        self.touch_down_key = None;
        self.pending_click = None;
        self.pending_click_pos = None;
        self.last_touch_pos = None;
        self.drag_delta_y = 0.0;
        self.scroll_pos = None;
        self.action_log.clear();
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
            self.action_log.push(ActionEvent::Click {
                key: key.to_owned(),
                pos,
            });
            return (true, pos);
        }

        (false, None)
    }

    /// Check if a button is currently pressed (touch down on it).
    #[must_use]
    pub fn is_pressed(&self, key: &str) -> bool {
        self.touch_down_key.as_deref() == Some(key)
    }

    /// Check if any touch is currently down (processed state).
    #[must_use]
    pub fn any_touch_down(&self) -> bool {
        self.touch_down_key.is_some()
    }

    /// Get the last known touch position in absolute coordinates.
    #[must_use]
    pub fn last_touch_pos(&self) -> Option<(f32, f32)> {
        self.last_touch_pos
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

    /// Get the scroll delta if the wheel event landed inside `bounds`.
    #[must_use]
    pub fn get_global_scroll_delta(&self) -> f32 {
        self.drag_delta_y
    }

    /// Get the scroll delta if the wheel event landed inside `bounds`.
    #[must_use]
    pub fn get_scroll_delta_in(&self, bounds: &super::Rect) -> f32 {
        if let Some((x, y)) = self.scroll_pos
            && bounds.contains(x, y)
        {
            self.drag_delta_y
        } else {
            0.0
        }
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

impl InteractionState {
    fn try_coalesce_tail(&mut self, event: TouchEvent) -> bool {
        match event {
            TouchEvent::Move { x, y } => {
                if let Some(TouchEvent::Move {
                    x: queued_x,
                    y: queued_y,
                }) = self.event_queue.back_mut()
                {
                    *queued_x = x;
                    *queued_y = y;
                    return true;
                }
            }
            TouchEvent::Scroll { x, y, delta_y } => {
                if let Some(TouchEvent::Scroll {
                    x: queued_x,
                    y: queued_y,
                    delta_y: queued_delta_y,
                }) = self.event_queue.back_mut()
                {
                    *queued_x = x;
                    *queued_y = y;
                    *queued_delta_y += delta_y;
                    return true;
                }
            }
            TouchEvent::Down { .. } | TouchEvent::Up | TouchEvent::Cancel => {}
        }

        false
    }

    fn make_room_for(&mut self, incoming: TouchEvent) {
        if self.evict_oldest_motion_event() {
            return;
        }

        if self.drop_oldest_completed_gesture() {
            return;
        }

        if matches!(incoming, TouchEvent::Cancel) {
            self.event_queue.clear();
            return;
        }

        let _ = self.event_queue.pop_front();
    }

    fn evict_oldest_motion_event(&mut self) -> bool {
        if let Some(index) = self.event_queue.iter().position(TouchEvent::is_motion_like) {
            let _ = self.event_queue.remove(index);
            return true;
        }

        false
    }

    fn drop_oldest_completed_gesture(&mut self) -> bool {
        let Some(end_index) = self
            .event_queue
            .iter()
            .position(|event| matches!(event, TouchEvent::Up | TouchEvent::Cancel))
        else {
            return false;
        };

        self.event_queue.drain(..=end_index);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{InteractionState, MAX_PENDING_TOUCH_EVENTS};
    use crate::interaction::{Rect, TouchEvent};

    fn assert_valid_touch_sequence(events: &std::collections::VecDeque<TouchEvent>) {
        let mut touch_active = false;

        for event in events {
            match event {
                TouchEvent::Down { .. } => {
                    assert!(!touch_active, "unexpected nested touch down in queue");
                    touch_active = true;
                }
                TouchEvent::Up | TouchEvent::Cancel => {
                    assert!(touch_active, "unexpected terminal touch event in queue");
                    touch_active = false;
                }
                TouchEvent::Move { .. } | TouchEvent::Scroll { .. } => {}
            }
        }
    }

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

    #[test]
    fn cancel_touch_immediately_drops_active_and_queued_input() {
        let bounds = Rect::new(50.0, 20.0, 100.0, 40.0);
        let mut state = InteractionState::new();

        assert!(!state.button("btn", bounds));
        state.push_event(TouchEvent::Down { x: 80.0, y: 35.0 });
        state.begin_frame();
        assert!(state.is_pressed("btn"));

        state.push_event(TouchEvent::Up);
        state.cancel_touch();

        assert!(!state.is_pressed("btn"));
        assert!(!state.has_pending_events());
        state.begin_frame();
        assert!(
            !state.button("btn", bounds),
            "cancelled input must not click"
        );
    }

    #[test]
    fn move_flood_stays_bounded_and_keeps_latest_position() {
        let bounds = Rect::new(0.0, 0.0, 500.0, 500.0);
        let mut state = InteractionState::new();

        assert!(!state.button("btn", bounds));

        state.push_event(TouchEvent::Down { x: 40.0, y: 50.0 });

        let mut expected_pos = (40.0, 50.0);
        for idx in 0..(MAX_PENDING_TOUCH_EVENTS * 4) {
            #[expect(
                clippy::cast_precision_loss,
                reason = "test range is <256 iterations; idx fits in f32 mantissa exactly"
            )]
            let step = idx as f32;
            let x = 60.0 + step;
            let y = 90.0 + step;
            state.push_event(TouchEvent::Move { x, y });
            expected_pos = (x, y);
            assert!(state.event_queue.len() <= MAX_PENDING_TOUCH_EVENTS);
        }

        state.push_event(TouchEvent::Up);
        assert!(state.event_queue.len() <= MAX_PENDING_TOUCH_EVENTS);

        state.begin_frame();

        let (clicked, pos) = state.button_with_pos("btn", bounds);
        assert!(clicked);
        assert_eq!(pos, Some(expected_pos));
    }

    #[test]
    fn saturated_queue_evicts_motion_before_control_edge() {
        let mut state = InteractionState::new();
        state
            .event_queue
            .push_back(TouchEvent::Down { x: 10.0, y: 20.0 });
        state
            .event_queue
            .push_back(TouchEvent::Move { x: 15.0, y: 25.0 });

        while state.event_queue.len() < MAX_PENDING_TOUCH_EVENTS {
            state.event_queue.push_back(TouchEvent::Up);
        }

        state.push_event(TouchEvent::Down { x: 30.0, y: 40.0 });

        assert_eq!(state.event_queue.len(), MAX_PENDING_TOUCH_EVENTS);
        assert!(
            !state
                .event_queue
                .iter()
                .any(|event| matches!(event, TouchEvent::Move { .. }))
        );
        assert!(matches!(
            state.event_queue.front(),
            Some(TouchEvent::Down { x: 10.0, y: 20.0 })
        ));
        assert!(matches!(
            state.event_queue.back(),
            Some(TouchEvent::Down { x: 30.0, y: 40.0 })
        ));
    }

    #[test]
    fn queue_overflow_preserves_complete_down_up_sequences() {
        let mut state = InteractionState::new();

        for idx in 0..(MAX_PENDING_TOUCH_EVENTS * 4) {
            #[expect(
                clippy::cast_precision_loss,
                reason = "test loop indices are tiny and exactly representable"
            )]
            let x = idx as f32;
            state.push_event(TouchEvent::Down { x, y: 0.0 });
            state.push_event(TouchEvent::Up);
        }

        assert!(state.event_queue.len() <= MAX_PENDING_TOUCH_EVENTS);
        assert_valid_touch_sequence(&state.event_queue);
    }

    #[test]
    fn queue_overflow_preserves_complete_down_cancel_sequences() {
        let mut state = InteractionState::new();

        for idx in 0..(MAX_PENDING_TOUCH_EVENTS * 4) {
            #[expect(
                clippy::cast_precision_loss,
                reason = "test loop indices are tiny and exactly representable"
            )]
            let x = idx as f32;
            state.push_event(TouchEvent::Down { x, y: 0.0 });
            state.push_event(TouchEvent::Cancel);
        }

        assert!(state.event_queue.len() <= MAX_PENDING_TOUCH_EVENTS);
        assert_valid_touch_sequence(&state.event_queue);
    }
}
