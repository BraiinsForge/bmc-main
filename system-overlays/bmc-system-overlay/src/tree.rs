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
//! It also keeps the last tree it drew, so a frame where only animations move
//! replays it instead of building a new one.

use std::collections::HashMap;

use bmc_render::interaction::InteractionState;
use bmc_render::renderer::Renderer;
use bmc_render::tree::{NodeContext, ProcessContext, TreeNode, TreeResult, layout_and_render};
use bmc_render::{
    AnimationState, FrameTimings, ModalState, ScrollState, TransitionState, TransitionStateKey,
};
use taffy::prelude::TaffyTree;

use crate::overlay::{FrameKind, NextFrame, RenderOutcome};

/// Static-layer key for overlays drawn through the host's shared renderer.
///
/// Sound only because every overlay renders with [`LayerUse::Ignore`], so no
/// layer is ever stored under it.
const STATIC_LAYER_KEY: &str = "bmc-system-overlay";

/// The last successfully rendered tree and the size it was laid out at.
struct CachedTree {
    node: TreeNode,
    size: (u32, u32),
}

/// A rendered frame: its interaction read-back and what it asks of the schedule.
#[derive(Debug)]
pub struct TreeFrame {
    pub result: TreeResult,
    pub outcome: RenderOutcome,
}

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
    cached: Option<CachedTree>,
    #[cfg(any(test, feature = "test-support"))]
    replayed_last_frame: bool,
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
            cached: None,
            #[cfg(any(test, feature = "test-support"))]
            replayed_last_frame: false,
        }
    }
}

impl TreeUi {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lay out and draw a frame at `size` against `renderer`.
    ///
    /// An [`FrameKind::AnimationOnly`] frame replays the cached tree.
    /// `build` runs instead when the frame is fresh, nothing is cached,
    /// the size changed, or touch events are pending, as widgets do.
    /// A failed render drops the cache, so the next frame builds again.
    pub fn render(
        &mut self,
        frame: FrameKind,
        size: (u32, u32),
        delta_ms: u32,
        renderer: &mut dyn Renderer,
        build: impl FnOnce() -> TreeNode,
    ) -> anyhow::Result<TreeFrame> {
        let replay = frame == FrameKind::AnimationOnly
            && !self.interaction.has_pending_events()
            && self
                .cached
                .as_ref()
                .is_some_and(|cached| cached.size == size);
        let node = match self.cached.take() {
            Some(cached) if replay => cached.node,
            _ => build(),
        };
        #[cfg(any(test, feature = "test-support"))]
        {
            self.replayed_last_frame = false;
        }
        let frame = self.render_node(&node, size, delta_ms, renderer)?;
        #[cfg(any(test, feature = "test-support"))]
        {
            self.replayed_last_frame = replay;
        }
        self.cached = Some(CachedTree { node, size });
        Ok(frame)
    }

    /// Drop the cached tree, so the next frame builds a new one.
    pub fn clear(&mut self) {
        self.cached = None;
    }

    /// Whether the last [`Self::render`] replayed the cached tree and drew it.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn replayed_last_frame(&self) -> bool {
        self.replayed_last_frame
    }

    fn render_node(
        &mut self,
        node: &TreeNode,
        size: (u32, u32),
        delta_ms: u32,
        renderer: &mut dyn Renderer,
    ) -> anyhow::Result<TreeFrame> {
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
            emit: bmc_render::tree::EmitMode::All,
            static_layer: bmc_render::tree::LayerUse::Ignore,
            static_layer_key: STATIC_LAYER_KEY,
            layout_outlines: false,
            stroke_outlines: bmc_render::tree::debug_layout_enabled(),
            damage_rects: &[],
        };
        #[expect(
            clippy::cast_precision_loss,
            reason = "display sizes are well below f32 mantissa precision"
        )]
        let (width, height) = (size.0 as f32, size.1 as f32);
        let (result, has_active_animations) =
            layout_and_render(node, width, height, renderer, &mut timings, &mut ctx)?;
        let outcome = RenderOutcome {
            next: if has_active_animations {
                NextFrame::Replay
            } else {
                NextFrame::Idle
            },
            host_frame_delay_ms: result.next_frame_delay_ms,
        };
        Ok(TreeFrame { result, outcome })
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
    use crate::overlay::{FrameKind, NextFrame, TouchEvent};
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

    /// An indeterminate bar, which the renderer animates on its own.
    fn animated_tree() -> TreeNode {
        TreeNode::ProgressBar {
            touch_key: None,
            track_h: 8.0,
            mode: bmc_wasm_protocol::ProgressKind::Indeterminate,
            fraction: 0.0,
            active: true,
            fill_color: Color::from_rgba(0, 255, 0, 255),
            track_color: Color::from_rgba(80, 80, 80, 255),
            bg_color: Color::from_rgba(0, 0, 0, 0),
            skin: None,
        }
    }

    /// Render one frame, counting how often the tree was built.
    fn frame(
        ui: &mut TreeUi,
        kind: FrameKind,
        size: (u32, u32),
        builds: &mut usize,
    ) -> super::TreeFrame {
        ui.render(kind, size, 16, &mut TestRenderer::default(), || {
            *builds += 1;
            animated_tree()
        })
        .expect("BUG: render must succeed")
    }

    #[test]
    fn touch_on_slider_reports_drag_fraction() {
        let mut ui = TreeUi::default();
        let mut r = TestRenderer::default();
        // Warm-up frame establishes the slider's hit region (regions are
        // prior-frame state); the touch is hit-tested on the next frame.
        ui.render(FrameKind::Fresh, (100, 20), 16, &mut r, slider_tree)
            .expect("BUG: warm-up render must succeed");
        // A 100px-wide frame; touch down at x≈75 → fraction ≈ 0.75.
        ui.push_touch(TouchEvent::Down {
            id: 0,
            x: 75.0,
            y: 10.0,
        });
        let result = ui
            .render(FrameKind::Fresh, (100, 20), 16, &mut r, slider_tree)
            .expect("BUG: hit-test render must succeed");
        let hit = result
            .result
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
            .render(FrameKind::Fresh, (100, 20), 16, &mut r, slider_tree)
            .expect("BUG: render must succeed");
        assert!(!result.result.drags.contains_key("brightness"));
    }

    #[test]
    fn coalesced_down_up_on_slider_reports_release_click() {
        let mut ui = TreeUi::default();
        let mut r = TestRenderer::default();
        // Warm-up frame establishes the slider's hit region.
        ui.render(FrameKind::Fresh, (100, 20), 16, &mut r, slider_tree)
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
            .render(FrameKind::Fresh, (100, 20), 16, &mut r, slider_tree)
            .expect("BUG: hit-test render must succeed");
        assert!(
            !result.result.drags.contains_key("brightness"),
            "a released touch must not report an active drag"
        );
        let hit = result
            .result
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

    #[test]
    fn an_animation_only_frame_replays_the_cached_tree() {
        let mut ui = TreeUi::default();
        let mut builds = 0;
        frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
        frame(&mut ui, FrameKind::AnimationOnly, (100, 20), &mut builds);
        assert_eq!(builds, 1, "the replayed frame must not build a tree");
        assert!(ui.replayed_last_frame());
    }

    #[test]
    fn a_fresh_frame_builds_a_new_tree() {
        let mut ui = TreeUi::default();
        let mut builds = 0;
        frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
        frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
        assert_eq!(builds, 2);
        assert!(!ui.replayed_last_frame());
    }

    #[test]
    fn an_animation_only_frame_builds_without_a_cached_tree() {
        let mut ui = TreeUi::default();
        let mut builds = 0;
        frame(&mut ui, FrameKind::AnimationOnly, (100, 20), &mut builds);
        assert_eq!(builds, 1);
    }

    #[test]
    fn a_size_change_builds_a_new_tree() {
        let mut ui = TreeUi::default();
        let mut builds = 0;
        frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
        frame(&mut ui, FrameKind::AnimationOnly, (200, 20), &mut builds);
        assert_eq!(builds, 2, "the cached tree was laid out for another size");
    }

    #[test]
    fn a_pending_touch_builds_a_new_tree() {
        let mut ui = TreeUi::default();
        let mut builds = 0;
        frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
        ui.push_touch(TouchEvent::Down {
            id: 0,
            x: 50.0,
            y: 10.0,
        });
        frame(&mut ui, FrameKind::AnimationOnly, (100, 20), &mut builds);
        assert_eq!(builds, 2);
    }

    #[test]
    fn clear_drops_the_cached_tree() {
        let mut ui = TreeUi::default();
        let mut builds = 0;
        frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
        ui.clear();
        frame(&mut ui, FrameKind::AnimationOnly, (100, 20), &mut builds);
        assert_eq!(builds, 2);
    }

    #[test]
    fn a_running_animation_is_reported_on_replay() {
        let mut ui = TreeUi::default();
        let mut builds = 0;
        frame(&mut ui, FrameKind::Fresh, (100, 20), &mut builds);
        let replayed = frame(&mut ui, FrameKind::AnimationOnly, (100, 20), &mut builds);
        assert_eq!(replayed.outcome.next, NextFrame::Replay);
    }

    #[test]
    fn a_still_tree_reports_no_animation() {
        let mut ui = TreeUi::default();
        let frame = ui
            .render(
                FrameKind::Fresh,
                (100, 20),
                16,
                &mut TestRenderer::default(),
                slider_tree,
            )
            .expect("BUG: render must succeed");
        assert_eq!(frame.outcome.next, NextFrame::Idle);
    }
}
