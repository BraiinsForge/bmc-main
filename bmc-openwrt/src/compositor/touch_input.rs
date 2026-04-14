// Copyright (C) 2026  Braiins Systems s.r.o.

//! Touch input handling with drag gesture detection and velocity tracking.

use std::collections::VecDeque;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;
use std::time::Instant;

use evdev::{AbsInfo, AbsoluteAxisCode, Device, EventSummary, KeyCode, SynchronizationCode};

/// Movement (in pixels) required before a drag activates.
const DRAG_DEAD_ZONE: i32 = 15;

/// Maximum vertical deviation allowed during a horizontal drag.
const DRAG_MAX_Y_DEVIATION: i32 = 150;

/// Maximum number of recent position samples kept for velocity estimation.
const VELOCITY_SAMPLE_COUNT: usize = 5;

/// Maximum duration (ms) for a tap gesture.
const TAP_MAX_DURATION_MS: u128 = 300;

/// Maximum movement (px) for a tap gesture.
const TAP_MAX_MOVEMENT: i32 = 30;

/// Current drag offset while finger is down.
#[derive(Debug, Clone, Copy)]
pub struct DragInfo {
    /// Horizontal offset from touch start in logical pixels.
    pub dx: i32,
}

/// Touch gesture result emitted on finger up.
#[derive(Debug, Clone, Copy)]
pub enum TouchGesture {
    Tap,
    DragEnd { dx: i32, velocity_x: f32 },
}

/// Raw touch event for forwarding to widgets.
#[derive(Debug, Clone, Copy)]
pub enum RawTouchEvent {
    Down { id: u32, x: i32, y: i32 },
    Motion { id: u32, x: i32, y: i32 },
    Up { id: u32 },
}

/// Touch state tracking for gesture detection.
///
/// All coordinates (`start_x/y`, `current_x/y`) are in **logical** display
/// pixels after calibration and 270° rotation.
#[expect(
    clippy::struct_excessive_bools,
    reason = "touch input state is easier to maintain as flat flags than as nested enums"
)]
#[derive(Debug)]
struct TouchState {
    start_x: i32,
    start_y: i32,
    current_x: i32,
    current_y: i32,
    /// Raw evdev coordinates accumulated before SYN_REPORT.
    pending_raw_x: i32,
    pending_raw_y: i32,
    start_time: Instant,
    is_touching: bool,
    /// Set on BTN_TOUCH press, consumed on the first SYN_REPORT with
    /// position data. Replaces the timing-based `TOUCH_START_WINDOW_MS`
    /// heuristic with deterministic first-sample detection.
    needs_down: bool,
    /// Set on BTN_TOUCH release and finalized on the next SYN_REPORT so
    /// the final coordinate sample is applied before gesture classification.
    pending_release: bool,
    /// Whether the drag dead zone has been exceeded.
    drag_active: bool,
    /// Recent (logical_x, timestamp) samples for velocity estimation.
    velocity_samples: VecDeque<(i32, Instant)>,
}

impl Default for TouchState {
    fn default() -> Self {
        Self {
            start_x: 0,
            start_y: 0,
            current_x: 0,
            current_y: 0,
            pending_raw_x: 0,
            pending_raw_y: 0,
            start_time: Instant::now(),
            is_touching: false,
            needs_down: false,
            pending_release: false,
            drag_active: false,
            velocity_samples: VecDeque::with_capacity(VELOCITY_SAMPLE_COUNT),
        }
    }
}

/// Maps raw evdev coordinates to logical display coordinates.
///
/// The touchscreen driver already reports coordinates aligned with the
/// logical landscape orientation (evdev X = horizontal, evdev Y = vertical),
/// so no rotation is needed — only range normalization and scaling.
#[derive(Debug)]
struct AxisCalibration {
    evdev_x_min: i32,
    evdev_x_range: i32,
    evdev_y_min: i32,
    evdev_y_range: i32,
    logical_width: i32,
    logical_height: i32,
}

impl AxisCalibration {
    fn new(
        x_info: Option<AbsInfo>,
        y_info: Option<AbsInfo>,
        logical_w: u32,
        logical_h: u32,
    ) -> Self {
        // Fallback to physical panel dimensions if axis info unavailable
        let x_min = x_info.map_or(0, |i| i.minimum());
        let x_max = x_info.map_or(479, |i| i.maximum());
        let y_min = y_info.map_or(0, |i| i.minimum());
        let y_max = y_info.map_or(1279, |i| i.maximum());

        #[expect(clippy::cast_possible_wrap)]
        Self {
            evdev_x_min: x_min,
            evdev_x_range: (x_max - x_min).max(1),
            evdev_y_min: y_min,
            evdev_y_range: (y_max - y_min).max(1),
            logical_width: logical_w as i32,
            logical_height: logical_h as i32,
        }
    }

    /// Convert raw evdev coordinates to logical display coordinates.
    fn to_logical(&self, evdev_x: i32, evdev_y: i32) -> (i32, i32) {
        let norm_x = f64::from(evdev_x - self.evdev_x_min) / f64::from(self.evdev_x_range);
        let norm_y = f64::from(evdev_y - self.evdev_y_min) / f64::from(self.evdev_y_range);

        #[expect(clippy::cast_possible_truncation)]
        let logical_x = (norm_x * f64::from(self.logical_width)) as i32;
        #[expect(clippy::cast_possible_truncation)]
        let logical_y = (norm_y * f64::from(self.logical_height)) as i32;

        (
            logical_x.clamp(0, self.logical_width - 1),
            logical_y.clamp(0, self.logical_height - 1),
        )
    }
}

fn handle_touch_key_event(state: &mut TouchState, pressed: bool) {
    if pressed && !state.is_touching {
        state.is_touching = true;
        state.needs_down = true;
        state.pending_release = false;
        state.drag_active = false;
        state.start_time = Instant::now();
        state.velocity_samples.clear();
    } else if !pressed && state.is_touching {
        state.pending_release = true;
    }
}

fn flush_syn_report(
    calibration: &AxisCalibration,
    state: &mut TouchState,
    pending_x: &mut Option<i32>,
    pending_y: &mut Option<i32>,
    raw_events: &mut Vec<RawTouchEvent>,
    drag_seen_this_poll: &mut bool,
) -> Option<TouchGesture> {
    let mut pos_updated = false;
    if let Some(raw_x) = pending_x.take() {
        state.pending_raw_x = raw_x;
        pos_updated = true;
    }
    if let Some(raw_y) = pending_y.take() {
        state.pending_raw_y = raw_y;
        pos_updated = true;
    }

    if (state.is_touching || state.pending_release) && pos_updated {
        let (lx, ly) = calibration.to_logical(state.pending_raw_x, state.pending_raw_y);

        state.current_x = lx;
        state.current_y = ly;

        if state.needs_down {
            // First position sample after BTN_TOUCH — treat as start.
            state.needs_down = false;
            state.start_x = lx;
            state.start_y = ly;
            raw_events.push(RawTouchEvent::Down {
                id: 0,
                x: lx,
                y: ly,
            });
        } else {
            raw_events.push(RawTouchEvent::Motion {
                id: 0,
                x: lx,
                y: ly,
            });
        }
        update_drag_tracking(state, drag_seen_this_poll);
    }

    if state.pending_release {
        state.pending_release = false;
        state.is_touching = false;
        raw_events.push(RawTouchEvent::Up { id: 0 });
        let gesture = detect_gesture(state);
        state.drag_active = false;
        return gesture;
    }

    None
}

fn update_drag_tracking(state: &mut TouchState, drag_seen_this_poll: &mut bool) {
    let now = Instant::now();
    let samples = &mut state.velocity_samples;

    samples.push_back((state.current_x, now));
    if samples.len() > VELOCITY_SAMPLE_COUNT {
        samples.pop_front();
    }

    if !state.drag_active {
        let dx = (state.current_x - state.start_x).abs();
        let dy = (state.current_y - state.start_y).abs();
        if dx > DRAG_DEAD_ZONE && dy <= DRAG_MAX_Y_DEVIATION {
            state.drag_active = true;
            *drag_seen_this_poll = true;
            tracing::debug!("Drag activated: dx={}", dx);
        }
    }

    if state.drag_active {
        *drag_seen_this_poll = true;
    }
}

fn detect_gesture(state: &TouchState) -> Option<TouchGesture> {
    let duration = state.start_time.elapsed().as_millis();
    let dx = state.current_x - state.start_x;
    let dy = (state.current_y - state.start_y).abs();

    tracing::debug!(
        "Touch ended: dx={}, dy={}, duration={}ms, drag_active={}",
        dx,
        dy,
        duration,
        state.drag_active
    );

    if state.drag_active {
        let velocity_x = TouchInput::compute_velocity(&state.velocity_samples);
        tracing::info!("DragEnd: dx={}, velocity={:.0} px/s", dx, velocity_x);
        return Some(TouchGesture::DragEnd { dx, velocity_x });
    }

    if duration <= TAP_MAX_DURATION_MS && dx.abs() <= TAP_MAX_MOVEMENT && dy <= TAP_MAX_MOVEMENT {
        tracing::info!("Tap detected at ({}, {})", state.current_x, state.current_y);
        return Some(TouchGesture::Tap);
    }

    tracing::debug!("No gesture detected");
    None
}

/// Touch input handler with drag gesture detection.
pub struct TouchInput {
    device: Device,
    calibration: AxisCalibration,
    state: TouchState,
    /// Pending raw X coordinate (applied on SYN_REPORT).
    pending_x: Option<i32>,
    /// Pending raw Y coordinate (applied on SYN_REPORT).
    pending_y: Option<i32>,
    /// Raw touch events collected during poll (for forwarding to widgets).
    raw_events: Vec<RawTouchEvent>,
    /// Whether a drag was active at any point during the last `poll()` call.
    /// Prevents leaking raw touch events to widgets when a complete swipe
    /// (down→move→up) arrives in one evdev batch.
    drag_seen_this_poll: bool,
}

impl std::fmt::Debug for TouchInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TouchInput").finish_non_exhaustive()
    }
}

impl TouchInput {
    /// Open a touch input device.
    ///
    /// `logical_width` and `logical_height` are the display dimensions after
    /// rotation (landscape).  Evdev coordinates are mapped to this logical
    /// space with 270° rotation applied.
    ///
    /// # Errors
    /// Returns an error if the device cannot be opened.
    pub fn open(path: &Path, logical_width: u32, logical_height: u32) -> std::io::Result<Self> {
        let device = Device::open(path)?;
        device.set_nonblocking(true)?;

        tracing::info!("Opened touch device: {:?}", path);

        if let Some(name) = device.name() {
            tracing::info!("  Device name: {}", name);
        }

        let mut x_info = None;
        let mut y_info = None;
        if let Ok(abs_iter) = device.get_absinfo() {
            for (code, info) in abs_iter {
                match code {
                    AbsoluteAxisCode::ABS_X => x_info = Some(info),
                    AbsoluteAxisCode::ABS_Y => y_info = Some(info),
                    _ => {}
                }
            }
        }

        let calibration = AxisCalibration::new(x_info, y_info, logical_width, logical_height);
        tracing::info!(
            "Touch calibration: evdev X=[{}, {}], Y=[{}, {}] -> logical {}x{}",
            calibration.evdev_x_min,
            calibration.evdev_x_min + calibration.evdev_x_range,
            calibration.evdev_y_min,
            calibration.evdev_y_min + calibration.evdev_y_range,
            logical_width,
            logical_height,
        );

        Ok(Self {
            device,
            calibration,
            state: TouchState::default(),
            pending_x: None,
            pending_y: None,
            raw_events: Vec::new(),
            drag_seen_this_poll: false,
        })
    }

    /// Borrow the underlying device file descriptor (for calloop registration).
    #[must_use]
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.device.as_fd()
    }

    /// Drain collected raw touch events (for forwarding to widgets).
    pub fn drain_raw_events(&mut self) -> Vec<RawTouchEvent> {
        std::mem::take(&mut self.raw_events)
    }

    /// Whether a scene drag was active at any point during the last `poll()`.
    ///
    /// Use this instead of `drag_info().is_some()` to decide whether raw touch
    /// events should be forwarded to widgets. When a complete swipe arrives in
    /// one evdev batch, `drag_active` is already cleared by the time `poll()`
    /// returns, but this flag remains set.
    #[must_use]
    pub fn drag_seen_this_poll(&self) -> bool {
        self.drag_seen_this_poll
    }

    /// Returns drag info while a drag is active (finger down, past dead zone).
    #[must_use]
    pub fn drag_info(&self) -> Option<DragInfo> {
        if !self.state.drag_active {
            return None;
        }
        let dx = self.state.current_x - self.state.start_x;
        Some(DragInfo { dx })
    }

    /// Compute velocity from the full sample window (px/sec).
    fn compute_velocity(samples: &VecDeque<(i32, Instant)>) -> f32 {
        if samples.len() < 2 {
            return 0.0;
        }
        let (x_first, t_first) = samples[0];
        let (x_last, t_last) = samples[samples.len() - 1];
        let dt = t_last.duration_since(t_first).as_secs_f32();
        if dt < 0.002 {
            return 0.0;
        }
        #[expect(clippy::cast_precision_loss)]
        let vel = (x_last - x_first) as f32 / dt;
        vel
    }

    /// Poll for touch events and detect gestures.
    ///
    /// Returns `Some(TouchGesture)` if a gesture completed (finger up), `None` otherwise.
    pub fn poll(&mut self) -> Option<TouchGesture> {
        let events: Vec<_> = match self.device.fetch_events() {
            Ok(events) => events.collect(),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return None,
            Err(e) => {
                tracing::warn!("Touch device read error: {}", e);
                return None;
            }
        };

        // Reset per-poll tracking. Updated below whenever drag_active transitions on.
        self.drag_seen_this_poll = false;

        let mut gesture_result = None;

        for event in events {
            #[expect(clippy::wildcard_enum_match_arm)]
            match event.destructure() {
                EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_X, value) => {
                    self.pending_x = Some(value);
                }
                EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_Y, value) => {
                    self.pending_y = Some(value);
                }
                EventSummary::Key(_, KeyCode::BTN_TOUCH, value) => {
                    let pressed = value == 1;
                    handle_touch_key_event(&mut self.state, pressed);
                }
                EventSummary::Synchronization(_, SynchronizationCode::SYN_REPORT, _) => {
                    if let Some(gesture) = flush_syn_report(
                        &self.calibration,
                        &mut self.state,
                        &mut self.pending_x,
                        &mut self.pending_y,
                        &mut self.raw_events,
                        &mut self.drag_seen_this_poll,
                    ) {
                        gesture_result = Some(gesture);
                    }
                }
                _ => {}
            }
        }

        gesture_result
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AxisCalibration, RawTouchEvent, TouchGesture, TouchState, flush_syn_report,
        handle_touch_key_event,
    };

    fn calibration() -> AxisCalibration {
        AxisCalibration::new(None, None, 480, 1280)
    }

    #[test]
    fn release_waits_for_syn_report_before_emitting_up() {
        let mut state = TouchState::default();
        let calibration = calibration();
        let mut pending_x = Some(100);
        let mut pending_y = Some(200);
        let mut raw_events = Vec::new();
        let mut drag_seen_this_poll = false;

        handle_touch_key_event(&mut state, true);
        let down = flush_syn_report(
            &calibration,
            &mut state,
            &mut pending_x,
            &mut pending_y,
            &mut raw_events,
            &mut drag_seen_this_poll,
        );
        assert!(down.is_none());

        handle_touch_key_event(&mut state, false);
        assert!(state.pending_release);
        assert!(matches!(
            raw_events.as_slice(),
            [RawTouchEvent::Down { x: 100, y: 200, .. }]
        ));
    }

    #[test]
    fn final_syn_report_updates_motion_before_release_gesture() {
        let mut state = TouchState::default();
        let calibration = calibration();
        let mut raw_events = Vec::new();
        let mut drag_seen_this_poll = false;

        handle_touch_key_event(&mut state, true);
        let mut pending_x = Some(100);
        let mut pending_y = Some(200);
        let first_gesture = flush_syn_report(
            &calibration,
            &mut state,
            &mut pending_x,
            &mut pending_y,
            &mut raw_events,
            &mut drag_seen_this_poll,
        );
        assert!(first_gesture.is_none());

        let mut release_x = Some(140);
        let mut release_y = Some(200);
        handle_touch_key_event(&mut state, false);
        let release_gesture = flush_syn_report(
            &calibration,
            &mut state,
            &mut release_x,
            &mut release_y,
            &mut raw_events,
            &mut drag_seen_this_poll,
        );

        assert!(matches!(
            release_gesture,
            Some(TouchGesture::DragEnd { dx: 40, .. })
        ));
        assert!(matches!(
            raw_events.as_slice(),
            [
                RawTouchEvent::Down { x: 100, y: 200, .. },
                RawTouchEvent::Motion { x: 140, y: 200, .. },
                RawTouchEvent::Up { .. }
            ]
        ));
        assert!(drag_seen_this_poll);
    }
}
