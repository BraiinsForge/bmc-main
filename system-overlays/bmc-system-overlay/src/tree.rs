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
    #[cfg(any(test, feature = "test-support"))]
    replayed_last_frame: bool,
    /// Fails the next walk once its layer is planned, as no real tree makes taffy fail.
    #[cfg(test)]
    fail_next_walk: bool,
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
            rel_time: RelTimeWords::new(),
            static_layer: overlay_layer(),
            warned_unkeyed: false,
            #[cfg(any(test, feature = "test-support"))]
            replayed_last_frame: false,
            #[cfg(test)]
            fail_next_walk: false,
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
    ///
    /// A [`Layering::Layered`] frame draws the static half from the layer
    /// while its key holds, and captures it again when it does not.
    /// `intl` words the labels the renderer formats itself, such as a relative time.
    #[expect(
        clippy::too_many_arguments,
        reason = "the frame's kind, layering, size, clock, target and wording"
    )]
    pub fn render(
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
mod tests {
    use super::{Layering, TreeUi};
    use crate::overlay::{FrameKind, NextFrame, TouchEvent};
    use crate::test_support::TestRenderer;
    use bmc_render::renderer::test_support::ShapingRecorder;
    use bmc_render::test_tracing::counting_warns;
    use bmc_render::tree::Node;
    use bmc_render::tree::StaticLayer;
    use bmc_render::ui::{Color, PropsData};

    fn english() -> bmc_intl::Intl {
        bmc_intl::formatter(bmc_intl::Language::default())
    }

    /// A full-width ProgressBar carrying the brightness touch key.
    fn slider_tree() -> Node {
        Node::ProgressBar {
            touch_key: Some("brightness".to_owned()),
            track_h: 8.0,
            mode: bmc_render::tree::ProgressMode::Slider(0.0),
            active: true,
            fill_color: Color::from_rgba(0, 255, 0, 255),
            track_color: Color::from_rgba(80, 80, 80, 255),
            bg_color: Color::from_rgba(0, 0, 0, 0),
            skin: None,
        }
    }

    /// An indeterminate bar, which the renderer animates on its own.
    fn animated_tree() -> Node {
        Node::ProgressBar {
            touch_key: None,
            track_h: 8.0,
            mode: bmc_render::tree::ProgressMode::Indeterminate,
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
        ui.render(
            kind,
            Layering::Unlayered,
            size,
            16,
            &mut TestRenderer::default(),
            &english(),
            || {
                *builds += 1;
                animated_tree()
            },
        )
        .expect("BUG: render must succeed")
    }

    #[test]
    fn touch_on_slider_reports_drag_fraction() {
        let mut ui = TreeUi::default();
        let mut r = TestRenderer::default();
        // Warm-up frame establishes the slider's hit region (regions are
        // prior-frame state); the touch is hit-tested on the next frame.
        ui.render(
            FrameKind::Fresh,
            Layering::Unlayered,
            (100, 20),
            16,
            &mut r,
            &english(),
            slider_tree,
        )
        .expect("BUG: warm-up render must succeed");
        // A 100px-wide frame; touch down at x≈75 → fraction ≈ 0.75.
        ui.push_touch(TouchEvent::Down {
            id: 0,
            x: 75.0,
            y: 10.0,
        });
        let result = ui
            .render(
                FrameKind::Fresh,
                Layering::Unlayered,
                (100, 20),
                16,
                &mut r,
                &english(),
                slider_tree,
            )
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
            .render(
                FrameKind::Fresh,
                Layering::Unlayered,
                (100, 20),
                16,
                &mut r,
                &english(),
                slider_tree,
            )
            .expect("BUG: render must succeed");
        assert!(!result.result.drags.contains_key("brightness"));
    }

    #[test]
    fn coalesced_down_up_on_slider_reports_release_click() {
        let mut ui = TreeUi::default();
        let mut r = TestRenderer::default();
        // Warm-up frame establishes the slider's hit region.
        ui.render(
            FrameKind::Fresh,
            Layering::Unlayered,
            (100, 20),
            16,
            &mut r,
            &english(),
            slider_tree,
        )
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
            .render(
                FrameKind::Fresh,
                Layering::Unlayered,
                (100, 20),
                16,
                &mut r,
                &english(),
                slider_tree,
            )
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
        assert_eq!(replayed.outcome().next, NextFrame::Replay);
    }

    #[test]
    fn a_still_tree_reports_no_animation() {
        let mut ui = TreeUi::default();
        let frame = ui
            .render(
                FrameKind::Fresh,
                Layering::Unlayered,
                (100, 20),
                16,
                &mut TestRenderer::default(),
                &english(),
                slider_tree,
            )
            .expect("BUG: render must succeed");
        assert_eq!(frame.outcome().next, NextFrame::Idle);
    }

    const SIZE: (u32, u32) = (100, 20);
    const GREY: Color = Color::from_rgb(40, 40, 40);
    const BLUE: Color = Color::from_rgb(0, 0, 200);

    /// A static panel fill around `bar`, which moves or takes touches.
    fn panel(background: Color, bar: Node) -> Node {
        Node::Column(
            PropsData {
                background,
                ..PropsData::default()
            },
            vec![bar],
        )
    }

    /// A static fill laid over the moving bar, so the layer beneath it captures nothing.
    fn fill_over_the_bar() -> Node {
        let over = PropsData {
            inset_top: 0.0,
            inset_left: 0.0,
            width: 100.0,
            height: 20.0,
            background: GREY,
            ..PropsData::default()
        };
        Node::Column(
            PropsData::default(),
            vec![animated_tree(), Node::Column(over, Vec::new())],
        )
    }

    fn layers() -> ShapingRecorder {
        ShapingRecorder::new(100.0, 20.0).with_static_layers()
    }

    fn layered(
        ui: &mut TreeUi,
        renderer: &mut ShapingRecorder,
        kind: FrameKind,
        size: (u32, u32),
        tree: &Node,
    ) -> StaticLayer {
        ui.render(
            kind,
            Layering::Layered,
            size,
            16,
            renderer,
            &english(),
            || tree.clone(),
        )
        .expect("BUG: render must succeed")
        .profile
        .static_layer
    }

    #[cfg(feature = "frame-timings")]
    #[test]
    fn only_the_frame_that_keys_the_tree_reports_its_cost() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        // Enough labels that encoding and decoding the tree takes well over a microsecond.
        let labels = (0..200).map(|i| {
            bmc_render::ui::text(format!("label {i}"), bmc_render::ui::TextStyle::default())
        });
        let tree = Node::Column(
            PropsData {
                background: GREY,
                ..PropsData::default()
            },
            labels.collect(),
        );
        let mut key_us = |kind| {
            ui.render(
                kind,
                Layering::Layered,
                SIZE,
                16,
                &mut r,
                &english(),
                || tree.clone(),
            )
            .expect("BUG: render must succeed")
            .profile
            .timings
            .deserialize_us
        };
        assert!(
            key_us(FrameKind::Fresh) > 0,
            "the fresh frame keys the tree"
        );
        assert_eq!(
            key_us(FrameKind::AnimationOnly),
            0,
            "a replay reuses the key the tree was given"
        );
    }

    #[test]
    fn a_layered_frame_captures_the_static_layer() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let tree = panel(GREY, animated_tree());
        let outcome = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        assert_eq!(outcome, StaticLayer::Captured { empty: false });
        assert_eq!(r.static_layer_owners(), [ui.static_layer_owner().clone()]);
    }

    #[test]
    fn an_unlayered_frame_paints_the_tree_without_a_layer() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let outcome = ui
            .render(
                FrameKind::Fresh,
                Layering::Unlayered,
                SIZE,
                16,
                &mut r,
                &english(),
                || panel(GREY, animated_tree()),
            )
            .expect("BUG: render must succeed")
            .profile
            .static_layer;
        assert_eq!(outcome, StaticLayer::Unused);
        assert!(r.static_layer_owners().is_empty());
    }

    #[test]
    fn a_fresh_tree_with_the_same_static_half_reuses_the_layer() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let tree = panel(GREY, animated_tree());
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        let outcome = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        assert_eq!(outcome, StaticLayer::Blitted);
    }

    #[test]
    fn a_replayed_tree_reuses_the_layer() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let tree = panel(GREY, animated_tree());
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        let outcome = layered(&mut ui, &mut r, FrameKind::AnimationOnly, SIZE, &tree);
        assert_eq!(outcome, StaticLayer::Blitted);
    }

    #[test]
    fn a_static_edit_captures_again() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        layered(
            &mut ui,
            &mut r,
            FrameKind::Fresh,
            SIZE,
            &panel(GREY, animated_tree()),
        );
        let edited = panel(BLUE, animated_tree());
        let outcome = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &edited);
        assert_eq!(outcome, StaticLayer::Captured { empty: false });
    }

    /// The panel around a live relative time anchored `ago_secs` before now,
    /// whose label sizes the static fill around it.
    fn panel_with_age(ago_secs: i64) -> Node {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("BUG: the clock is past the epoch")
            .as_secs()
            .cast_signed();
        let age = bmc_render::ui::relative_time_live(
            bmc_render::ui::SystemTime {
                unix_secs: now - ago_secs,
            },
            bmc_render::ui::RelTimeFormat::try_from(0).expect("BUG: 0 is a valid format"),
            bmc_render::ui::RelTimeClamp::default(),
            bmc_render::ui::TextStyle::default(),
        );
        Node::Row(
            PropsData {
                background: GREY,
                ..PropsData::default()
            },
            vec![age],
        )
    }

    #[test]
    fn a_layout_change_alone_captures_again() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let recent = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &panel_with_age(5));
        let old = layered(
            &mut ui,
            &mut r,
            FrameKind::Fresh,
            SIZE,
            &panel_with_age(3_600),
        );
        assert_eq!(
            [recent, old],
            [StaticLayer::Captured { empty: false }; 2],
            "a wider label moves the fill it sits in, though the static bytes did not change"
        );
    }

    #[test]
    fn a_size_change_captures_again() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let tree = panel(GREY, animated_tree());
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        let outcome = layered(&mut ui, &mut r, FrameKind::AnimationOnly, (200, 20), &tree);
        assert_eq!(outcome, StaticLayer::Captured { empty: false });
    }

    #[test]
    fn a_press_captures_again_and_holds_while_it_lasts() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let tree = panel(GREY, slider_tree());
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        ui.push_touch(TouchEvent::Down {
            id: 0,
            x: 50.0,
            y: 4.0,
        });
        let pressed = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        assert!(
            ui.is_pressed("brightness"),
            "the touch must land on the slider"
        );
        assert_eq!(pressed, StaticLayer::Captured { empty: false });
        let held = layered(&mut ui, &mut r, FrameKind::AnimationOnly, SIZE, &tree);
        assert_eq!(held, StaticLayer::Blitted);
    }

    #[test]
    fn a_cancelled_press_captures_again_on_replay() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let tree = panel(GREY, slider_tree());
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        ui.push_touch(TouchEvent::Down {
            id: 0,
            x: 50.0,
            y: 4.0,
        });
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        ui.cancel_touch();
        let outcome = layered(&mut ui, &mut r, FrameKind::AnimationOnly, SIZE, &tree);
        assert!(ui.replayed_last_frame());
        assert_eq!(outcome, StaticLayer::Captured { empty: false });
    }

    /// A drag across `tree`: a press, then a vertical move on the next frame.
    fn dragged(tree: &Node) -> StaticLayer {
        let mut ui = TreeUi::default();
        let mut r = layers();
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, tree);
        ui.push_touch(TouchEvent::Down {
            id: 0,
            x: 50.0,
            y: 4.0,
        });
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, tree);
        ui.push_touch(TouchEvent::Motion {
            id: 0,
            x: 50.0,
            y: 6.0,
        });
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, tree)
    }

    #[test]
    fn a_drag_over_a_tree_without_a_scroll_container_reuses_the_layer() {
        assert_eq!(dragged(&panel(GREY, slider_tree())), StaticLayer::Blitted);
    }

    #[test]
    fn a_drag_over_a_scroll_container_captures_again() {
        let list = bmc_render::ui::scroll("list", PropsData::default(), [slider_tree()]);
        assert_eq!(
            dragged(&panel(GREY, list)),
            StaticLayer::Captured { empty: false }
        );
    }

    #[test]
    fn a_tree_without_static_content_frees_the_layer() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        layered(
            &mut ui,
            &mut r,
            FrameKind::Fresh,
            SIZE,
            &panel(GREY, animated_tree()),
        );
        let outcome = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &animated_tree());
        assert_eq!(outcome, StaticLayer::Unused);
        assert!(r.static_layer_owners().is_empty());
    }

    #[test]
    fn an_empty_capture_frees_the_layer_and_is_not_captured_again() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let tree = fill_over_the_bar();
        let first = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        assert_eq!(first, StaticLayer::Captured { empty: true });
        assert!(r.static_layer_owners().is_empty());
        let second = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        assert_eq!(second, StaticLayer::Unused);
    }

    #[test]
    fn returning_to_an_empty_capture_frees_the_layer_another_tree_left() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let empty = fill_over_the_bar();
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &empty);
        layered(
            &mut ui,
            &mut r,
            FrameKind::Fresh,
            SIZE,
            &panel(GREY, animated_tree()),
        );
        assert!(
            !r.static_layer_owners().is_empty(),
            "the panel holds a layer"
        );
        let outcome = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &empty);
        assert_eq!(outcome, StaticLayer::Unused);
        assert!(r.static_layer_owners().is_empty());
    }

    #[test]
    fn an_unkeyed_tree_paints_without_a_layer_and_frees_it() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        layered(
            &mut ui,
            &mut r,
            FrameKind::Fresh,
            SIZE,
            &panel(GREY, animated_tree()),
        );
        let negative_extent = panel(GREY, bmc_render::ui::spacer(-1.0));
        let outcome = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &negative_extent);
        assert_eq!(outcome, StaticLayer::Unused);
        assert!(r.static_layer_owners().is_empty());
    }

    #[test]
    fn an_unkeyed_run_warns_once_until_a_tree_keys_again() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let unkeyed = panel(GREY, bmc_render::ui::spacer(-1.0));
        let keyed = panel(GREY, animated_tree());
        let ((), warnings) = counting_warns(|_| {
            for _ in 0..3 {
                layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &unkeyed);
            }
        });
        assert_eq!(warnings, 1, "a run of unkeyed trees warns once");
        let ((), warnings) = counting_warns(|_| {
            layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &keyed);
            layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &unkeyed);
        });
        assert_eq!(warnings, 1, "a keyed tree re-arms the warning");
    }

    #[test]
    fn a_tree_built_unlayered_keys_on_its_first_layered_replay() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let tree = panel(GREY, animated_tree());
        ui.render(
            FrameKind::Fresh,
            Layering::Unlayered,
            SIZE,
            16,
            &mut r,
            &english(),
            || tree.clone(),
        )
        .expect("BUG: render must succeed");
        let outcome = layered(&mut ui, &mut r, FrameKind::AnimationOnly, SIZE, &tree);
        assert!(ui.replayed_last_frame());
        assert_eq!(outcome, StaticLayer::Captured { empty: false });
    }

    #[test]
    fn an_unlayered_frame_never_keys_its_tree() {
        let mut ui = TreeUi::default();
        let unkeyed = panel(GREY, bmc_render::ui::spacer(-1.0));
        let ((), warnings) = counting_warns(|_| {
            ui.render(
                FrameKind::Fresh,
                Layering::Unlayered,
                SIZE,
                16,
                &mut layers(),
                &english(),
                || unkeyed.clone(),
            )
            .expect("BUG: render must succeed");
        });
        assert_eq!(warnings, 0, "a tree never keyed cannot fail to key");
    }

    #[test]
    fn a_miss_stops_capturing_until_release() {
        let mut ui = TreeUi::default();
        let mut refusing = ShapingRecorder::new(100.0, 20.0);
        let tree = panel(GREY, animated_tree());
        let missed = layered(&mut ui, &mut refusing, FrameKind::Fresh, SIZE, &tree);
        assert_eq!(missed, StaticLayer::Missed);
        let stopped = layered(
            &mut ui,
            &mut refusing,
            FrameKind::AnimationOnly,
            SIZE,
            &tree,
        );
        assert!(ui.replayed_last_frame(), "a miss keeps the tree");
        assert_eq!(stopped, StaticLayer::Unused);
        ui.release(&mut refusing);
        let retried = layered(&mut ui, &mut refusing, FrameKind::Fresh, SIZE, &tree);
        assert_eq!(retried, StaticLayer::Missed);
    }

    #[test]
    fn a_failed_walk_stops_counting_on_the_layer() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let tree = panel(GREY, animated_tree());
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        ui.fail_next_walk = true;
        assert!(
            ui.render(
                FrameKind::Fresh,
                Layering::Layered,
                SIZE,
                16,
                &mut r,
                &english(),
                || { tree.clone() }
            )
            .is_err()
        );
        let after = layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        assert_eq!(
            after,
            StaticLayer::Captured { empty: false },
            "the failed walk may have left the layer half captured"
        );
    }

    #[test]
    fn release_frees_the_layer_and_the_next_frame_captures_it() {
        let mut ui = TreeUi::default();
        let mut r = layers();
        let tree = panel(GREY, animated_tree());
        layered(&mut ui, &mut r, FrameKind::Fresh, SIZE, &tree);
        ui.release(&mut r);
        assert!(r.static_layer_owners().is_empty());
        let outcome = layered(&mut ui, &mut r, FrameKind::AnimationOnly, SIZE, &tree);
        assert_eq!(outcome, StaticLayer::Captured { empty: false });
    }

    #[test]
    fn two_trees_capture_under_their_own_keys() {
        let (mut first, mut second) = (TreeUi::default(), TreeUi::default());
        let mut r = layers();
        layered(
            &mut first,
            &mut r,
            FrameKind::Fresh,
            SIZE,
            &panel(GREY, animated_tree()),
        );
        layered(
            &mut second,
            &mut r,
            FrameKind::Fresh,
            SIZE,
            &panel(BLUE, animated_tree()),
        );
        assert_ne!(first.static_layer_owner(), second.static_layer_owner());
        assert_eq!(r.static_layer_owners().len(), 2);
        let outcome = layered(
            &mut first,
            &mut r,
            FrameKind::AnimationOnly,
            SIZE,
            &panel(GREY, animated_tree()),
        );
        assert_eq!(outcome, StaticLayer::Blitted);
    }
}
