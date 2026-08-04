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

//! Retained per-frame state for rendering a `bmc_render` `TreeNode`.
//!
//! [`TreeUi`] owns the persistent interaction/animation/transition caches that
//! [`layout_and_render`] reads and updates, so an overlay can draw a declarative
//! UI tree against a [`Renderer`] without threading that state by hand.

use std::collections::HashMap;

use bmc_render::interaction::InteractionState;
use bmc_render::renderer::Renderer;
use bmc_render::tree::{NodeContext, ProcessContext, TreeNode, TreeResult, layout_and_render};
use bmc_render::{
    AnimationState, FrameTimings, ModalState, ScrollState, TransitionState, TransitionStateKey,
};
use taffy::prelude::TaffyTree;

/// Holds the layout/animation/interaction context for rendering a `TreeNode`.
#[expect(missing_debug_implementations)]
pub struct TreeUi {
    interaction: InteractionState,
    modal_states: HashMap<String, ModalState>,
    scroll_states: HashMap<String, ScrollState>,
    animation_states: HashMap<u64, AnimationState>,
    transition_states: HashMap<TransitionStateKey, TransitionState>,
    taffy: TaffyTree<NodeContext>,
    frame_counter: u64,
}

impl Default for TreeUi {
    fn default() -> Self {
        Self {
            interaction: InteractionState::new(),
            modal_states: HashMap::new(),
            scroll_states: HashMap::new(),
            animation_states: HashMap::new(),
            transition_states: HashMap::new(),
            taffy: TaffyTree::new(),
            frame_counter: 0,
        }
    }
}

impl TreeUi {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lay out and draw `node` at `size` against `renderer`.
    pub fn render(
        &mut self,
        node: &TreeNode,
        size: (u32, u32),
        delta_ms: u32,
        renderer: &mut dyn Renderer,
    ) -> anyhow::Result<TreeResult> {
        self.interaction.begin_frame();
        let frame_counter = self.frame_counter;
        self.frame_counter = self.frame_counter.wrapping_add(1);
        let mut timings = FrameTimings::default();
        // Host-side and not capture-replayed, so the real wall clock
        // is correct for any RelativeTimeLive the system overlay renders.
        let now_unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs().cast_signed());
        let mut ctx = ProcessContext {
            interaction: &mut self.interaction,
            modal_states: &mut self.modal_states,
            scroll_states: &mut self.scroll_states,
            animation_states: &mut self.animation_states,
            transition_states: &mut self.transition_states,
            taffy: &mut self.taffy,
            frame_counter,
            delta_ms,
            now_unix_secs,
        };
        #[expect(
            clippy::cast_precision_loss,
            reason = "display sizes are well below f32 mantissa precision"
        )]
        let (width, height) = (size.0 as f32, size.1 as f32);
        let (result, _has_active) =
            layout_and_render(node, width, height, renderer, &mut timings, &mut ctx)?;
        Ok(result)
    }

    /// Feed a touch event into the interaction state so the next `render`
    /// hit-tests it against the tree's `touch_key`s.
    pub fn push_touch(&mut self, event: crate::overlay::TouchEvent) {
        use bmc_render::interaction::TouchEvent as IE;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "surface-local logical coordinates fit f32 comfortably"
        )]
        let mapped = match event {
            crate::overlay::TouchEvent::Down { x, y, .. } => IE::Down {
                x: x as f32,
                y: y as f32,
            },
            crate::overlay::TouchEvent::Motion { x, y, .. } => IE::Move {
                x: x as f32,
                y: y as f32,
            },
            crate::overlay::TouchEvent::Up { .. } => IE::Up,
            crate::overlay::TouchEvent::Cancel => IE::Cancel,
        };
        self.interaction.push_event(mapped);
    }

    /// Cancel the active touch without discarding the last rendered hit regions.
    pub fn cancel_touch(&mut self) {
        self.interaction.cancel_touch();
    }

    /// Whether an element with `key` is currently pressed (one-frame latency,
    /// matching the WASM host). Backs the hold-to-confirm buttons.
    #[must_use]
    pub fn is_pressed(&self, key: &str) -> bool {
        self.interaction.is_pressed(key)
    }
}

#[cfg(test)]
mod tests {
    use super::TreeUi;
    use crate::overlay::TouchEvent;
    use crate::test_support::TestRenderer;
    use bmc_render::colors::Color;
    use bmc_render::tree::TreeNode;

    /// A full-width ProgressBar carrying the brightness touch key.
    fn slider_tree() -> TreeNode {
        TreeNode::ProgressBar {
            touch_key: Some("brightness".to_owned()),
            track_h: 8.0,
            mode: bmc_wasm_protocol::ProgressKind::Slider,
            fraction: 0.0,
            active: true,
            fill_color: Color::from_rgba(0, 255, 0, 255),
            track_color: Color::from_rgba(80, 80, 80, 255),
            bg_color: Color::from_rgba(0, 0, 0, 0),
            skin: None,
        }
    }

    #[test]
    fn touch_on_slider_reports_drag_fraction() {
        let mut ui = TreeUi::default();
        let mut r = TestRenderer::default();
        // Warm-up frame establishes the slider's hit region (regions are
        // prior-frame state); the touch is hit-tested on the next frame.
        ui.render(&slider_tree(), (100, 20), 16, &mut r)
            .expect("BUG: warm-up render must succeed");
        // A 100px-wide frame; touch down at x≈75 → fraction ≈ 0.75.
        ui.push_touch(TouchEvent::Down {
            id: 0,
            x: 75.0,
            y: 10.0,
        });
        let result = ui
            .render(&slider_tree(), (100, 20), 16, &mut r)
            .expect("BUG: hit-test render must succeed");
        let hit = result
            .drags
            .get("brightness")
            .expect("slider drag should be hit-tested");
        let frac = (hit.x / hit.width).clamp(0.0, 1.0);
        assert!(
            (0.6..=0.9).contains(&frac),
            "expected ~0.75, got {frac} (x={}, w={})",
            hit.x,
            hit.width
        );
    }

    #[test]
    fn touch_off_the_slider_reports_no_drag() {
        let mut ui = TreeUi::default();
        let mut r = TestRenderer::default();
        // No touch pushed → no drag entry.
        let result = ui
            .render(&slider_tree(), (100, 20), 16, &mut r)
            .expect("BUG: render must succeed");
        assert!(!result.drags.contains_key("brightness"));
    }

    #[test]
    fn coalesced_down_up_on_slider_reports_release_click() {
        let mut ui = TreeUi::default();
        let mut r = TestRenderer::default();
        // Warm-up frame establishes the slider's hit region.
        ui.render(&slider_tree(), (100, 20), 16, &mut r)
            .expect("BUG: warm-up render must succeed");
        // Down + Up drained into one frame (no render between) — the host's
        // event-coalescing case. The release leaves no `drags` entry, but the
        // click still hit-tests the finger-up position so the final value is
        // recoverable.
        ui.push_touch(TouchEvent::Down {
            id: 0,
            x: 75.0,
            y: 10.0,
        });
        ui.push_touch(TouchEvent::Up { id: 0 });
        let result = ui
            .render(&slider_tree(), (100, 20), 16, &mut r)
            .expect("BUG: hit-test render must succeed");
        assert!(
            !result.drags.contains_key("brightness"),
            "a released touch must not report an active drag"
        );
        let hit = result
            .clicks
            .get("brightness")
            .expect("a coalesced down-up on the slider must report a release click");
        let frac = (hit.x / hit.width).clamp(0.0, 1.0);
        assert!(
            (0.6..=0.9).contains(&frac),
            "expected ~0.75, got {frac} (x={}, w={})",
            hit.x,
            hit.width
        );
    }
}
