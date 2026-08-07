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

//! Widget scene layout tracking with drag-based scene navigation.

use std::collections::HashSet;

use bmc::compositor::{InstanceId, SceneLayout};

use super::scene_cycling::TransitionFrame;

pub use bmc_widget_protocol::server::deck_widget_surface_v1::LifecycleState;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneTransitionTarget {
    pub from_index: usize,
    pub to_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AutomaticTransition {
    target: SceneTransitionTarget,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderedScene<'a> {
    pub scene: &'a SceneLayout,
    pub x_offset: i32,
    /// Scene opacity, `1.0` except while a cross-fade transition runs.
    pub alpha: f32,
}

#[derive(Debug)]
pub struct WidgetTracker {
    /// All scene layouts available for cycling.
    scenes: Vec<SceneLayout>,
    /// Index of the currently displayed scene.
    current_index: usize,
    /// Current drag offset in logical pixels (negative = dragging left).
    drag_offset: Option<i32>,
    /// Automatic scene transition target while timer-driven cycling runs.
    automatic_transition: Option<AutomaticTransition>,
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
            automatic_transition: None,
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
    /// in-progress drag. When `scenes` is empty the list is reset
    /// to a single sentinel `SceneLayout::default()` (scene_id=None);
    /// a follow-up `set_active_scene(layout_with_id)` for an id not
    /// in the (still-empty) list will then reset to a single-scene
    /// list with that layout.
    pub fn set_scene_cycling(&mut self, scenes: Vec<SceneLayout>) {
        self.automatic_transition = None;
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
        self.automatic_transition = None;
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

    /// Show scene at `index` if it exists.
    #[cfg(test)]
    pub fn set_active_scene_index(&mut self, index: usize) {
        self.automatic_transition = None;
        if index < self.scenes.len() {
            self.current_index = index;
            self.drag_offset = None;
        }
    }

    pub fn reset_to_first_scene(&mut self) {
        if self.scenes.is_empty() {
            return;
        }
        self.current_index = 0;
        self.drag_offset = None;
        self.automatic_transition = None;
    }

    #[must_use]
    pub fn active_scene(&self) -> &SceneLayout {
        &self.scenes[self.current_index]
    }

    #[must_use]
    pub fn scene_at(&self, index: usize) -> Option<&SceneLayout> {
        self.scenes.get(index)
    }

    #[must_use]
    pub fn active_scene_id(&self) -> Option<bmc::scene::SceneId> {
        self.active_scene().scene_id
    }

    #[must_use]
    pub fn active_visible_widget_ids(&self) -> Vec<InstanceId> {
        self.active_scene()
            .widgets
            .iter()
            .filter(|widget| widget.visible)
            .map(|widget| widget.instance_id.clone())
            .collect()
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
        if let Some(transition) = self.automatic_transition {
            collect_visible_widget_ids(&self.scenes[transition.target.from_index], &mut ids);
            collect_visible_widget_ids(&self.scenes[transition.target.to_index], &mut ids);
            return ids;
        }

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
            self.automatic_transition = None;
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
    ///
    /// A drag the tracker no longer follows commits nothing. Anything that lands
    /// mid-touch — a screen-off reset, night mode, a new cycling list — clears
    /// `drag_offset` while the gesture machine keeps accumulating, so a lift
    /// would otherwise advance off the scene that reset just chose.
    #[expect(clippy::cast_precision_loss)]
    pub fn end_drag(&mut self, dx: i32, velocity_x: f32) -> bool {
        if self.drag_offset.take().is_none() {
            return false;
        }

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

    #[must_use]
    pub fn automatic_transition_active(&self) -> bool {
        self.automatic_transition.is_some()
    }

    #[must_use]
    pub fn suppresses_frame_callbacks(&self) -> bool {
        self.drag_offset().is_some() || self.automatic_transition_active()
    }

    pub fn begin_automatic_transition_to_next(&mut self) -> Option<SceneTransitionTarget> {
        if self.drag_offset.is_some() {
            return None;
        }
        let to_index = self.neighbor_index(1)?;
        let target = SceneTransitionTarget {
            from_index: self.current_index,
            to_index,
        };
        self.automatic_transition = Some(AutomaticTransition { target });
        Some(target)
    }

    pub fn cancel_automatic_transition(&mut self) {
        self.automatic_transition = None;
    }

    pub fn finish_automatic_transition(&mut self) -> Option<usize> {
        let transition = self.automatic_transition.take()?;
        self.current_index = transition.target.to_index;
        self.drag_offset = None;
        Some(self.current_index)
    }

    #[cfg(test)]
    #[must_use]
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    #[must_use]
    pub fn scene_count(&self) -> usize {
        self.scenes.len()
    }

    #[must_use]
    pub fn rendered_scenes(
        &self,
        transition_frame: Option<TransitionFrame>,
        seam_overlap_px: i32,
    ) -> Vec<RenderedScene<'_>> {
        if let Some(transition) = self.automatic_transition {
            let outgoing = &self.scenes[transition.target.from_index];
            let incoming = &self.scenes[transition.target.to_index];
            if let Some(TransitionFrame::Fade { progress }) = transition_frame {
                // Source-over with complementary alphas dips toward the black clear
                // (~25% at midpoint). Deliberate: it matches the 25.10 Slint fade
                // and was verified on device.
                return vec![
                    RenderedScene {
                        scene: outgoing,
                        x_offset: 0,
                        alpha: 1.0 - progress,
                    },
                    RenderedScene {
                        scene: incoming,
                        x_offset: 0,
                        alpha: progress,
                    },
                ];
            }
            // Mid-slide the incoming scene trails the outgoing one overlapped
            // by `seam_overlap_px` (GC400 edge-sampling at the moving boundary);
            // the pre-transition warm-up frames of every effect (`transition_frame`
            // is `None` there) park it fully off screen instead — the outgoing
            // scene covers the whole panel, so there is no seam to compensate.
            let width = i32::try_from(self.screen_width).unwrap_or(i32::MAX);
            let (outgoing_offset, incoming_offset) = match transition_frame {
                Some(TransitionFrame::Slide { offset }) => {
                    (offset, offset + width - seam_overlap_px)
                }
                Some(TransitionFrame::Fade { .. }) | None => (0, width),
            };
            return vec![
                RenderedScene {
                    scene: outgoing,
                    x_offset: outgoing_offset,
                    alpha: 1.0,
                },
                RenderedScene {
                    scene: incoming,
                    x_offset: incoming_offset,
                    alpha: 1.0,
                },
            ];
        }

        let drag_offset = self.drag_offset.unwrap_or(0);
        let mut rendered = vec![RenderedScene {
            scene: self.active_scene(),
            x_offset: drag_offset,
            alpha: 1.0,
        }];
        if let Some(dx) = self.drag_offset {
            let logical_width = i32::try_from(self.screen_width).unwrap_or(i32::MAX);
            let neighbor_offset = if dx <= 0 {
                dx + logical_width - seam_overlap_px
            } else {
                dx - logical_width + seam_overlap_px
            };
            if let Some(neighbor) = self.drag_neighbor_scene() {
                rendered.push(RenderedScene {
                    scene: neighbor,
                    x_offset: neighbor_offset,
                    alpha: 1.0,
                });
            }
        }
        rendered
    }

    /// Derive the lifecycle state of every widget reachable from the
    /// scene-cycling list.
    ///
    /// - The widget in the active scene is `Visible` (idle) or `Leaving`
    ///   (during a drag).
    /// - The widgets in the immediate cycle neighbours (both -1 and +1)
    ///   are `Prepared` regardless of drag state. In a 2-scene cycle the
    ///   two neighbour indices wrap to the same scene, so the single
    ///   neighbouring widget is simply marked `Prepared` once.
    /// - During a drag, the widget in the drag-direction neighbour scene
    ///   is promoted from `Prepared` to `Entering`.
    /// - During an automatic transition, the outgoing scene is `Leaving`,
    ///   the incoming scene is `Entering`, and the outgoing scene's idle
    ///   neighbours remain `Prepared`; post-transition neighbour changes
    ///   are emitted only after the transition commits.
    ///
    /// Pure function of tracker scene state — no GL, no Wayland.
    #[must_use]
    pub fn lifecycle_states(&self) -> std::collections::HashMap<InstanceId, LifecycleState> {
        let mut out: std::collections::HashMap<InstanceId, LifecycleState> =
            std::collections::HashMap::new();

        for scene in &self.scenes {
            for widget in &scene.widgets {
                if widget.visible {
                    out.insert(widget.instance_id.clone(), LifecycleState::Dormant);
                }
            }
        }

        if let Some(transition) = self.automatic_transition {
            self.apply_transition_lifecycle(transition.target, &mut out);
            return out;
        }

        // A drag with offset 0 has ambiguous direction (the user has
        // touched but not yet moved). Treat it as idle so the active
        // widget stays Visible and the "next" scene is not erroneously
        // flagged as Entering until motion picks a direction.
        let dragging = matches!(self.drag_offset, Some(dx) if dx != 0);
        let active = self.active_scene();

        let active_state = if dragging {
            LifecycleState::Leaving
        } else {
            LifecycleState::Visible
        };
        for widget in &active.widgets {
            if widget.visible {
                out.insert(widget.instance_id.clone(), active_state);
            }
        }

        for direction in [-1_i32, 1] {
            if let Some(neighbour) = self.neighbor_scene(direction) {
                for widget in &neighbour.widgets {
                    if widget.visible {
                        out.insert(widget.instance_id.clone(), LifecycleState::Prepared);
                    }
                }
            }
        }

        if dragging && let Some(neighbour) = self.drag_neighbor_scene() {
            for widget in &neighbour.widgets {
                if widget.visible {
                    out.insert(widget.instance_id.clone(), LifecycleState::Entering);
                }
            }
        }

        out
    }

    fn apply_transition_lifecycle(
        &self,
        target: SceneTransitionTarget,
        out: &mut std::collections::HashMap<InstanceId, LifecycleState>,
    ) {
        for index in [-1_i32, 1]
            .into_iter()
            .filter_map(|direction| self.neighbor_index_from(target.from_index, direction))
        {
            if index != target.from_index && index != target.to_index {
                for widget in &self.scenes[index].widgets {
                    if widget.visible {
                        out.insert(widget.instance_id.clone(), LifecycleState::Prepared);
                    }
                }
            }
        }

        for widget in &self.scenes[target.from_index].widgets {
            if widget.visible {
                out.insert(widget.instance_id.clone(), LifecycleState::Leaving);
            }
        }
        for widget in &self.scenes[target.to_index].widgets {
            if widget.visible {
                out.insert(widget.instance_id.clone(), LifecycleState::Entering);
            }
        }
    }

    fn neighbor_index_from(&self, index: usize, direction: i32) -> Option<usize> {
        let len = self.scenes.len();
        if len <= 1 || index >= len {
            return None;
        }
        Some(if direction > 0 {
            (index + 1) % len
        } else if index == 0 {
            len - 1
        } else {
            index - 1
        })
    }

    fn neighbor_index(&self, direction: i32) -> Option<usize> {
        self.neighbor_index_from(self.current_index, direction)
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
    use std::collections::HashMap;

    use bmc::compositor::{Position, SceneLayout, Size, WidgetPlacement};
    use bmc::scene::SceneId;

    use super::{
        LifecycleState, SceneCommitConfig, SceneTransitionTarget, TransitionFrame, WidgetTracker,
    };
    use crate::compositor::lifecycle_emitter::LifecycleEmitter;

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
            cycle_duration: None,
            combined: false,
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
    fn suppresses_frame_callbacks_during_drag_or_automatic_transition() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);

        assert!(!t.suppresses_frame_callbacks());

        t.start_drag();
        assert!(t.suppresses_frame_callbacks());

        t.end_drag(0, 0.0);
        assert!(!t.suppresses_frame_callbacks());

        t.begin_automatic_transition_to_next();
        assert!(t.suppresses_frame_callbacks());

        t.finish_automatic_transition();
        assert!(!t.suppresses_frame_callbacks());
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

    #[test]
    fn automatic_transition_selects_next_scene_with_wraparound() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
        ]);
        t.set_active_scene_index(2);

        let target = t.begin_automatic_transition_to_next();

        assert_eq!(
            target,
            Some(SceneTransitionTarget {
                from_index: 2,
                to_index: 0,
            })
        );
    }

    #[test]
    fn lifecycle_automatic_transition_marks_outgoing_leaving_and_incoming_entering() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
            scene_with_widget("d"),
            scene_with_widget("e"),
        ]);
        t.set_active_scene_index(1);
        t.begin_automatic_transition_to_next();

        let states = t.lifecycle_states();

        assert_eq!(states.get("b"), Some(&LifecycleState::Leaving));
        assert_eq!(states.get("c"), Some(&LifecycleState::Entering));
        assert_eq!(states.get("a"), Some(&LifecycleState::Prepared));
        assert_eq!(states.get("d"), Some(&LifecycleState::Dormant));
        assert_eq!(states.get("e"), Some(&LifecycleState::Dormant));
    }

    #[test]
    fn lifecycle_automatic_transition_in_two_scene_cycle_has_no_prepared_scene() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);
        t.begin_automatic_transition_to_next();

        let states = t.lifecycle_states();

        assert_eq!(states.get("a"), Some(&LifecycleState::Leaving));
        assert_eq!(states.get("b"), Some(&LifecycleState::Entering));
        assert_eq!(
            states
                .values()
                .filter(|&&s| s == LifecycleState::Prepared)
                .count(),
            0
        );
    }

    #[test]
    fn lifecycle_automatic_transition_in_three_scene_cycle_prepares_opposite_scene() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
        ]);
        t.begin_automatic_transition_to_next();

        let states = t.lifecycle_states();

        assert_eq!(states.get("a"), Some(&LifecycleState::Leaving));
        assert_eq!(states.get("b"), Some(&LifecycleState::Entering));
        assert_eq!(states.get("c"), Some(&LifecycleState::Prepared));
    }

    #[test]
    fn presented_widget_ids_include_outgoing_and_incoming_during_automatic_transition() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);
        t.begin_automatic_transition_to_next();

        assert_eq!(
            t.presented_widget_ids(),
            std::collections::HashSet::from([String::from("a"), String::from("b")])
        );
    }

    #[test]
    fn manual_drag_cancels_automatic_transition() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);
        t.begin_automatic_transition_to_next();

        t.start_drag();

        assert!(!t.automatic_transition_active());
        assert!(t.drag_offset().is_some());
    }

    #[test]
    fn finish_automatic_transition_commits_target_scene() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);
        t.begin_automatic_transition_to_next();

        assert_eq!(t.finish_automatic_transition(), Some(1));
        assert_eq!(
            t.lifecycle_states().get("b"),
            Some(&LifecycleState::Visible)
        );
        assert!(!t.automatic_transition_active());
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "idle-scene alpha is the literal 1.0, not a computed value"
    )]
    fn rendered_scenes_include_offsets_during_automatic_transition() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);
        t.begin_automatic_transition_to_next();

        let rendered = t.rendered_scenes(Some(TransitionFrame::Slide { offset: -120 }), 3);

        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0].scene.widgets[0].instance_id, "a");
        assert_eq!(rendered[0].x_offset, -120);
        assert_eq!(rendered[0].alpha, 1.0);
        assert_eq!(rendered[1].scene.widgets[0].instance_id, "b");
        assert_eq!(rendered[1].x_offset, 877);
        assert_eq!(rendered[1].alpha, 1.0);
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "0.25 and its complement 0.75 are exact in binary"
    )]
    fn rendered_scenes_cross_fade_overlays_scenes_with_complementary_alpha() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);
        t.begin_automatic_transition_to_next();

        let rendered = t.rendered_scenes(Some(TransitionFrame::Fade { progress: 0.25 }), 3);

        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0].scene.widgets[0].instance_id, "a");
        assert_eq!(rendered[0].x_offset, 0);
        assert_eq!(rendered[1].scene.widgets[0].instance_id, "b");
        assert_eq!(rendered[1].x_offset, 0);

        assert_eq!(rendered[0].alpha + rendered[1].alpha, 1.0);
        assert_eq!(rendered[1].alpha, 0.25); // incoming tracks progress
    }

    #[test]
    fn rendered_scenes_pre_transition_parks_incoming_scene_offscreen() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);
        t.begin_automatic_transition_to_next();

        let rendered = t.rendered_scenes(None, 3);

        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0].x_offset, 0);
        assert_eq!(rendered[1].x_offset, 1000);
    }

    #[test]
    fn cancel_automatic_transition_keeps_current_scene() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);
        t.begin_automatic_transition_to_next();

        t.cancel_automatic_transition();

        assert!(!t.automatic_transition_active());
        assert_eq!(t.current_index(), 0);
        assert_eq!(t.scene_count(), 2);
        assert_eq!(t.rendered_scenes(None, 3).len(), 1);
    }

    fn scene_with_id(id: SceneId) -> SceneLayout {
        SceneLayout {
            scene_id: Some(id),
            cycle_duration: None,
            combined: false,
            widgets: vec![],
        }
    }

    fn scene_with_id_and_widgets(id: SceneId, widgets: &[(&str, bool)]) -> SceneLayout {
        SceneLayout {
            scene_id: Some(id),
            cycle_duration: None,
            combined: false,
            widgets: widgets
                .iter()
                .map(|(instance_id, visible)| WidgetPlacement {
                    instance_id: (*instance_id).to_owned(),
                    position: Position { x: 0, y: 0 },
                    size: Size {
                        width: 100,
                        height: 100,
                    },
                    visible: *visible,
                })
                .collect(),
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

    #[test]
    fn reset_to_first_scene_clears_transition_and_drag() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
        ]);
        t.set_active_scene_index(2);
        t.begin_automatic_transition_to_next();

        t.reset_to_first_scene();

        assert_eq!(
            t.lifecycle_states().get("a"),
            Some(&LifecycleState::Visible)
        );
        assert!(!t.automatic_transition_active());
        assert!(t.drag_offset().is_none());
    }

    #[test]
    fn end_drag_after_reset_commits_nothing() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);
        t.set_active_scene_index(1);
        t.start_drag();
        t.update_drag(-500);

        t.reset_to_first_scene();

        assert!(
            !t.end_drag(-500, -2000.0),
            "a lift past both commit thresholds must not commit once the reset \
             stopped tracking the drag"
        );
        assert_eq!(
            t.current_index(),
            0,
            "the scene the reset chose must survive the lift"
        );
    }

    #[test]
    fn lifecycle_idle_single_scene_active_is_visible() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a")]);

        let states = t.lifecycle_states();
        assert_eq!(
            states,
            HashMap::from([(String::from("a"), LifecycleState::Visible)])
        );
    }

    #[test]
    fn lifecycle_idle_three_scenes_both_neighbours_prepared() {
        // With three scenes and active=0, the +1 neighbour is scene 1 and
        // the -1 neighbour (via wrap-around) is scene 2 — so every other
        // widget is reachable by a single swipe and therefore Prepared.
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
        ]);

        let states = t.lifecycle_states();
        assert_eq!(states.get("a"), Some(&LifecycleState::Visible));
        assert_eq!(states.get("b"), Some(&LifecycleState::Prepared));
        assert_eq!(states.get("c"), Some(&LifecycleState::Prepared));
    }

    #[test]
    fn lifecycle_idle_five_scenes_only_immediate_neighbours_prepared() {
        // With five scenes and active=2, only scenes 1 and 3 are
        // reachable by a single swipe; scenes 0 and 4 require two
        // swipes and therefore stay Dormant.
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
            scene_with_widget("d"),
            scene_with_widget("e"),
        ]);
        t.set_active_scene_index(2);

        let states = t.lifecycle_states();
        assert_eq!(states.get("a"), Some(&LifecycleState::Dormant));
        assert_eq!(states.get("b"), Some(&LifecycleState::Prepared));
        assert_eq!(states.get("c"), Some(&LifecycleState::Visible));
        assert_eq!(states.get("d"), Some(&LifecycleState::Prepared));
        assert_eq!(states.get("e"), Some(&LifecycleState::Dormant));
    }

    #[test]
    fn lifecycle_drag_left_outgoing_leaving_incoming_entering() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
        ]);
        t.start_drag();
        t.update_drag(-100);

        let states = t.lifecycle_states();
        assert_eq!(
            states.get("a"),
            Some(&LifecycleState::Leaving),
            "active widget is leaving on a leftward drag"
        );
        assert_eq!(
            states.get("b"),
            Some(&LifecycleState::Entering),
            "next-scene widget enters on a leftward drag"
        );
        assert_eq!(
            states.get("c"),
            Some(&LifecycleState::Prepared),
            "the opposite neighbour stays prepared"
        );
    }

    #[test]
    fn drag_start_emission_releases_no_buffers() {
        // The compositor emits the drag-start transitions immediately
        // instead of deferring them until after a render, and that is only
        // safe because the emission frees no buffer: a drag moves the active
        // widget to Leaving and a neighbour to Entering, never to Dormant. A
        // release here would reach the host before the compositor's render
        // and break the buffer-release ordering the deferral protects.
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
        ]);

        let mut emitter = LifecycleEmitter::new();
        emitter.step(&t.lifecycle_states());

        t.start_drag();
        t.update_drag(-100);
        let emission = emitter.step(&t.lifecycle_states());

        assert!(
            emission.releases.is_empty(),
            "drag start released buffers: {:?}",
            emission.releases
        );
        assert!(
            emission
                .acquires
                .contains(&(String::from("a"), LifecycleState::Leaving)),
            "active widget should transition to Leaving"
        );
        assert!(
            emission
                .acquires
                .contains(&(String::from("b"), LifecycleState::Entering)),
            "drag-direction neighbour should transition to Entering"
        );
    }

    #[test]
    fn automatic_pre_transition_emits_only_outgoing_and_incoming_changes() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
            scene_with_widget("d"),
        ]);
        t.set_active_scene_index(1);

        let mut emitter = LifecycleEmitter::new();
        emitter.step(&t.lifecycle_states());

        t.begin_automatic_transition_to_next();
        let emission = emitter.step(&t.lifecycle_states());

        assert!(
            emission.releases.is_empty(),
            "pre-transition must not send releases: {:?}",
            emission.releases
        );
        assert_eq!(
            emission.acquires,
            vec![
                (String::from("b"), LifecycleState::Leaving),
                (String::from("c"), LifecycleState::Entering),
            ],
        );
    }

    #[test]
    fn automatic_finish_emits_final_scene_and_neighbour_updates() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
            scene_with_widget("d"),
        ]);
        t.set_active_scene_index(1);

        let mut emitter = LifecycleEmitter::new();
        emitter.step(&t.lifecycle_states());

        t.begin_automatic_transition_to_next();
        emitter.step(&t.lifecycle_states());

        t.finish_automatic_transition();
        let emission = emitter.step(&t.lifecycle_states());

        assert_eq!(
            emission.releases,
            vec![(String::from("a"), LifecycleState::Dormant)],
        );
        assert_eq!(
            emission.acquires,
            vec![
                (String::from("b"), LifecycleState::Prepared),
                (String::from("c"), LifecycleState::Visible),
                (String::from("d"), LifecycleState::Prepared),
            ],
        );
    }

    #[test]
    fn lifecycle_drag_right_uses_previous_neighbour() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
        ]);
        t.set_active_scene_index(1);
        t.start_drag();
        t.update_drag(100);

        let states = t.lifecycle_states();
        assert_eq!(states.get("b"), Some(&LifecycleState::Leaving));
        assert_eq!(states.get("a"), Some(&LifecycleState::Entering));
        assert_eq!(states.get("c"), Some(&LifecycleState::Prepared));
    }

    #[test]
    fn lifecycle_drag_in_three_scene_cycle_keeps_opposite_neighbour_prepared() {
        // Bug: while swiping from scene 1 to scene 2 in a 3-scene cycle,
        // scene 0 (the wrap-around neighbour of scene 2) was transiently
        // dropped to Dormant even though it stays one swipe away through
        // the whole gesture.
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
        ]);
        t.set_active_scene_index(1);
        t.start_drag();
        t.update_drag(-100);

        let states = t.lifecycle_states();
        assert_eq!(states.get("a"), Some(&LifecycleState::Prepared));
        assert_eq!(states.get("b"), Some(&LifecycleState::Leaving));
        assert_eq!(states.get("c"), Some(&LifecycleState::Entering));
    }

    #[test]
    fn lifecycle_idle_two_scenes_neighbour_is_prepared_not_dormant() {
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![scene_with_widget("a"), scene_with_widget("b")]);

        let states = t.lifecycle_states();
        assert_eq!(states.get("a"), Some(&LifecycleState::Visible));
        assert_eq!(
            states.get("b"),
            Some(&LifecycleState::Prepared),
            "with one neighbour it is always prepared"
        );
    }

    #[test]
    fn lifecycle_at_drag_zero_offset_is_treated_as_idle() {
        // A drag with offset 0 has no direction; the active widget must
        // stay Visible and neighbours keep their idle state until motion
        // picks a direction. Otherwise a tap-without-move thrashes the
        // lifecycle (Visible→Leaving→Visible plus four flushes).
        let mut t = WidgetTracker::with_screen_width(1000);
        t.set_scene_cycling(vec![
            scene_with_widget("a"),
            scene_with_widget("b"),
            scene_with_widget("c"),
        ]);
        t.start_drag();

        let states = t.lifecycle_states();
        assert_eq!(states.get("a"), Some(&LifecycleState::Visible));
        assert_eq!(states.get("b"), Some(&LifecycleState::Prepared));
        assert_eq!(states.get("c"), Some(&LifecycleState::Prepared));

        // update_drag(0) (user wiggled back to origin) — still ambiguous.
        t.update_drag(0);
        let states = t.lifecycle_states();
        assert_eq!(states.get("a"), Some(&LifecycleState::Visible));
        assert_eq!(states.get("b"), Some(&LifecycleState::Prepared));
        assert_eq!(states.get("c"), Some(&LifecycleState::Prepared));
    }

    #[test]
    fn set_scene_cycling_empty_then_set_active_scene_resets_to_single() {
        let mut t = WidgetTracker::default();
        let id_a = SceneId::generate();
        let layout_a = SceneLayout {
            scene_id: Some(id_a),
            ..SceneLayout::default()
        };

        t.set_scene_cycling(vec![layout_a.clone()]);
        t.set_active_scene(layout_a.clone());
        assert_eq!(t.active_scene().scene_id, Some(id_a));

        t.set_scene_cycling(vec![]);
        assert_eq!(t.active_scene().scene_id, None);

        let id_b = SceneId::generate();
        let layout_b = SceneLayout {
            scene_id: Some(id_b),
            ..SceneLayout::default()
        };
        t.set_active_scene(layout_b.clone());
        assert_eq!(t.active_scene().scene_id, Some(id_b));
        assert!(
            !t.can_drag(),
            "BUG: active scene not in (empty) list must reset to single-scene list",
        );
    }

    #[test]
    fn active_scene_helpers_follow_set_active_scene_index() {
        let id_a = SceneId::generate();
        let id_b = SceneId::generate();
        let id_c = SceneId::generate();

        let mut tracker = WidgetTracker::default();
        tracker.set_scene_cycling(vec![
            scene_with_id_and_widgets(id_a, &[("a-visible", true), ("a-hidden", false)]),
            scene_with_id_and_widgets(id_b, &[("b-visible", true)]),
            scene_with_id_and_widgets(
                id_c,
                &[
                    ("c-hidden", false),
                    ("c-visible-1", true),
                    ("c-visible-2", true),
                ],
            ),
        ]);

        assert_eq!(tracker.active_scene_id(), Some(id_a));
        assert_eq!(
            tracker.active_visible_widget_ids(),
            vec![String::from("a-visible")]
        );

        tracker.set_active_scene_index(2);
        assert_eq!(tracker.active_scene_id(), Some(id_c));
        assert_eq!(
            tracker.active_visible_widget_ids(),
            vec![String::from("c-visible-1"), String::from("c-visible-2")]
        );
    }

    #[test]
    fn active_scene_helpers_ignore_out_of_bounds_scene_index() {
        let id_a = SceneId::generate();
        let id_b = SceneId::generate();

        let mut tracker = WidgetTracker::default();
        tracker.set_scene_cycling(vec![
            scene_with_id_and_widgets(id_a, &[("a-visible", true)]),
            scene_with_id_and_widgets(id_b, &[("b-visible", true)]),
        ]);
        tracker.set_active_scene_index(1);
        assert_eq!(tracker.active_scene_id(), Some(id_b));

        tracker.set_active_scene_index(9);
        assert_eq!(
            tracker.active_scene_id(),
            Some(id_b),
            "invalid index should keep the previous active scene"
        );
        assert_eq!(
            tracker.active_visible_widget_ids(),
            vec![String::from("b-visible")]
        );
    }
}
