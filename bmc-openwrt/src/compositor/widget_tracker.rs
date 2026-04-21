// Copyright (C) 2025  Braiins Systems s.r.o.

//! Widget scene layout tracking with drag-based scene navigation.

use bmc::compositor::SceneLayout;

/// Default drag distance (fraction of screen width) required to commit a
/// scene change.
pub const COMMIT_DISTANCE_FRACTION: f32 = 0.30;

/// Default velocity threshold (px/s) to commit even if distance is short.
pub const COMMIT_VELOCITY: f32 = 800.0;

/// Tuning knobs for scene-swipe commit detection.
///
/// [`SceneCommitConfig::default`] reproduces the tuned appliance values
/// used today; tests supply alternative instances via
/// [`WidgetTracker::with_commit_config`].
#[derive(Debug, Clone, Copy)]
pub struct SceneCommitConfig {
    /// Fraction of screen width required to commit by distance alone.
    pub distance_fraction: f32,
    /// Absolute velocity (px/s) that commits by flick even under the
    /// distance threshold, provided the flick direction agrees with the
    /// drag direction.
    pub velocity: f32,
}

impl Default for SceneCommitConfig {
    fn default() -> Self {
        Self {
            distance_fraction: COMMIT_DISTANCE_FRACTION,
            velocity: COMMIT_VELOCITY,
        }
    }
}

#[derive(Debug)]
pub struct WidgetTracker {
    /// All scene layouts available for cycling.
    scenes: Vec<SceneLayout>,
    /// Index of the currently displayed scene.
    current_index: usize,
    /// Current drag offset in logical pixels (negative = dragging left).
    drag_offset: Option<i32>,
    /// Logical screen width for commit threshold calculation.
    screen_width: u32,
    /// Commit thresholds for `end_drag`.
    commit: SceneCommitConfig,
}

impl Default for WidgetTracker {
    fn default() -> Self {
        Self {
            scenes: vec![SceneLayout::default()],
            current_index: 0,
            drag_offset: None,
            screen_width: 0,
            commit: SceneCommitConfig::default(),
        }
    }
}

impl WidgetTracker {
    #[must_use]
    pub fn with_screen_width(width: u32) -> Self {
        Self {
            screen_width: width,
            ..Self::default()
        }
    }

    /// Build a tracker with an explicit commit configuration. Used by
    /// tests and future per-panel tuning overrides.
    #[cfg(test)]
    #[must_use]
    pub fn with_commit_config(width: u32, commit: SceneCommitConfig) -> Self {
        Self {
            screen_width: width,
            commit,
            ..Self::default()
        }
    }

    /// Set all scenes for drag-based cycling. Resets to the first scene.
    pub fn set_scene_cycling(&mut self, scenes: Vec<SceneLayout>) {
        if scenes.is_empty() {
            self.scenes = vec![SceneLayout::default()];
        } else {
            self.scenes = scenes;
        }
        self.current_index = 0;
        self.drag_offset = None;
    }

    /// Set a single active scene (replaces all scenes).
    pub fn set_active_scene(&mut self, layout: SceneLayout) {
        self.scenes = vec![layout];
        self.current_index = 0;
        self.drag_offset = None;
    }

    #[must_use]
    pub fn active_scene(&self) -> &SceneLayout {
        &self.scenes[self.current_index]
    }

    /// Get the neighbor scene in the given direction (+1 = next, -1 = prev).
    #[must_use]
    pub fn neighbor_scene(&self, direction: i32) -> Option<&SceneLayout> {
        let idx = self.neighbor_index(direction)?;
        self.scenes.get(idx)
    }

    /// Whether a drag gesture can be started — i.e. there is at least one
    /// alternative scene to swipe to.
    #[must_use]
    pub fn can_drag(&self) -> bool {
        self.scenes.len() > 1
    }

    /// Begin a drag gesture. Only activates if there are multiple scenes.
    pub fn start_drag(&mut self) {
        if self.can_drag() {
            self.drag_offset = Some(0);
        }
    }

    /// Update the drag offset (logical pixels from touch start).
    pub fn update_drag(&mut self, dx: i32) {
        if self.drag_offset.is_some() {
            self.drag_offset = Some(dx);
        }
    }

    /// End the drag gesture. Returns `true` if a scene change was committed.
    #[expect(clippy::cast_precision_loss)]
    pub fn end_drag(&mut self, dx: i32, velocity_x: f32) -> bool {
        self.drag_offset = None;

        if self.scenes.len() <= 1 || self.screen_width == 0 {
            return false;
        }

        let width = self.screen_width as f32;
        let distance = dx.abs() as f32;

        // Commit if dragged far enough, or if a quick flick in the drag direction
        let should_commit = distance > width * self.commit.distance_fraction
            || (velocity_x.abs() > self.commit.velocity
                && velocity_x.signum() == (dx as f32).signum());

        if should_commit {
            // Drag left (dx < 0) → next scene; drag right (dx > 0) → prev scene
            let direction = if dx < 0 { 1 } else { -1 };
            self.advance(direction);
            true
        } else {
            false
        }
    }

    #[must_use]
    pub fn drag_offset(&self) -> Option<i32> {
        self.drag_offset
    }

    fn neighbor_index(&self, direction: i32) -> Option<usize> {
        let len = self.scenes.len();
        if len <= 1 {
            return None;
        }
        Some(if direction > 0 {
            (self.current_index + 1) % len
        } else if self.current_index == 0 {
            len - 1
        } else {
            self.current_index - 1
        })
    }

    fn advance(&mut self, direction: i32) {
        if let Some(idx) = self.neighbor_index(direction) {
            tracing::info!(
                "Scene change: {} -> {} (of {})",
                self.current_index,
                idx,
                self.scenes.len()
            );
            self.current_index = idx;
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc::compositor::SceneLayout;

    use super::{SceneCommitConfig, WidgetTracker};

    /// Build a tracker with three distinct scenes on a 1000-px-wide panel
    /// and the default commit thresholds (`distance_fraction = 0.30`,
    /// `velocity = 800`).
    fn three_scene_tracker() -> WidgetTracker {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            SceneLayout::default(),
            SceneLayout::default(),
            SceneLayout::default(),
        ]);
        t
    }

    #[test]
    fn single_scene_never_commits() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_active_scene(SceneLayout::default());
        assert!(!t.end_drag(-5000, -10_000.0), "no neighbour to advance to");
    }

    #[test]
    fn zero_screen_width_never_commits() {
        // Fresh tracker — no `with_screen_width` call, so width is 0.
        let mut t = WidgetTracker::default();
        t.set_scene_cycling(vec![SceneLayout::default(), SceneLayout::default()]);
        assert!(!t.end_drag(-500, -2000.0), "width==0 disables commit");
    }

    #[test]
    fn short_slow_drag_snaps_back() {
        let mut t = three_scene_tracker();
        t.start_drag();
        t.update_drag(-100); // 10% of width, below 30% threshold
        assert!(
            !t.end_drag(-100, -50.0),
            "short drag with slow release should not commit"
        );
    }

    #[test]
    fn long_drag_left_advances_to_next_scene() {
        let mut t = three_scene_tracker();
        t.start_drag();
        t.update_drag(-400); // 40% of width, above 30% threshold
        assert!(
            t.end_drag(-400, -200.0),
            "drag past distance threshold commits regardless of velocity"
        );
    }

    #[test]
    fn long_drag_right_advances_to_previous_scene() {
        let mut t = three_scene_tracker();
        t.start_drag();
        t.update_drag(400);
        assert!(t.end_drag(400, 200.0));
    }

    #[test]
    fn fast_flick_beats_short_distance() {
        let mut t = three_scene_tracker();
        t.start_drag();
        t.update_drag(-50); // 5% of width
        assert!(
            t.end_drag(-50, -900.0),
            "velocity above threshold commits with matching sign"
        );
    }

    #[test]
    fn fast_opposite_flick_does_not_commit() {
        let mut t = three_scene_tracker();
        t.start_drag();
        t.update_drag(-50);
        // Finger moved left but released with a sharp rightward flick —
        // sign mismatch means we refuse to commit the leftward swipe.
        assert!(
            !t.end_drag(-50, 900.0),
            "velocity sign must agree with drag direction"
        );
    }

    #[test]
    fn custom_config_lowers_velocity_threshold() {
        let config = SceneCommitConfig {
            velocity: 100.0,
            ..SceneCommitConfig::default()
        };
        let mut t = WidgetTracker::with_commit_config(1000, config);
        t.set_scene_cycling(vec![SceneLayout::default(), SceneLayout::default()]);
        t.start_drag();
        t.update_drag(-20);
        assert!(
            t.end_drag(-20, -150.0),
            "velocity=150 exceeds custom threshold 100"
        );
    }
}
