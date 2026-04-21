// Copyright (C) 2026  Braiins Systems s.r.o.

//! Backend-agnostic gesture state machine for touch input.
//!
//! The compositor drives a single gesture policy — tap detection,
//! horizontal drag activation past a dead zone, and velocity-weighted
//! scene-swipe commit — from Smithay's libinput backend. This module
//! encodes that policy as a pure state machine.
//!
//! Positions are passed as [`Point<f64, Logical>`][smithay::utils::Point]
//! (Smithay's typed logical coordinate) so the rest of the compositor
//! shares one frame of reference, matching what `wl_touch.down` and
//! `wl_touch.motion` carry on the wire. Timestamps are `u32` milliseconds
//! sourced from libinput event `time_msec()`, so this module knows
//! nothing about libinput itself.

use std::collections::VecDeque;

use smithay::utils::{Logical, Point};

/// Movement (in logical pixels) required before a drag activates.
pub const DRAG_DEAD_ZONE: f64 = 15.0;

/// Maximum vertical deviation allowed during a horizontal drag.
pub const DRAG_MAX_Y_DEVIATION: f64 = 150.0;

/// Maximum number of recent position samples kept for velocity estimation.
pub const VELOCITY_SAMPLE_COUNT: usize = 5;

/// Maximum duration (ms) for a tap gesture.
pub const TAP_MAX_DURATION_MS: u32 = 300;

/// Maximum movement (logical pixels) for a tap gesture.
pub const TAP_MAX_MOVEMENT: f64 = 30.0;

/// Drag offset reported while a horizontal drag is in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragInfo {
    /// Horizontal offset from touch start in logical pixels.
    pub dx: f64,
}

/// Gesture classification emitted on touch release.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchGesture {
    Tap,
    DragEnd { dx: f64, velocity_x: f32 },
}

/// Pure gesture state machine.
///
/// Positions are in the Smithay logical coordinate space. Callers pass
/// `Point<f64, Logical>` values directly from `wl_touch.down` /
/// `wl_touch.motion` or from libinput's `x_transformed` /
/// `y_transformed` results.
#[derive(Debug, Default)]
pub struct GestureState {
    active: bool,
    start: Point<f64, Logical>,
    current: Point<f64, Logical>,
    start_time_ms: u32,
    drag_active: bool,
    /// Recent `(x_logical, time_ms)` samples for velocity estimation.
    velocity_samples: VecDeque<(f64, u32)>,
}

impl GestureState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a new touch at `location` with `time_ms` as the start time.
    ///
    /// Any previously active touch is replaced without emitting a gesture —
    /// callers must call [`GestureState::on_up`] or
    /// [`GestureState::on_cancel`] before starting a new one if they want
    /// the prior result.
    pub fn on_down(&mut self, location: Point<f64, Logical>, time_ms: u32) {
        self.active = true;
        self.start = location;
        self.current = location;
        self.start_time_ms = time_ms;
        self.drag_active = false;
        self.velocity_samples.clear();
        self.velocity_samples.push_back((location.x, time_ms));
    }

    /// Update the current touch position. Returns `true` when this call
    /// transitions `drag_active` from `false` to `true`.
    pub fn on_motion(&mut self, location: Point<f64, Logical>, time_ms: u32) -> bool {
        if !self.active {
            return false;
        }
        self.current = location;
        self.push_velocity_sample(location.x, time_ms);
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
    pub fn on_cancel(&mut self) {
        self.active = false;
        self.drag_active = false;
    }

    /// Drag info while a drag is active (past dead zone, finger still down).
    #[must_use]
    pub fn drag_info(&self) -> Option<DragInfo> {
        if self.drag_active {
            Some(DragInfo {
                dx: self.current.x - self.start.x,
            })
        } else {
            None
        }
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "exposed for downstream arbitration that may need read-only drag state"
        )
    )]
    #[must_use]
    pub fn drag_active(&self) -> bool {
        self.drag_active
    }

    fn push_velocity_sample(&mut self, x: f64, time_ms: u32) {
        self.velocity_samples.push_back((x, time_ms));
        if self.velocity_samples.len() > VELOCITY_SAMPLE_COUNT {
            self.velocity_samples.pop_front();
        }
    }

    fn update_drag_activation(&mut self) {
        if self.drag_active {
            return;
        }
        let dx = (self.current.x - self.start.x).abs();
        let dy = (self.current.y - self.start.y).abs();
        if dx > DRAG_DEAD_ZONE && dy <= DRAG_MAX_Y_DEVIATION {
            self.drag_active = true;
            tracing::debug!("Drag activated: dx={:.1}", dx);
        }
    }

    fn classify(&self, end_time_ms: u32) -> Option<TouchGesture> {
        let duration = end_time_ms.saturating_sub(self.start_time_ms);
        let dx = self.current.x - self.start.x;
        let dy = (self.current.y - self.start.y).abs();

        tracing::debug!(
            "Touch ended: dx={:.1}, dy={:.1}, duration={}ms, drag_active={}",
            dx,
            dy,
            duration,
            self.drag_active,
        );

        if self.drag_active {
            let velocity_x = compute_velocity(&self.velocity_samples);
            tracing::info!("DragEnd: dx={:.1}, velocity={:.0} px/s", dx, velocity_x);
            return Some(TouchGesture::DragEnd { dx, velocity_x });
        }

        if duration <= TAP_MAX_DURATION_MS && dx.abs() <= TAP_MAX_MOVEMENT && dy <= TAP_MAX_MOVEMENT
        {
            tracing::info!(
                "Tap detected at ({:.1}, {:.1})",
                self.current.x,
                self.current.y
            );
            return Some(TouchGesture::Tap);
        }

        None
    }
}

/// Velocity in px/sec across the recorded sample window.
fn compute_velocity(samples: &VecDeque<(f64, u32)>) -> f32 {
    if samples.len() < 2 {
        return 0.0;
    }
    let (x_first, t_first) = samples[0];
    let (x_last, t_last) = samples[samples.len() - 1];
    let dt_ms = t_last.saturating_sub(t_first);
    if dt_ms < 2 {
        return 0.0;
    }
    let dt_s = f64::from(dt_ms) / 1000.0;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "gesture velocity fits comfortably in f32; downstream consumers are f32"
    )]
    let vel = ((x_last - x_first) / dt_s) as f32;
    vel
}

#[cfg(test)]
mod tests {
    use smithay::utils::{Logical, Point};

    use super::{DragInfo, GestureState, TouchGesture};

    fn p(x: f64, y: f64) -> Point<f64, Logical> {
        Point::<f64, Logical>::from((x, y))
    }

    #[test]
    fn short_touch_under_dead_zone_is_a_tap() {
        let mut g = GestureState::new();
        g.on_down(p(100.0, 200.0), 0);
        assert!(!g.on_motion(p(110.0, 205.0), 50));
        assert_eq!(g.on_up(100), Some(TouchGesture::Tap));
    }

    #[test]
    fn slow_long_touch_is_not_a_tap() {
        let mut g = GestureState::new();
        g.on_down(p(100.0, 200.0), 0);
        assert_eq!(g.on_up(400), None);
    }

    #[test]
    fn horizontal_drag_beyond_dead_zone_activates_drag() {
        let mut g = GestureState::new();
        g.on_down(p(100.0, 200.0), 0);
        assert!(!g.on_motion(p(110.0, 200.0), 10));
        assert!(
            g.on_motion(p(120.0, 200.0), 20),
            "drag should activate past dead zone"
        );
        assert_eq!(g.drag_info(), Some(DragInfo { dx: 20.0 }));
    }

    #[test]
    fn excessive_vertical_deviation_blocks_drag() {
        let mut g = GestureState::new();
        g.on_down(p(100.0, 200.0), 0);
        assert!(
            !g.on_motion(p(200.0, 400.0), 10),
            "dy > DRAG_MAX_Y_DEVIATION must not activate drag"
        );
        assert!(!g.drag_active());
        assert_eq!(g.drag_info(), None);
    }

    #[test]
    fn drag_release_emits_dragend_with_velocity() {
        let mut g = GestureState::new();
        g.on_down(p(0.0, 200.0), 0);
        g.on_motion(p(50.0, 200.0), 50);
        g.on_motion(p(100.0, 200.0), 100);
        g.on_motion(p(150.0, 200.0), 150);

        let gesture = g
            .on_up(200)
            .expect("BUG: drag release must produce a gesture");
        let TouchGesture::DragEnd { dx, velocity_x } = gesture else {
            panic!("BUG: expected DragEnd, got {gesture:?}");
        };
        assert!((dx - 150.0).abs() < f64::EPSILON);
        assert!(
            velocity_x > 0.0,
            "velocity must be positive for rightward drag"
        );
    }

    #[test]
    fn cancel_aborts_without_emitting_gesture() {
        let mut g = GestureState::new();
        g.on_down(p(100.0, 200.0), 0);
        g.on_motion(p(200.0, 200.0), 50);
        assert!(g.drag_active());

        g.on_cancel();
        assert!(!g.drag_active());
        assert_eq!(g.drag_info(), None);
        assert_eq!(g.on_up(100), None, "on_up after cancel must return None");
    }

    #[test]
    fn duplicate_timestamps_produce_zero_velocity() {
        let mut g = GestureState::new();
        g.on_down(p(0.0, 0.0), 42);
        g.on_motion(p(80.0, 0.0), 42);
        g.on_motion(p(200.0, 0.0), 42);
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
        assert!(!g.on_motion(p(0.0, 0.0), 0));
        assert_eq!(g.on_up(0), None);
    }
}
