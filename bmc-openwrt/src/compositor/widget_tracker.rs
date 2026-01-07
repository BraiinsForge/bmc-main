// Copyright (C) 2025  Braiins Systems s.r.o.

//! Widget scene layout tracking.

use bmc::compositor::SceneLayout;

#[derive(Debug, Default)]
pub struct WidgetTracker {
    active_scene: SceneLayout,
}

impl WidgetTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_active_scene(&mut self, layout: SceneLayout) {
        self.active_scene = layout;
    }

    #[must_use]
    pub fn active_scene(&self) -> &SceneLayout {
        &self.active_scene
    }
}
