// Copyright (C) 2026  Braiins Systems s.r.o.

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
        let frame_counter = self.frame_counter;
        self.frame_counter = self.frame_counter.wrapping_add(1);
        let mut timings = FrameTimings::default();
        let mut ctx = ProcessContext {
            interaction: &mut self.interaction,
            modal_states: &mut self.modal_states,
            scroll_states: &mut self.scroll_states,
            animation_states: &mut self.animation_states,
            transition_states: &mut self.transition_states,
            taffy: &mut self.taffy,
            frame_counter,
            delta_ms,
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
}
