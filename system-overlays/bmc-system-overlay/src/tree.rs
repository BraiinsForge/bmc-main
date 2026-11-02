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

//! Retained per-frame state for rendering a `bmc_render` `Node`.
//!
//! [`TreeUi`] owns the persistent interaction/animation/transition caches that
//! [`layout_and_render`] reads and updates, so an overlay can draw a declarative
//! UI tree against a [`Renderer`] without threading that state by hand.
//! It also keeps the last tree it drew, so a frame where only animations move
//! replays it instead of building a new one,
//! and the tree's static half in a renderer layer,
//! so a frame where only dynamic content moves blits it back.

use std::collections::HashMap;
use std::time::Instant;

use bmc_render::components::RelTimeWords;
use bmc_render::interaction::InteractionState;
use bmc_render::partition::static_layer::{
    AssetGeneration, FrameTree, LayerKey, LayerPlan, MissPolicy, StaticLayerCache,
};
use bmc_render::partition::{HostPaintState, StaticBytesKey, tree_static_key};
use bmc_render::renderer::{LayerOwner, OverlayId, Renderer};
use bmc_render::tree::{Node, NodeContext, ProcessContext, TreeResult, layout_and_render};
use bmc_render::{
    AnimationState, FrameTimings, ModalState, ScrollState, TransitionState, TransitionStateKey,
};
use taffy::prelude::TaffyTree;

use crate::overlay::{FrameKind, NextFrame, RenderOutcome, TreeFrameProfile};

/// Whether a frame may draw its tree's static half from the layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layering {
    /// Draw the static half from the layer: the host cleared the frame to its background.
    Layered,
    /// Paint the tree in full and leave the layer alone.
    /// For a frame that holds the caller's own drawing, as the gallery's backdrop,
    /// which a layer blit would replace, or one that must leave no layer behind, as a prewarm.
    Unlayered,
}

/// One frame an overlay draws: why it renders, whether it may use the layer, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayFrame {
    pub kind: FrameKind,
    pub layering: Layering,
    /// The instant the frame's animations advance to.
    pub now: Instant,
}

impl OverlayFrame {
    /// A frame of the host's surface, drawn now.
    #[must_use]
    pub fn on_surface(kind: FrameKind) -> Self {
        Self {
            kind,
            layering: crate::SURFACE_LAYERING,
            now: Instant::now(),
        }
    }

    /// A frame kept off the layer, drawn now.
    #[must_use]
    pub fn unlayered(kind: FrameKind) -> Self {
        Self {
            kind,
            layering: Layering::Unlayered,
            now: Instant::now(),
        }
    }
}

/// The last successfully rendered tree, the size it was laid out at, and its static key.
struct CachedTree {
    node: Node,
    size: (u32, u32),
    key: TreeKey,
}

/// What the wire format makes of a tree's static half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeKey {
    Keyed(StaticBytesKey),
    /// The wire format cannot carry the tree, or the decoder refuses it.
    Unkeyable,
    /// Not keyed yet. Keying costs a full encode and decode of the tree,
    /// so it waits for the first frame that may use the layer.
    NotKeyed,
}

/// A layer cache owned by this overlay tree alone,
/// so two overlays of one kind never capture over each other.
///
/// After a miss it waits for a release, as an overlay's trees change often:
/// a progress step is a fresh tree.
fn overlay_layer() -> StaticLayerCache {
    StaticLayerCache::new(LayerOwner::Overlay(OverlayId::next()), MissPolicy::Release)
}

/// A rendered frame: its interaction read-back, how it drew its tree,
/// and what it asks of the schedule.
#[derive(Debug)]
pub struct TreeFrame {
    pub result: TreeResult,
    pub profile: TreeFrameProfile,
    next: NextFrame,
    host_frame_delay_ms: Option<u32>,
}

impl TreeFrame {
    /// What the frame asks of the schedule, with its profile for the host.
    pub fn outcome(&self) -> RenderOutcome {
        RenderOutcome {
            next: self.next,
            host_frame_delay_ms: self.host_frame_delay_ms,
            tree: Some(self.profile),
        }
    }
}

/// Holds the layout/animation/interaction context for rendering a `Node`.
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
    rel_time: RelTimeWords,
    static_layer: StaticLayerCache,
    warned_unkeyed: bool,
    /// The instant the last frame's animations advanced to.
    last_render: Instant,
    #[cfg(any(test, feature = "test-support"))]
    replayed_last_frame: bool,
    /// Fails the next walk once its layer is planned, as no real tree makes taffy fail.
    #[cfg(test)]
    fail_next_walk: bool,
}

#[cfg(test)]
impl Default for TreeUi {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl TreeUi {
    /// A tree whose first frame's animations advance from `now`.
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            interaction: InteractionState::new(),
            modal_states: HashMap::new(),
            scroll_states: HashMap::new(),
            animation_states: HashMap::new(),
            transition_states: HashMap::new(),
            taffy: TaffyTree::new(),
            frame_counter: 0,
            cached: None,
            rel_time: RelTimeWords::new(),
            static_layer: overlay_layer(),
            warned_unkeyed: false,
            last_render: now,
            #[cfg(any(test, feature = "test-support"))]
            replayed_last_frame: false,
            #[cfg(test)]
            fail_next_walk: false,
        }
    }

    /// Lay out and draw `frame` at `size` against `renderer`,
    /// advancing animations by the time since the last frame.
    ///
    /// An [`FrameKind::AnimationOnly`] frame replays the cached tree.
    /// `build` runs instead when the frame is fresh, nothing is cached,
    /// the size changed, or touch events are pending, as widgets do.
    /// A failed render drops the cache, so the next frame builds again.
    ///
    /// A [`Layering::Layered`] frame draws the static half from the layer
    /// while its key holds, and captures it again when it does not.
    /// `intl` words the labels the renderer formats itself, such as a relative time.
    pub fn render(
        &mut self,
        frame: OverlayFrame,
        size: (u32, u32),
        renderer: &mut dyn Renderer,
        intl: &bmc_intl::Intl,
        build: impl FnOnce() -> Node,
    ) -> anyhow::Result<TreeFrame> {
        let OverlayFrame {
            kind,
            layering,
            now,
        } = frame;
        let delta_ms = u32::try_from(now.saturating_duration_since(self.last_render).as_millis())
            .unwrap_or(u32::MAX);
        self.last_render = now;
        self.render_after(kind, layering, size, delta_ms, renderer, intl, build)
    }

    /// [`Self::render`], `delta_ms` after the last frame.
    #[expect(
        clippy::too_many_arguments,
        reason = "the frame's kind, layering, size, clock, target and wording"
    )]
    pub(crate) fn render_after(
        &mut self,
        frame: FrameKind,
        layering: Layering,
        size: (u32, u32),
        delta_ms: u32,
        renderer: &mut dyn Renderer,
        intl: &bmc_intl::Intl,
        build: impl FnOnce() -> Node,
    ) -> anyhow::Result<TreeFrame> {
        let replay = frame == FrameKind::AnimationOnly
            && !self.interaction.has_pending_events()
            && self
                .cached
                .as_ref()
                .is_some_and(|cached| cached.size == size);
        let mut tree = match self.cached.take() {
            Some(cached) if replay => cached,
            _ => CachedTree {
                node: build(),
                size,
                key: TreeKey::NotKeyed,
            },
        };
        #[cfg(any(test, feature = "test-support"))]
        {
            self.replayed_last_frame = false;
        }
        let frame = match self.render_node(&mut tree, layering, !replay, delta_ms, renderer, intl) {
            Ok(frame) => frame,
            Err(error) => {
                self.static_layer.forget();
                return Err(error);
            }
        };
        #[cfg(any(test, feature = "test-support"))]
        {
            self.replayed_last_frame = replay;
        }
        self.cached = Some(tree);
        Ok(frame)
    }

    /// Drop the cached tree, so the next frame builds a new one.
    pub fn clear(&mut self) {
        self.cached = None;
        self.static_layer.forget();
    }

    /// Free the static layer in `renderer` and capture again from the next frame,
    /// even after a miss stopped it.
    /// Call it once the surface is hidden, before a renderer is replaced,
    /// and at teardown, with the renderer's GL context current.
    pub fn release(&mut self, renderer: &mut dyn Renderer) {
        self.static_layer.release(renderer);
    }

    /// Whose static layer this tree's is in the renderer.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn static_layer_owner(&self) -> &LayerOwner {
        self.static_layer.owner()
    }

    /// Whether the last [`Self::render`] replayed the cached tree and drew it.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn replayed_last_frame(&self) -> bool {
        self.replayed_last_frame
    }

    /// Key `node`'s static half, warning once for each run of trees that have no key.
    fn static_key(&mut self, node: &Node, timings: &mut FrameTimings) -> Option<StaticBytesKey> {
        #[cfg(feature = "frame-timings")]
        let keying = std::time::Instant::now();
        let key = match tree_static_key(node) {
            Ok(key) => {
                self.warned_unkeyed = false;
                Some(key)
            }
            Err(error) => {
                if !std::mem::replace(&mut self.warned_unkeyed, true) {
                    tracing::warn!(
                        "overlay tree has no static key, painting it uncached: {error:#}"
                    );
                }
                None
            }
        };
        #[cfg(feature = "frame-timings")]
        {
            timings.deserialize_us =
                u32::try_from(keying.elapsed().as_micros()).unwrap_or(u32::MAX);
        }
        #[cfg(not(feature = "frame-timings"))]
        let _ = timings;
        key
    }

    /// What this frame does with the layer, keying the tree on its first layered frame.
    #[expect(
        clippy::too_many_arguments,
        reason = "the tree, the frame's layering, freshness, clock and wording, and the renderer"
    )]
    fn plan_layer(
        &mut self,
        renderer: &mut dyn Renderer,
        tree: &mut CachedTree,
        layering: Layering,
        fresh: bool,
        now_unix_secs: i64,
        intl: &bmc_intl::Intl,
        timings: &mut FrameTimings,
    ) -> LayerPlan {
        if layering == Layering::Unlayered {
            return LayerPlan::Ignore;
        }
        let bytes = match tree.key {
            TreeKey::Keyed(bytes) => Some(bytes),
            TreeKey::Unkeyable => None,
            TreeKey::NotKeyed => {
                let bytes = self.static_key(&tree.node, timings);
                tree.key = bytes.map_or(TreeKey::Unkeyable, TreeKey::Keyed);
                bytes
            }
        };
        let host = HostPaintState {
            pressed_key: self.interaction.pressed_key(),
            scroll_offsets: &self.scroll_states,
        };
        let key = bytes.map(|bytes| {
            LayerKey::new(
                &tree.node,
                bytes,
                &host,
                tree.size,
                AssetGeneration::Unchanging,
                now_unix_secs,
                intl,
            )
        });
        let dragging = self.interaction.get_global_scroll_delta() != 0.0;
        let frame = FrameTree::new(&tree.node, key, dragging, fresh);
        self.static_layer.plan(renderer, frame)
    }

    fn render_node(
        &mut self,
        tree: &mut CachedTree,
        layering: Layering,
        fresh: bool,
        delta_ms: u32,
        renderer: &mut dyn Renderer,
        intl: &bmc_intl::Intl,
    ) -> anyhow::Result<TreeFrame> {
        // Processes the queued touches, so the pressed key below is the one this frame paints.
        self.interaction.begin_frame();
        let frame_counter = self.frame_counter;
        self.frame_counter = self.frame_counter.wrapping_add(1);
        let mut timings = FrameTimings::default();
        // Host-side and not capture-replayed, so the real wall clock
        // is correct for any RelativeTimeLive the system overlay renders.
        let now_unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs().cast_signed());
        let plan = self.plan_layer(
            renderer,
            tree,
            layering,
            fresh,
            now_unix_secs,
            intl,
            &mut timings,
        );
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_walk) {
            anyhow::bail!("the walk failed on purpose");
        }
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
            intl,
            rel_time: &self.rel_time,
            emit: bmc_render::tree::EmitMode::All,
            static_layer: self.static_layer.layer_use(plan),
            layout_outlines: false,
            stroke_outlines: bmc_render::tree::debug_layout_enabled(),
            damage_rects: &[],
        };
        #[expect(
            clippy::cast_precision_loss,
            reason = "display sizes are well below f32 mantissa precision"
        )]
        let (width, height) = (tree.size.0 as f32, tree.size.1 as f32);
        let (result, has_active_animations) =
            layout_and_render(&tree.node, width, height, renderer, &mut timings, &mut ctx)?;
        self.static_layer.commit(
            renderer,
            plan,
            result.static_layer,
            AssetGeneration::Unchanging,
        );
        Ok(TreeFrame {
            profile: TreeFrameProfile {
                static_layer: result.static_layer,
                timings,
            },
            next: if has_active_animations {
                NextFrame::Replay
            } else {
                NextFrame::Idle
            },
            host_frame_delay_ms: result.next_frame_delay_ms,
            result,
        })
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

    /// Where the last render laid out the element with `key`, if it drew one.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn element_bounds(&self, key: &str) -> Option<bmc_render::interaction::Rect> {
        self.interaction.element_bounds(key)
    }
}

#[cfg(test)]
mod tests;
