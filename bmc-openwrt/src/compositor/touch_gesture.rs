// Copyright (C) 2026  Braiins Systems s.r.o.

//! Backend-agnostic gesture state machine for touch input.
//!
//! The compositor receives touch events from two potential sources — the
//! legacy `evdev` polling wrapper in [`super::touch_input`] and, after
//! Stage 3b, Smithay's libinput backend. Both drive the same gesture
//! policy: tap detection, horizontal drag activation past a dead zone,
//! and velocity-weighted commit of a scene swipe.
//!
//! This module encodes that policy as a pure state machine driven by
//! explicit logical coordinates and monotonic millisecond timestamps.
//! It knows nothing about `evdev`, `SYN_REPORT`, libinput, or the
//! compositor event loop.

use std::collections::VecDeque;

/// Movement (in pixels) required before a drag activates.
pub const DRAG_DEAD_ZONE: i32 = 15;

/// Maximum vertical deviation allowed during a horizontal drag.
pub const DRAG_MAX_Y_DEVIATION: i32 = 150;

/// Maximum number of recent position samples kept for velocity estimation.
pub const VELOCITY_SAMPLE_COUNT: usize = 5;

/// Maximum duration (ms) for a tap gesture.
pub const TAP_MAX_DURATION_MS: u32 = 300;

/// Maximum movement (px) for a tap gesture.
pub const TAP_MAX_MOVEMENT: i32 = 30;

/// Drag offset reported while a horizontal drag is in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragInfo {
    /// Horizontal offset from touch start in logical pixels.
    pub dx: i32,
}

/// Gesture classification emitted on touch release.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchGesture {
    Tap,
    DragEnd { dx: i32, velocity_x: f32 },
}

/// Pure gesture state machine.
///
/// All coordinates are in logical display pixels — the caller is expected
/// to apply any evdev-to-logical calibration or libinput-to-output
/// transform before feeding events in.
#[derive(Debug, Default)]
pub struct GestureState {
    active: bool,
    start_x: i32,
    start_y: i32,
    current_x: i32,
    current_y: i32,
    start_time_ms: u32,
    drag_active: bool,
    velocity_samples: VecDeque<(i32, u32)>,
}

impl GestureState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a new touch at `(x, y)` with `time_ms` as the start time.
    ///
    /// Any previously active touch is replaced without emitting a gesture —
    /// callers must call [`GestureState::on_up`] or
    /// [`GestureState::on_cancel`] before starting a new one if they want
    /// the prior result.
    pub fn on_down(&mut self, x: i32, y: i32, time_ms: u32) {
        self.active = true;
        self.start_x = x;
        self.start_y = y;
        self.current_x = x;
        self.current_y = y;
        self.start_time_ms = time_ms;
        self.drag_active = false;
        self.velocity_samples.clear();
        self.velocity_samples.push_back((x, time_ms));
    }

    /// Update the current touch position. Returns `true` when this call
    /// transitions `drag_active` from `false` to `true`.
    pub fn on_motion(&mut self, x: i32, y: i32, time_ms: u32) -> bool {
        if !self.active {
            return false;
        }
        self.current_x = x;
        self.current_y = y;
        self.push_velocity_sample(x, time_ms);
        let was_active = self.drag_active;
        self.update_drag_activation();
        self.drag_active && !was_active
    }

    /// Finalize the touch. Returns the detected gesture, if any.
    pub fn on_up(&mut self, time_ms: u32) -> Option<TouchGesture> {
        if !self.active {
            return None;
        }
        self.active = false;
        let gesture = self.classify(time_ms);
        self.drag_active = false;
        gesture
    }

    /// Abandon the current touch without emitting a gesture (protocol
    /// cancel, lost focus, etc.).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "consumed in Stage 3b when libinput InputEvent::TouchCancel is routed here"
        )
    )]
    pub fn on_cancel(&mut self) {
        self.active = false;
        self.drag_active = false;
    }

    /// Drag info while a drag is active (past dead zone, finger still down).
    #[must_use]
    pub fn drag_info(&self) -> Option<DragInfo> {
        if self.drag_active {
            Some(DragInfo {
                dx: self.current_x - self.start_x,
            })
        } else {
            None
        }
    }

    #[must_use]
    pub fn drag_active(&self) -> bool {
        self.drag_active
    }

    fn push_velocity_sample(&mut self, x: i32, time_ms: u32) {
        self.velocity_samples.push_back((x, time_ms));
        if self.velocity_samples.len() > VELOCITY_SAMPLE_COUNT {
            self.velocity_samples.pop_front();
        }
    }

    fn update_drag_activation(&mut self) {
        if self.drag_active {
            return;
        }
        let dx = (self.current_x - self.start_x).abs();
        let dy = (self.current_y - self.start_y).abs();
        if dx > DRAG_DEAD_ZONE && dy <= DRAG_MAX_Y_DEVIATION {
            self.drag_active = true;
            tracing::debug!("Drag activated: dx={}", dx);
        }
    }

    fn classify(&self, end_time_ms: u32) -> Option<TouchGesture> {
        let duration = end_time_ms.saturating_sub(self.start_time_ms);
        let dx = self.current_x - self.start_x;
        let dy = (self.current_y - self.start_y).abs();

        tracing::debug!(
            "Touch ended: dx={}, dy={}, duration={}ms, drag_active={}",
            dx,
            dy,
            duration,
            self.drag_active,
        );

        if self.drag_active {
            let velocity_x = compute_velocity(&self.velocity_samples);
            tracing::info!("DragEnd: dx={}, velocity={:.0} px/s", dx, velocity_x);
            return Some(TouchGesture::DragEnd { dx, velocity_x });
        }

        if duration <= TAP_MAX_DURATION_MS && dx.abs() <= TAP_MAX_MOVEMENT && dy <= TAP_MAX_MOVEMENT
        {
            tracing::info!("Tap detected at ({}, {})", self.current_x, self.current_y);
            return Some(TouchGesture::Tap);
        }

        None
    }
}

/// Velocity in px/sec across the recorded sample window.
fn compute_velocity(samples: &VecDeque<(i32, u32)>) -> f32 {
    if samples.len() < 2 {
        return 0.0;
    }
    let (x_first, t_first) = samples[0];
    let (x_last, t_last) = samples[samples.len() - 1];
    let dt_ms = t_last.saturating_sub(t_first);
    if dt_ms < 2 {
        return 0.0;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel distances and gesture durations remain within f32 exact-integer range"
    )]
    let dx = (x_last - x_first) as f32;
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel distances and gesture durations remain within f32 exact-integer range"
    )]
    let dt_s = dt_ms as f32 / 1000.0;
    dx / dt_s
}

#[cfg(test)]
mod tests {
    use super::{DragInfo, GestureState, TouchGesture};

    #[test]
    fn short_touch_under_dead_zone_is_a_tap() {
        let mut g = GestureState::new();
        g.on_down(100, 200, 0);
        assert!(!g.on_motion(110, 205, 50));
        assert_eq!(g.on_up(100), Some(TouchGesture::Tap));
    }

    #[test]
    fn slow_long_touch_is_not_a_tap() {
        let mut g = GestureState::new();
        g.on_down(100, 200, 0);
        assert_eq!(g.on_up(400), None);
    }

    #[test]
    fn horizontal_drag_beyond_dead_zone_activates_drag() {
        let mut g = GestureState::new();
        g.on_down(100, 200, 0);
        assert!(!g.on_motion(110, 200, 10));
        assert!(
            g.on_motion(120, 200, 20),
            "drag should activate past dead zone"
        );
        assert_eq!(g.drag_info(), Some(DragInfo { dx: 20 }));
    }

    #[test]
    fn excessive_vertical_deviation_blocks_drag() {
        let mut g = GestureState::new();
        g.on_down(100, 200, 0);
        assert!(
            !g.on_motion(200, 400, 10),
            "dy > DRAG_MAX_Y_DEVIATION must not activate drag"
        );
        assert!(!g.drag_active());
        assert_eq!(g.drag_info(), None);
    }

    #[test]
    fn drag_release_emits_dragend_with_velocity() {
        let mut g = GestureState::new();
        g.on_down(0, 200, 0);
        g.on_motion(50, 200, 50);
        g.on_motion(100, 200, 100);
        g.on_motion(150, 200, 150);

        let gesture = g
            .on_up(200)
            .expect("BUG: drag release must produce a gesture");
        let TouchGesture::DragEnd { dx, velocity_x } = gesture else {
            panic!("BUG: expected DragEnd, got {gesture:?}");
        };
        assert_eq!(dx, 150);
        assert!(
            velocity_x > 0.0,
            "velocity must be positive for rightward drag"
        );
    }

    #[test]
    fn cancel_aborts_without_emitting_gesture() {
        let mut g = GestureState::new();
        g.on_down(100, 200, 0);
        g.on_motion(200, 200, 50);
        assert!(g.drag_active());

        g.on_cancel();
        assert!(!g.drag_active());
        assert_eq!(g.drag_info(), None);
        assert_eq!(g.on_up(100), None, "on_up after cancel must return None");
    }

    #[test]
    fn duplicate_timestamps_produce_zero_velocity() {
        let mut g = GestureState::new();
        g.on_down(0, 0, 42);
        g.on_motion(80, 0, 42);
        g.on_motion(200, 0, 42);
        let gesture = g
            .on_up(42)
            .expect("BUG: drag release must produce a gesture");
        let TouchGesture::DragEnd { velocity_x, .. } = gesture else {
            panic!("BUG: expected DragEnd, got {gesture:?}");
        };
        assert!(
            velocity_x.abs() < f32::EPSILON,
            "velocity must be zero when all samples share a timestamp, got {velocity_x}"
        );
    }

    #[test]
    fn motion_before_down_is_a_no_op() {
        let mut g = GestureState::new();
        assert!(!g.on_motion(0, 0, 0));
        assert_eq!(g.on_up(0), None);
    }
}
