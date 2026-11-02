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

//! What a tree's walks keep from one frame to the next.

use std::collections::HashMap;

use taffy::TaffyTree;

use super::{EmitMode, LayerUse, NodeContext, ProcessContext};
use crate::components::RelTimeWords;
use crate::interaction::InteractionState;
use crate::partition::HostPaintState;
use crate::partition::StaticBytesKey;
use crate::partition::static_layer::{AssetGeneration, FrameTree, LayerKey};
use crate::{AnimationState, ModalState, ScrollState, TransitionState, TransitionStateKey};

/// The interaction, the modal, scroll, animation and transition state,
/// the layout tree, the worded relative times and the frame counter
/// that one tree's walks carry from frame to frame.
#[derive(Debug)]
pub struct WalkState {
    pub interaction: InteractionState,
    pub modal_states: HashMap<String, ModalState>,
    pub scroll_states: HashMap<String, ScrollState>,
    pub animation_states: HashMap<u64, AnimationState>,
    pub transition_states: HashMap<TransitionStateKey, TransitionState>,
    pub taffy: TaffyTree<NodeContext>,
    pub rel_time: RelTimeWords,
    /// The number the next walk is told it is. A walk reusing a number replays
    /// the last layout over the new one, so a caller counts every frame.
    pub frame_counter: u64,
}

impl Default for WalkState {
    fn default() -> Self {
        Self {
            interaction: InteractionState::new(),
            modal_states: HashMap::new(),
            scroll_states: HashMap::new(),
            animation_states: HashMap::new(),
            transition_states: HashMap::new(),
            taffy: TaffyTree::new(),
            rel_time: RelTimeWords::new(),
            frame_counter: 0,
        }
    }
}

impl WalkState {
    /// A walk's context under the current frame number, worded by `intl`: a full frame,
    /// off the layer and without outlines, for the caller to adjust.
    pub fn context<'a>(
        &'a mut self,
        delta_ms: u32,
        now_unix_secs: i64,
        intl: &'a bmc_intl::Intl,
    ) -> ProcessContext<'a> {
        ProcessContext {
            interaction: &mut self.interaction,
            modal_states: &mut self.modal_states,
            scroll_states: &mut self.scroll_states,
            animation_states: &mut self.animation_states,
            transition_states: &mut self.transition_states,
            taffy: &mut self.taffy,
            frame_counter: self.frame_counter,
            delta_ms,
            now_unix_secs,
            intl,
            rel_time: &self.rel_time,
            emit: EmitMode::All,
            damage_rects: &[],
            static_layer: LayerUse::Ignore,
            layout_outlines: false,
            stroke_outlines: false,
        }
    }

    /// [`Self::context`] under a frame number of its own, counting the walk.
    pub fn next_context<'a>(
        &'a mut self,
        delta_ms: u32,
        now_unix_secs: i64,
        intl: &'a bmc_intl::Intl,
    ) -> ProcessContext<'a> {
        let frame_counter = self.frame_counter;
        self.frame_counter = frame_counter.wrapping_add(1);
        ProcessContext {
            frame_counter,
            ..self.context(delta_ms, now_unix_secs, intl)
        }
    }

    /// What the host draws into the static half beyond the tree, for its layer key.
    #[must_use]
    pub fn paint_state(&self) -> HostPaintState<'_> {
        HostPaintState {
            pressed_key: self.interaction.pressed_key(),
            scroll_offsets: &self.scroll_states,
        }
    }

    /// What the static layer cache needs to know about a frame of `node`,
    /// keyed by its wire bytes `static_bytes` when the wire format can carry it.
    ///
    /// Call it before the walk, which advances the scroll offsets the key describes.
    #[expect(
        clippy::too_many_arguments,
        reason = "the tree, its key parts, the frame's freshness and the clock and wording"
    )]
    #[must_use]
    pub fn layer_frame(
        &self,
        node: &super::Node,
        static_bytes: Option<StaticBytesKey>,
        size: (u32, u32),
        assets: AssetGeneration,
        fresh: bool,
        now_unix_secs: i64,
        intl: &bmc_intl::Intl,
    ) -> FrameTree {
        let host = self.paint_state();
        let key = static_bytes
            .map(|bytes| LayerKey::new(node, bytes, &host, size, assets, now_unix_secs, intl));
        FrameTree::new(node, key, self.dragging(), fresh)
    }

    /// Whether a finger moved this frame.
    #[must_use]
    pub fn dragging(&self) -> bool {
        self.interaction.get_global_scroll_delta() != 0.0
    }
}
