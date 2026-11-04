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
//! When the host keeps the target such a frame paints into,
//! it repaints only the regions that moved.

use std::time::{Duration, Instant};

use bmc_render::FrameTimings;
use bmc_render::partition::static_layer::{AssetGeneration, MissPolicy, StaticLayerCache};
use bmc_render::partition::{StaticBytesKey, tree_static_key};
use bmc_render::renderer::{LayerOwner, OverlayId, Renderer};
use bmc_render::repaint::{LayeredFrame, LayeredTree, TargetContents};
use bmc_render::tree::{Node, ProcessContext, TreeResult, WalkState, layout_and_render};

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
    walk: WalkState,
    cached: Option<CachedTree>,
    layered: LayeredTree,
    /// What [`Self::prepare_target`] told the host the next frame may find in its target.
    target: TargetContents,
    warned_unkeyed: bool,
    /// The instant the last frame's animations advanced to;
    /// `None` starts them from the next frame.
    last_render: Option<Instant>,
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
            walk: WalkState::default(),
            cached: None,
            layered: LayeredTree::new(overlay_layer()),
            target: TargetContents::Cleared,
            warned_unkeyed: false,
            last_render: Some(now),
            #[cfg(any(test, feature = "test-support"))]
            replayed_last_frame: false,
            #[cfg(test)]
            fail_next_walk: false,
        }
    }

    /// Whether the host may keep the target the next `kind` frame at `size` paints into.
    ///
    /// Asked before the host clears,
    /// which would wipe exactly the pixels a damage-tracked frame reuses.
    /// Only a replay of the cached tree over its static layer,
    /// with damage to scissor to, keeps them;
    /// the next [`Self::render`] holds the frame to the answer.
    #[expect(
        clippy::cast_precision_loss,
        reason = "display sizes are well below f32 mantissa precision"
    )]
    pub fn prepare_target(&mut self, kind: FrameKind, size: (u32, u32)) -> TargetContents {
        let replays = crate::SURFACE_LAYERING == Layering::Layered && self.replays(kind, size);
        self.target = self
            .layered
            .target_for(replays, size.0 as f32, size.1 as f32);
        self.target
    }

    /// Repaint every export buffer in full, as after a frame that painted no tree
    /// into its buffer: none may hold the frame the damage history describes.
    pub fn invalidate_target(&mut self) {
        self.target = TargetContents::Cleared;
        self.layered.owe_every_buffer();
    }

    /// Count on the layer no more and repaint every buffer in full,
    /// after a frame that may have left its target half painted.
    fn fail(&mut self) {
        self.target = TargetContents::Cleared;
        self.layered.fail();
    }

    /// Whether a `kind` frame at `size` replays the cached tree rather than building one.
    fn replays(&self, kind: FrameKind, size: (u32, u32)) -> bool {
        kind == FrameKind::AnimationOnly
            && !self.walk.interaction.has_pending_events()
            && self
                .cached
                .as_ref()
                .is_some_and(|cached| cached.size == size)
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
    /// Over a target [`Self::prepare_target`] kept, it repaints only what moved.
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
        let since_last = self
            .last_render
            .map_or(Duration::ZERO, |last| now.saturating_duration_since(last));
        let delta_ms = u32::try_from(since_last.as_millis()).unwrap_or(u32::MAX);
        self.last_render = Some(now);
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
        let replay = self.replays(frame, size);
        let target = std::mem::replace(&mut self.target, TargetContents::Cleared);
        if target == TargetContents::Preserved && !(replay && layering == Layering::Layered) {
            self.fail();
            anyhow::bail!(
                "BUG: a {frame:?} {layering:?} frame at {size:?} was not the replay \
                 its host kept the target for"
            );
        }
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
        let frame = match self.render_node(
            &mut tree, layering, target, !replay, delta_ms, renderer, intl,
        ) {
            Ok(frame) => frame,
            Err(error) => {
                self.fail();
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

    /// Drop the cached tree, so the next frame builds a new one,
    /// count on the layer no more, and repaint every export buffer in full, which a hide frees.
    pub fn clear(&mut self) {
        self.cached = None;
        self.fail();
    }

    /// [`Self::clear`], and start every animation and transition over from the next frame.
    /// The layer keeps its owner, so the next capture overwrites it,
    /// and touches carry on, so a tap under way still lands.
    pub fn restart(&mut self) {
        self.clear();
        self.walk.animation_states.clear();
        self.walk.transition_states.clear();
        self.last_render = None;
    }

    /// Free the static layer in `renderer` and capture again from the next frame,
    /// even after a miss stopped it.
    /// Call it once the surface is hidden, before a renderer is replaced,
    /// and at teardown, with the renderer's GL context current.
    pub fn release(&mut self, renderer: &mut dyn Renderer) {
        self.layered.release(renderer);
    }

    /// Whose static layer this tree's is in the renderer.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn static_layer_owner(&self) -> &LayerOwner {
        self.layered.static_layer().owner()
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

    /// What this frame does with the layer over a target holding `target`,
    /// keying the tree on its first layered frame.
    #[expect(
        clippy::too_many_arguments,
        reason = "the tree, the frame's layering, target, freshness, clock and wording, and the renderer"
    )]
    fn plan_layer(
        &mut self,
        renderer: &mut dyn Renderer,
        tree: &mut CachedTree,
        layering: Layering,
        target: TargetContents,
        fresh: bool,
        now_unix_secs: i64,
        intl: &bmc_intl::Intl,
        timings: &mut FrameTimings,
    ) -> LayeredFrame {
        if layering == Layering::Unlayered {
            return LayeredTree::begin_unlayered(fresh);
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
        let frame = self.walk.layer_frame(
            &tree.node,
            bytes,
            tree.size,
            AssetGeneration::Unchanging,
            fresh,
            now_unix_secs,
            intl,
        );
        #[expect(
            clippy::cast_precision_loss,
            reason = "display sizes are well below f32 mantissa precision"
        )]
        let (width, height) = (tree.size.0 as f32, tree.size.1 as f32);
        self.layered.begin(renderer, frame, target, width, height)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the frame's tree, how it is drawn, over what target, and in which words"
    )]
    fn render_node(
        &mut self,
        tree: &mut CachedTree,
        layering: Layering,
        target: TargetContents,
        fresh: bool,
        delta_ms: u32,
        renderer: &mut dyn Renderer,
        intl: &bmc_intl::Intl,
    ) -> anyhow::Result<TreeFrame> {
        // Processes the queued touches, so the pressed key below is the one this frame paints.
        self.walk.interaction.begin_frame();
        let mut timings = FrameTimings::default();
        // Host-side and not capture-replayed, so the real wall clock
        // is correct for any RelativeTimeLive the system overlay renders.
        let now_unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs().cast_signed());
        let frame = self.plan_layer(
            renderer,
            tree,
            layering,
            target,
            fresh,
            now_unix_secs,
            intl,
            &mut timings,
        );
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_walk) {
            anyhow::bail!("the walk failed on purpose");
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "display sizes are well below f32 mantissa precision"
        )]
        let (width, height) = (tree.size.0 as f32, tree.size.1 as f32);
        let mut ctx = ProcessContext {
            static_layer: self.layered.layer_use(&frame),
            damage_rects: frame.damage(),
            stroke_outlines: bmc_render::tree::debug_layout_enabled(),
            ..self.walk.next_context(delta_ms, now_unix_secs, intl)
        };
        let (result, has_active_animations) =
            layout_and_render(&tree.node, width, height, renderer, &mut timings, &mut ctx)?;
        let repaint = self.layered.finish(
            renderer,
            frame,
            result.static_layer,
            &result.dynamic_rects,
            AssetGeneration::Unchanging,
        );
        Ok(TreeFrame {
            profile: TreeFrameProfile {
                static_layer: result.static_layer,
                repainted_px: whole_pixels(repaint.area(width, height)),
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
        self.walk.interaction.push_event(mapped);
    }

    /// Cancel the active touch without discarding the last rendered hit regions.
    pub fn cancel_touch(&mut self) {
        self.walk.interaction.cancel_touch();
    }

    /// Whether an element with `key` is currently pressed (one-frame latency,
    /// matching the WASM host). Backs the hold-to-confirm buttons.
    #[must_use]
    pub fn is_pressed(&self, key: &str) -> bool {
        self.walk.interaction.is_pressed(key)
    }

    /// Where the last render laid out the element with `key`, if it drew one.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn element_bounds(&self, key: &str) -> Option<bmc_render::interaction::Rect> {
        self.walk.interaction.element_bounds(key)
    }
}

/// `area` in whole pixels, for the host's profiling.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "an area on a display surface fits usize"
)]
fn whole_pixels(area: f32) -> usize {
    area.round().max(0.0) as usize
}

#[cfg(test)]
mod tests;
