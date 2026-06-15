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

/// Default movement (in logical pixels) required before a drag activates.
pub const DRAG_DEAD_ZONE: f64 = 15.0;

/// Default maximum vertical deviation allowed during a horizontal drag.
pub const DRAG_MAX_Y_DEVIATION: f64 = 150.0;

/// Fraction of screen height that accepts a top-edge reveal origin.
pub const EDGE_HOT_ZONE_FRACTION: f64 = 0.20;

/// Downward motion required before a top-edge reveal activates.
pub const EDGE_ACTIVATION_DY: f64 = 40.0;

/// Maximum horizontal deviation for top-edge reveal activation.
pub const EDGE_MAX_X_DEVIATION: f64 = 150.0;

/// Default number of recent position samples kept for velocity estimation.
pub const VELOCITY_SAMPLE_COUNT: usize = 5;

/// Default maximum duration (ms) for a tap gesture.
pub const TAP_MAX_DURATION_MS: u32 = 300;

/// Default maximum movement (logical pixels) for a tap gesture.
pub const TAP_MAX_MOVEMENT: f64 = 30.0;

/// Tuning knobs for the gesture state machine.
///
/// [`GestureConfig::default`] reproduces the tuned appliance values used
/// today; tests supply alternative instances via
/// [`GestureState::with_config`].
#[derive(Debug, Clone, Copy)]
pub struct GestureConfig {
    /// Movement (logical pixels) required before a drag activates.
    pub drag_dead_zone: f64,
    /// Maximum vertical deviation allowed during a horizontal drag.
    pub drag_max_y_deviation: f64,
    /// Maximum number of recent position samples kept for velocity estimation.
    pub velocity_sample_count: usize,
    /// Maximum duration (ms) for a tap gesture.
    pub tap_max_duration_ms: u32,
    /// Maximum movement (logical pixels) for a tap gesture.
    pub tap_max_movement: f64,
    /// Fraction of screen height that accepts a top-edge reveal origin.
    pub edge_hot_zone_fraction: f64,
    /// Downward motion required before a top-edge reveal activates.
    pub edge_activation_dy: f64,
    /// Maximum horizontal deviation for top-edge reveal activation.
    pub edge_max_x_deviation: f64,
    /// Screen height in logical pixels; `0.0` disables edge reveal.
    pub screen_height: f64,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            drag_dead_zone: DRAG_DEAD_ZONE,
            drag_max_y_deviation: DRAG_MAX_Y_DEVIATION,
            velocity_sample_count: VELOCITY_SAMPLE_COUNT,
            tap_max_duration_ms: TAP_MAX_DURATION_MS,
            tap_max_movement: TAP_MAX_MOVEMENT,
            edge_hot_zone_fraction: EDGE_HOT_ZONE_FRACTION,
            edge_activation_dy: EDGE_ACTIVATION_DY,
            edge_max_x_deviation: EDGE_MAX_X_DEVIATION,
            screen_height: 0.0,
        }
    }
}

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

/// Gesture activation emitted on the motion sample that claims a touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionActivation {
    /// No gesture claimed this motion sample.
    None,
    /// Horizontal scene drag claimed this touch sequence.
    SceneDrag,
    /// Top-edge reveal claimed this touch sequence.
    TopEdgeReveal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TopEdgeState {
    #[default]
    Ineligible,
    Candidate,
    Active,
}

/// Pure gesture state machine.
///
/// Positions are in the Smithay logical coordinate space. Callers pass
/// `Point<f64, Logical>` values directly from `wl_touch.down` /
/// `wl_touch.motion` or from libinput's `x_transformed` /
/// `y_transformed` results.
#[derive(Debug, Default)]
pub struct GestureState {
    config: GestureConfig,
    active: bool,
    start: Point<f64, Logical>,
    current: Point<f64, Logical>,
    start_time_ms: u32,
    drag_active: bool,
    top_edge: TopEdgeState,
    /// Recent `(x_logical, time_ms)` samples for velocity estimation.
    velocity_samples: VecDeque<(f64, u32)>,
}

impl GestureState {
    #[must_use]
    pub fn with_config(config: GestureConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
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
        self.top_edge = if self.top_edge_contains(location.y) {
            TopEdgeState::Candidate
        } else {
            TopEdgeState::Ineligible
        };
        self.velocity_samples.clear();
        self.velocity_samples.push_back((location.x, time_ms));
    }

    /// Update the current touch position and report first activation.
    pub fn on_motion(&mut self, location: Point<f64, Logical>, time_ms: u32) -> MotionActivation {
        if !self.active {
            return MotionActivation::None;
        }
        self.current = location;
        self.push_velocity_sample(location.x, time_ms);
        self.update_motion_activation()
    }

    /// Finalize the touch. Returns the detected gesture, if any.
    pub fn on_up(&mut self, time_ms: u32) -> Option<TouchGesture> {
        if !self.active {
            return None;
        }
        self.active = false;
        let gesture = self.classify(time_ms);
        self.drag_active = false;
        self.top_edge = TopEdgeState::Ineligible;
        gesture
    }

    /// Abandon the current touch without emitting a gesture (protocol
    /// cancel, lost focus, etc.).
    pub fn on_cancel(&mut self) {
        self.active = false;
        self.drag_active = false;
        self.top_edge = TopEdgeState::Ineligible;
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

    fn push_velocity_sample(&mut self, x: f64, time_ms: u32) {
        self.velocity_samples.push_back((x, time_ms));
        if self.velocity_samples.len() > self.config.velocity_sample_count {
            self.velocity_samples.pop_front();
        }
    }

    fn top_edge_contains(&self, y: f64) -> bool {
        self.config.screen_height > 0.0
            && y <= self.config.screen_height * self.config.edge_hot_zone_fraction
    }

    fn update_motion_activation(&mut self) -> MotionActivation {
        if self.drag_active || matches!(self.top_edge, TopEdgeState::Active) {
            return MotionActivation::None;
        }
        let dx = (self.current.x - self.start.x).abs();
        let dy_signed = self.current.y - self.start.y;

        // Check top-edge reveal before scene drag. A downward edge swipe may
        // drift horizontally past the scene-drag dead zone; the vertical
        // activation threshold decides that sample belongs to reveal.
        if matches!(self.top_edge, TopEdgeState::Candidate)
            && dy_signed >= self.config.edge_activation_dy
            && dx <= self.config.edge_max_x_deviation
        {
            self.top_edge = TopEdgeState::Active;
            tracing::debug!(
                "Top-edge reveal activated: dy={:.1}, dx={:.1}",
                dy_signed,
                dx
            );
            return MotionActivation::TopEdgeReveal;
        }

        if dx > self.config.drag_dead_zone && dy_signed.abs() <= self.config.drag_max_y_deviation {
            self.drag_active = true;
            tracing::debug!("Drag activated: dx={:.1}", dx);
            return MotionActivation::SceneDrag;
        }

        MotionActivation::None
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

        if duration <= self.config.tap_max_duration_ms
            && dx.abs() <= self.config.tap_max_movement
            && dy <= self.config.tap_max_movement
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

    use super::{DragInfo, GestureConfig, GestureState, MotionActivation, TouchGesture};

    fn p(x: f64, y: f64) -> Point<f64, Logical> {
        Point::<f64, Logical>::from((x, y))
    }

    fn edge_aware() -> GestureState {
        GestureState::with_config(GestureConfig {
            screen_height: 480.0,
            ..GestureConfig::default()
        })
    }

    #[test]
    fn short_touch_under_dead_zone_is_a_tap() {
        let mut g = GestureState::default();
        g.on_down(p(100.0, 200.0), 0);
        assert_eq!(g.on_motion(p(110.0, 205.0), 50), MotionActivation::None);
        assert_eq!(g.on_up(100), Some(TouchGesture::Tap));
    }

    #[test]
    fn slow_long_touch_is_not_a_tap() {
        let mut g = GestureState::default();
        g.on_down(p(100.0, 200.0), 0);
        assert_eq!(g.on_up(400), None);
    }

    #[test]
    fn horizontal_drag_beyond_dead_zone_activates_drag() {
        let mut g = GestureState::default();
        g.on_down(p(100.0, 200.0), 0);
        assert_eq!(g.on_motion(p(110.0, 200.0), 10), MotionActivation::None);
        assert_eq!(
            g.on_motion(p(120.0, 200.0), 20),
            MotionActivation::SceneDrag,
            "drag should activate past dead zone",
        );
        assert_eq!(g.drag_info(), Some(DragInfo { dx: 20.0 }));
    }

    #[test]
    fn excessive_vertical_deviation_blocks_drag() {
        let mut g = GestureState::default();
        g.on_down(p(100.0, 200.0), 0);
        assert_eq!(
            g.on_motion(p(200.0, 400.0), 10),
            MotionActivation::None,
            "dy > DRAG_MAX_Y_DEVIATION must not activate drag",
        );
        assert_eq!(g.drag_info(), None);
    }

    #[test]
    fn drag_release_emits_dragend_with_velocity() {
        let mut g = GestureState::default();
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
        let mut g = GestureState::default();
        g.on_down(p(100.0, 200.0), 0);
        g.on_motion(p(200.0, 200.0), 50);
        assert!(g.drag_info().is_some());

        g.on_cancel();
        assert_eq!(g.drag_info(), None);
        assert_eq!(g.on_up(100), None, "on_up after cancel must return None");
    }

    #[test]
    fn duplicate_timestamps_produce_zero_velocity() {
        let mut g = GestureState::default();
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
        let mut g = GestureState::default();
        assert_eq!(g.on_motion(p(0.0, 0.0), 0), MotionActivation::None);
        assert_eq!(g.on_up(0), None);
    }

    #[test]
    fn custom_config_tightens_drag_dead_zone() {
        let config = GestureConfig {
            drag_dead_zone: 2.0,
            ..GestureConfig::default()
        };
        let mut g = GestureState::with_config(config);
        g.on_down(p(100.0, 200.0), 0);
        assert_eq!(
            g.on_motion(p(103.0, 200.0), 10),
            MotionActivation::SceneDrag,
            "3 px motion should activate drag when dead zone is 2",
        );
    }

    #[test]
    fn custom_config_relaxes_tap_movement_budget() {
        // Raise both drag_dead_zone and tap_max_movement so that 50 px of
        // motion is inside the tap budget instead of promoting to a drag.
        let config = GestureConfig {
            drag_dead_zone: 300.0,
            tap_max_movement: 200.0,
            ..GestureConfig::default()
        };
        let mut g = GestureState::with_config(config);
        g.on_down(p(100.0, 200.0), 0);
        g.on_motion(p(50.0, 210.0), 100);
        assert_eq!(
            g.on_up(150),
            Some(TouchGesture::Tap),
            "50 px motion should still count as tap at tap_max_movement=200"
        );
    }

    #[test]
    fn top_edge_reveal_activates_on_motion_from_hot_zone() {
        let mut g = edge_aware();
        g.on_down(p(100.0, 80.0), 0);

        assert_eq!(
            g.on_motion(p(100.0, 121.0), 10),
            MotionActivation::TopEdgeReveal
        );
    }

    #[test]
    fn diagonal_downward_swipe_from_top_band_reveals_edge() {
        let mut g = edge_aware();
        g.on_down(p(100.0, 80.0), 0);

        assert_eq!(
            g.on_motion(p(200.0, 130.0), 10),
            MotionActivation::TopEdgeReveal,
            "edge swipes may drift horizontally while moving down"
        );
    }

    #[test]
    fn top_edge_reveal_ignores_touch_started_in_the_middle() {
        let mut g = edge_aware();
        g.on_down(p(100.0, 200.0), 0);

        assert_eq!(g.on_motion(p(100.0, 260.0), 10), MotionActivation::None);
    }

    #[test]
    fn horizontal_drag_from_top_band_still_navigates_scenes() {
        let mut g = edge_aware();
        g.on_down(p(100.0, 80.0), 0);

        assert_eq!(g.on_motion(p(116.0, 80.0), 10), MotionActivation::SceneDrag);
    }

    #[test]
    fn top_edge_reveal_activates_only_once_per_sequence() {
        let mut g = edge_aware();
        g.on_down(p(100.0, 80.0), 0);

        assert_eq!(
            g.on_motion(p(100.0, 121.0), 10),
            MotionActivation::TopEdgeReveal
        );
        assert_eq!(g.on_motion(p(100.0, 180.0), 20), MotionActivation::None);
    }

    #[test]
    fn top_edge_reveal_prevents_later_scene_drag() {
        let mut g = edge_aware();
        g.on_down(p(100.0, 80.0), 0);

        assert_eq!(
            g.on_motion(p(100.0, 121.0), 10),
            MotionActivation::TopEdgeReveal
        );
        assert_eq!(g.on_motion(p(200.0, 121.0), 20), MotionActivation::None);
        assert_eq!(g.drag_info(), None);
    }
}
