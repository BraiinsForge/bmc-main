// Copyright (C) 2025  Braiins Systems s.r.o.

//! Widget scene layout tracking with drag-based scene navigation.

use std::collections::HashSet;

use bmc::compositor::{InstanceId, SceneLayout};

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

    /// Replace the cycling list. Tries to keep the same scene active
    /// by remapping `current_index` via `scene_id`; falls back to 0
    /// when the active scene is no longer in the list. Cancels any
    /// in-progress drag.
    pub fn set_scene_cycling(&mut self, scenes: Vec<SceneLayout>) {
        let active_id = self.scenes.get(self.current_index).and_then(|s| s.scene_id);

        if scenes.is_empty() {
            self.scenes = vec![SceneLayout::default()];
            self.current_index = 0;
        } else {
            let new_index = active_id
                .and_then(|id| scenes.iter().position(|s| s.scene_id == Some(id)))
                .unwrap_or(0);
            self.scenes = scenes;
            self.current_index = new_index;
        }
        self.drag_offset = None;
    }

    /// Show `layout` as the active scene. If a scene with the same
    /// `scene_id` is already in the cycling list, replace it in place
    /// and move `current_index` to it; otherwise reset to a single-scene
    /// list with this layout.
    pub fn set_active_scene(&mut self, layout: SceneLayout) {
        let idx = layout
            .scene_id
            .and_then(|id| self.scenes.iter().position(|s| s.scene_id == Some(id)));
        if let Some(idx) = idx {
            self.scenes[idx] = layout;
            self.current_index = idx;
        } else {
            self.scenes = vec![layout];
            self.current_index = 0;
        }
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

    #[must_use]
    pub fn drag_neighbor_scene(&self) -> Option<&SceneLayout> {
        let dx = self.drag_offset?;
        let direction = if dx <= 0 { 1 } else { -1 };
        self.neighbor_scene(direction)
    }

    #[must_use]
    pub fn presented_widget_ids(&self) -> HashSet<InstanceId> {
        let mut ids = HashSet::new();
        collect_visible_widget_ids(self.active_scene(), &mut ids);
        if self.drag_offset.is_some()
            && let Some(neighbor) = self.drag_neighbor_scene()
        {
            collect_visible_widget_ids(neighbor, &mut ids);
        }
        ids
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

        let commit_by_distance = distance > width * self.commit.distance_fraction;
        let commit_by_velocity = dx != 0
            && velocity_x.abs() > self.commit.velocity
            && velocity_x.signum() == (dx as f32).signum();
        let should_commit = commit_by_distance || commit_by_velocity;

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

fn collect_visible_widget_ids(scene: &SceneLayout, ids: &mut HashSet<InstanceId>) {
    ids.extend(
        scene
            .widgets
            .iter()
            .filter(|widget| widget.visible)
            .map(|widget| widget.instance_id.clone()),
    );
}

#[cfg(test)]
mod tests {
    use bmc::compositor::{Position, SceneLayout, Size, WidgetPlacement};
    use bmc::scene::SceneId;

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
    fn zero_dx_fast_positive_velocity_does_not_commit() {
        let mut t = three_scene_tracker();
        t.start_drag();
        t.update_drag(0);
        assert!(
            !t.end_drag(0, 900.0),
            "zero horizontal travel must not commit by velocity alone"
        );
    }

    #[test]
    fn zero_dx_fast_negative_velocity_does_not_commit() {
        let mut t = three_scene_tracker();
        t.start_drag();
        t.update_drag(0);
        assert!(
            !t.end_drag(0, -900.0),
            "zero horizontal travel must not commit by velocity alone"
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

    fn scene_with_widget(id: &str) -> SceneLayout {
        SceneLayout {
            scene_id: None,
            widgets: vec![WidgetPlacement {
                instance_id: id.to_owned(),
                position: Position { x: 0, y: 0 },
                size: Size {
                    width: 1280,
                    height: 480,
                },
                visible: true,
            }],
        }
    }

    #[test]
    fn presented_widget_ids_include_active_scene_when_idle() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("active"), scene_with_widget("next")]);

        assert_eq!(
            t.presented_widget_ids(),
            std::collections::HashSet::from([String::from("active")])
        );
    }

    #[test]
    fn presented_widget_ids_include_drag_neighbor_during_drag() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("active"),
            scene_with_widget("next"),
            scene_with_widget("previous"),
        ]);

        t.start_drag();
        t.update_drag(-100);

        assert_eq!(
            t.presented_widget_ids(),
            std::collections::HashSet::from([String::from("active"), String::from("next")])
        );
    }

    fn scene_with_id(id: SceneId) -> SceneLayout {
        SceneLayout {
            scene_id: Some(id),
            widgets: vec![],
        }
    }

    #[test]
    fn set_scene_cycling_preserves_active_scene_after_reorder() {
        let id_a = SceneId::generate();
        let id_b = SceneId::generate();
        let id_c = SceneId::generate();

        let a = scene_with_id(id_a);
        let b = scene_with_id(id_b);
        let c = scene_with_id(id_c);

        let mut tracker = WidgetTracker::default();
        tracker.set_scene_cycling(vec![a.clone(), b.clone(), c.clone()]);
        // Move to B (index 1).
        tracker.set_active_scene(b.clone());
        assert_eq!(
            tracker.active_scene().scene_id,
            Some(id_b),
            "active scene should be B after set_active_scene"
        );

        // Reorder so B is at index 2.
        tracker.set_scene_cycling(vec![a, c, b]);
        assert_eq!(
            tracker.active_scene().scene_id,
            Some(id_b),
            "active scene must follow its scene_id, not its old index",
        );
    }

    #[test]
    fn set_scene_cycling_falls_back_to_zero_when_active_scene_missing() {
        let id_a = SceneId::generate();
        let id_b = SceneId::generate();

        let a = scene_with_id(id_a);
        let b = scene_with_id(id_b);

        let mut tracker = WidgetTracker::default();
        tracker.set_scene_cycling(vec![a.clone(), b]);
        tracker.set_active_scene(scene_with_id(id_b));
        // Drop B from the cycling list.
        tracker.set_scene_cycling(vec![a]);
        // Current index falls back to 0 (only scene left is A).
        assert_eq!(
            tracker.active_scene().scene_id,
            Some(id_a),
            "should fall back to index 0 when active scene is removed"
        );
    }
}
