// Copyright (C) 2025  Braiins Systems s.r.o.

//! Widget scene layout tracking with drag-based scene navigation.

use bmc::compositor::SceneLayout;

/// Drag distance (fraction of screen width) required to commit a scene change.
const COMMIT_DISTANCE_FRACTION: f32 = 0.30;

/// Velocity threshold (px/s) to commit even if distance is short.
const COMMIT_VELOCITY: f32 = 800.0;

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
}

impl Default for WidgetTracker {
    fn default() -> Self {
        Self {
            scenes: vec![SceneLayout::default()],
            current_index: 0,
            drag_offset: None,
            screen_width: 0,
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
        let should_commit = distance > width * COMMIT_DISTANCE_FRACTION
            || (velocity_x.abs() > COMMIT_VELOCITY && velocity_x.signum() == (dx as f32).signum());

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
