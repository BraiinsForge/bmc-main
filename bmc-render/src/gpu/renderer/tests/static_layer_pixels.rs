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

//! A static layer only pays off if nobody can tell it is there: every frame
//! that captures, reuses or misses it has to match a full paint pixel for pixel,
//! over the transparent base overlays draw on as much as the opaque one widgets do.

use std::collections::HashMap;

use super::{FemtoVgRenderer, RenderTargetProbe};
use crate::FrameTimings;
use crate::interaction::{InteractionState, Rect};
use crate::renderer::prelude::*;
use crate::renderer::{FrameBackground, FrameClear};
use crate::test_harness::{GlHarness, create_readback_fbo, read_pixels_top_down};
use crate::tree::{
    AnimationDef, Draw, EmitMode, LayerUse, Node, ProcessContext, StaticLayer, TreeResult,
    layout_and_render,
};
use bmc_ui::{AnimProperty, Color, ColorSpace, Easing, Fill, LoopMode, PropsData, TextStyle};
use taffy::prelude::TaffyTree;

const W: u32 = 96;
const H: u32 = 64;
const FRAMES: u64 = 30;
const KEY: &str = "overlay:static";

/// Static text and a translucent circle under a fading circle,
/// with a static rectangle painted over the fade,
/// so the layer, the dynamic half and the band above it
/// all land on antialiased, partly transparent edges.
fn tree() -> Node {
    let fading = Draw::Modified {
        animations: vec![AnimationDef {
            property: AnimProperty::Alpha,
            from: 0.2,
            to: 0.9,
            duration_ms: 400,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(Draw::Circle {
            cx: 52.3,
            cy: 30.6,
            r: 13.7,
            fill: Fill::Solid(Color::from_rgba(255, 120, 0, 200)),
        }),
    };
    Node::Canvas {
        props: PropsData {
            width: W as f32,
            height: H as f32,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![
            Draw::Text {
                x: 3.0,
                y: 40.0,
                text: "Hash 42".to_owned(),
                style: TextStyle {
                    size: 18,
                    color: Color::from_rgba(255, 255, 255, 220),
                    ..TextStyle::default()
                },
            },
            Draw::Circle {
                cx: 34.5,
                cy: 28.2,
                r: 17.3,
                fill: Fill::Solid(Color::from_rgba(40, 160, 255, 140)),
            },
            fading,
            Draw::Rect {
                x: 60.5,
                y: 20.5,
                w: 21.3,
                h: 9.7,
                fill: Fill::Solid(Color::from_rgba(0, 255, 120, 90)),
            },
        ],
    }
}

/// How a run of frames treats the static layer.
#[derive(Debug, Clone, Copy)]
enum Pipeline {
    /// No layer: the reference every other pipeline must match.
    Full,
    /// Capture on the first frame, reuse on every later one.
    Cached,
    /// [`Self::Cached`] on a driver where the raw layer copy fails.
    RawCopyFails,
    /// [`Self::Cached`] with the layer dropped before every reuse.
    LayerLost,
    /// [`Self::Cached`] with the first frame on the other background,
    /// so the layer is captured over a surface the later frames no longer have.
    SurfaceChanged,
    /// A driver whose offscreen target is unusable, so every capture is refused.
    RefusedCapture,
}

/// What a frame finds in its target before painting.
#[derive(Debug, Clone, Copy)]
enum Target {
    /// Cleared to the background every frame.
    Cleared,
    /// Cleared on the first frame only, then left holding the previous frame,
    /// where a blended layer or a skipped clear would let old pixels show.
    Kept,
    /// [`Self::Kept`], with each reuse repainting only the previous frame's dynamic rects,
    /// as a damage-tracked host does.
    Damaged,
}

struct Slot {
    interaction: InteractionState,
    modal_states: HashMap<String, crate::ModalState>,
    scroll_states: HashMap<String, crate::ScrollState>,
    animation_states: HashMap<u64, crate::AnimationState>,
    transition_states: HashMap<crate::TransitionStateKey, crate::TransitionState>,
    taffy: TaffyTree<crate::tree::NodeContext>,
}

impl Slot {
    fn new() -> Self {
        Self {
            interaction: InteractionState::new(),
            modal_states: HashMap::new(),
            scroll_states: HashMap::new(),
            animation_states: HashMap::new(),
            transition_states: HashMap::new(),
            taffy: TaffyTree::new(),
        }
    }

    fn frame(
        &mut self,
        renderer: &mut FemtoVgRenderer,
        tree: &Node,
        frame_counter: u64,
        static_layer: LayerUse,
        damage_rects: &[Rect],
    ) -> TreeResult {
        let mut ctx = ProcessContext {
            interaction: &mut self.interaction,
            modal_states: &mut self.modal_states,
            scroll_states: &mut self.scroll_states,
            animation_states: &mut self.animation_states,
            transition_states: &mut self.transition_states,
            taffy: &mut self.taffy,
            frame_counter,
            delta_ms: 16,
            now_unix_secs: 0,
            emit: EmitMode::All,
            static_layer,
            static_layer_key: KEY,
            layout_outlines: false,
            stroke_outlines: false,
            damage_rects,
        };
        let (result, _) = layout_and_render(
            tree,
            W as f32,
            H as f32,
            renderer,
            &mut FrameTimings::default(),
            &mut ctx,
        )
        .expect("BUG: laying out a single canvas must succeed");
        result
    }
}

fn other(background: FrameBackground) -> FrameBackground {
    match background {
        FrameBackground::OpaqueBlack => FrameBackground::TransparentBlack,
        FrameBackground::TransparentBlack => FrameBackground::OpaqueBlack,
    }
}

/// Each frame's pixels, with what it did with the layer.
type Frames = Vec<(Vec<[u8; 4]>, StaticLayer)>;

/// Paint [`FRAMES`] frames of [`tree`] and return each one's pixels
/// with what it did with the layer.
fn run(background: FrameBackground, pipeline: Pipeline, target: Target) -> Frames {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    renderer.refuse_raw_blit = matches!(pipeline, Pipeline::RawCopyFails);
    if matches!(pipeline, Pipeline::RefusedCapture) {
        renderer.render_to_texture = RenderTargetProbe::FramebufferIncomplete(0);
    }
    let tree = tree();
    let mut slot = Slot::new();
    let mut last_dynamic_rects: Vec<Rect> = Vec::new();

    let frames = (1..=FRAMES)
        .map(|frame| {
            let clear = match target {
                Target::Kept | Target::Damaged if frame > 1 => FrameClear::Keep,
                Target::Cleared | Target::Kept | Target::Damaged => FrameClear::Clear,
            };
            let surface = match pipeline {
                Pipeline::SurfaceChanged if frame == 1 => other(background),
                Pipeline::Full
                | Pipeline::Cached
                | Pipeline::RawCopyFails
                | Pipeline::LayerLost
                | Pipeline::SurfaceChanged
                | Pipeline::RefusedCapture => background,
            };
            renderer.begin_frame_with_clear(W, H, 1.0, surface, clear);
            let static_layer = match pipeline {
                Pipeline::Full => LayerUse::Ignore,
                Pipeline::Cached
                | Pipeline::RawCopyFails
                | Pipeline::LayerLost
                | Pipeline::SurfaceChanged
                    if frame == 1 =>
                {
                    LayerUse::Capture
                }
                Pipeline::Cached | Pipeline::RawCopyFails | Pipeline::SurfaceChanged => {
                    LayerUse::Reuse
                }
                Pipeline::LayerLost => {
                    renderer.invalidate_static_layer(KEY);
                    LayerUse::Reuse
                }
                Pipeline::RefusedCapture => LayerUse::Capture,
            };
            let damage: &[Rect] = match target {
                Target::Damaged if frame > 1 => {
                    assert!(
                        !last_dynamic_rects.is_empty(),
                        "BUG: the walk reported no dynamic rects, so the reuse would repaint everything"
                    );
                    &last_dynamic_rects
                }
                Target::Cleared | Target::Kept | Target::Damaged => &[],
            };
            let result = slot.frame(&mut renderer, &tree, frame, static_layer, damage);
            renderer.flush();
            last_dynamic_rects = result.dynamic_rects;
            (read_pixels_top_down(&harness.gl, fbo, W, H), result.static_layer)
        })
        .collect();
    drop(renderer);
    frames
}

fn assert_matches_full_paint(background: FrameBackground, pipeline: Pipeline, target: Target) {
    let reference = run(background, Pipeline::Full, Target::Cleared);
    let frames = run(background, pipeline, target);
    assert_frames_match(
        &reference,
        &frames,
        0,
        &format!("{pipeline:?} over {background:?} on a {target:?} target"),
    );
}

/// Compare every frame from `first` on, counting the pixels that differ:
/// the count tells a shifted edge from a rectangle painted over the frame.
fn assert_frames_match(reference: &Frames, frames: &Frames, first: usize, run: &str) {
    for (frame, ((expected, _), (actual, layer))) in
        reference.iter().zip(frames).enumerate().skip(first)
    {
        let differing = expected.iter().zip(actual).filter(|(e, a)| e != a).count();
        assert_eq!(
            differing, 0,
            "{run}: frame {frame} ({layer:?}) differs from a full paint in {differing} pixels"
        );
    }
}

fn layers(background: FrameBackground, pipeline: Pipeline, target: Target) -> Vec<StaticLayer> {
    run(background, pipeline, target)
        .into_iter()
        .map(|(_, layer)| layer)
        .collect()
}

/// The comparison only means something when the frames move
/// and carry the partly covered edges a double blend would shift.
#[test]
fn the_reference_frames_animate_over_translucent_edges() {
    let frames = run(
        FrameBackground::TransparentBlack,
        Pipeline::Full,
        Target::Cleared,
    );
    let translucent = frames[0]
        .0
        .iter()
        .filter(|[_, _, _, a]| *a > 0 && *a < u8::MAX)
        .count();

    assert!(
        translucent > 100,
        "BUG: only {translucent} translucent pixels, so a blended layer could pass"
    );
    assert!(
        frames.windows(2).all(|pair| pair[0].0 != pair[1].0),
        "BUG: a frame repeated its predecessor, so a stale layer could pass"
    );
}

/// Painting straight over the previous frame must show it,
/// or the kept-target tests below could not catch a blend or a skipped clear.
#[test]
fn a_kept_target_carries_the_previous_frame() {
    let reference = run(
        FrameBackground::TransparentBlack,
        Pipeline::Full,
        Target::Cleared,
    );
    let kept = run(
        FrameBackground::TransparentBlack,
        Pipeline::Full,
        Target::Kept,
    );

    assert!(
        reference
            .iter()
            .zip(&kept)
            .any(|(cleared, kept)| cleared.0 != kept.0),
        "BUG: painting over a kept frame matched a cleared one, so stale pixels go unseen"
    );
}

#[test]
fn a_cached_layer_over_transparency_paints_as_a_full_frame() {
    assert_matches_full_paint(
        FrameBackground::TransparentBlack,
        Pipeline::Cached,
        Target::Cleared,
    );
}

#[test]
fn a_cached_layer_over_black_paints_as_a_full_frame() {
    assert_matches_full_paint(
        FrameBackground::OpaqueBlack,
        Pipeline::Cached,
        Target::Cleared,
    );
}

#[test]
fn a_cached_layer_is_captured_once_and_then_reused() {
    for background in [
        FrameBackground::OpaqueBlack,
        FrameBackground::TransparentBlack,
    ] {
        let layers = layers(background, Pipeline::Cached, Target::Cleared);
        assert!(
            matches!(layers[0], StaticLayer::Captured { empty: false }),
            "{background:?}: the first frame must capture a non-empty layer, got {:?}",
            layers[0]
        );
        assert!(
            layers[1..]
                .iter()
                .all(|layer| matches!(layer, StaticLayer::Blitted)),
            "{background:?}: every later frame must reuse the layer, got {layers:?}"
        );
    }
}

#[test]
fn a_lost_layer_over_transparency_paints_as_a_full_frame() {
    assert_matches_full_paint(
        FrameBackground::TransparentBlack,
        Pipeline::LayerLost,
        Target::Cleared,
    );
}

#[test]
fn a_lost_layer_over_black_paints_as_a_full_frame() {
    assert_matches_full_paint(
        FrameBackground::OpaqueBlack,
        Pipeline::LayerLost,
        Target::Cleared,
    );
}

#[test]
fn a_lost_layer_misses_every_reuse() {
    let layers = layers(
        FrameBackground::TransparentBlack,
        Pipeline::LayerLost,
        Target::Cleared,
    );
    assert!(
        layers[1..]
            .iter()
            .all(|layer| matches!(layer, StaticLayer::Missed)),
        "BUG: the layer was dropped before each reuse, got {layers:?}"
    );
}

/// A failed copy reports a miss instead of blending the layer in.
/// Over a cleared target the two look alike, so only the reported miss tells them apart.
#[test]
fn a_failed_raw_copy_over_transparency_paints_in_full() {
    let layers = layers(
        FrameBackground::TransparentBlack,
        Pipeline::RawCopyFails,
        Target::Cleared,
    );
    assert!(
        layers
            .iter()
            .all(|layer| matches!(layer, StaticLayer::Missed)),
        "a transparent layer must never be blended in, got {layers:?}"
    );
    assert_matches_full_paint(
        FrameBackground::TransparentBlack,
        Pipeline::RawCopyFails,
        Target::Cleared,
    );
}

/// An opaque layer still falls back to femtovg's blended blit, which
/// copies it exactly because nothing underneath shows through.
#[test]
fn a_failed_raw_copy_over_black_blends_the_layer_exactly() {
    let layers = layers(
        FrameBackground::OpaqueBlack,
        Pipeline::RawCopyFails,
        Target::Cleared,
    );
    assert!(
        layers[1..]
            .iter()
            .all(|layer| matches!(layer, StaticLayer::Blitted)),
        "an opaque layer keeps its blended fallback, got {layers:?}"
    );
    assert_matches_full_paint(
        FrameBackground::OpaqueBlack,
        Pipeline::RawCopyFails,
        Target::Cleared,
    );
}

/// A layer reused on a background other than the one it was captured over
/// is recaptured, not copied: over transparency its black would cover
/// whatever lies behind the overlay. Nor is it a miss,
/// which would end layering for a surface that only changed.
#[test]
fn a_layer_captured_over_another_background_is_recaptured() {
    for background in [
        FrameBackground::TransparentBlack,
        FrameBackground::OpaqueBlack,
    ] {
        let reference = run(background, Pipeline::Full, Target::Cleared);
        for target in [Target::Cleared, Target::Kept, Target::Damaged] {
            let frames = run(background, Pipeline::SurfaceChanged, target);

            assert!(
                matches!(frames[1].1, StaticLayer::Captured { empty: false }),
                "{background:?} on a {target:?} target: the first frame on it must recapture, got {:?}",
                frames[1].1
            );
            assert!(
                frames[2..]
                    .iter()
                    .all(|(_, layer)| matches!(layer, StaticLayer::Blitted)),
                "{background:?} on a {target:?} target: the recaptured layer must be reused after that"
            );
            // The first frame was painted on the other background, so the comparison starts after it.
            assert_frames_match(
                &reference,
                &frames,
                1,
                &format!("{background:?} on a {target:?} target"),
            );
        }
    }
}

/// An eviction mid-capture takes the layer with it,
/// and nothing the capture drew may be offered as one.
#[test]
fn a_capture_that_loses_its_layer_reports_it() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");

    renderer.begin_frame_with_clear(
        W,
        H,
        1.0,
        FrameBackground::TransparentBlack,
        FrameClear::Clear,
    );
    assert!(
        renderer.begin_static_layer(KEY, W, H),
        "BUG: the layer must be allocatable on the test driver"
    );
    renderer.evict_prefix("overlay:");
    renderer.end_static_layer(KEY);

    assert!(
        !renderer.blit_static_layer(KEY),
        "nothing may be blitted from a capture that lost its layer"
    );
}

/// A recapture overwrites the layer, so until it completes
/// the layer holds neither the old static half nor the new one.
#[test]
fn a_recapture_in_progress_cannot_be_blitted() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");

    renderer.begin_frame_with_clear(W, H, 1.0, FrameBackground::OpaqueBlack, FrameClear::Clear);
    assert!(
        renderer.begin_static_layer(KEY, W, H),
        "BUG: the layer must be allocatable on the test driver"
    );
    renderer.end_static_layer(KEY);
    assert!(
        renderer.begin_static_layer(KEY, W, H),
        "BUG: the layer must be recapturable"
    );

    assert!(
        !renderer.blit_static_layer(KEY),
        "a half-recaptured layer must not be blitted"
    );
}

/// A whole-frame reuse has to replace what the target held,
/// not blend the layer over it.
#[test]
fn a_cached_layer_over_transparency_replaces_a_kept_frame() {
    assert_matches_full_paint(
        FrameBackground::TransparentBlack,
        Pipeline::Cached,
        Target::Kept,
    );
}

/// A miss repaints the tree only where it draws,
/// so the frame has to be cleared back to transparency first.
#[test]
fn a_lost_layer_clears_a_kept_frame_before_painting() {
    assert_matches_full_paint(
        FrameBackground::TransparentBlack,
        Pipeline::LayerLost,
        Target::Kept,
    );
}

/// Blending the layer in instead of the failed copy
/// would leave the previous frame showing through its transparent pixels.
#[test]
fn a_failed_raw_copy_never_blends_over_a_kept_frame() {
    assert_matches_full_paint(
        FrameBackground::TransparentBlack,
        Pipeline::RawCopyFails,
        Target::Kept,
    );
}

/// A driver that cannot render to a texture refuses every capture,
/// and each frame has to clear back to transparency before painting in full.
#[test]
fn a_refused_capture_over_transparency_paints_as_a_full_frame() {
    for target in [Target::Cleared, Target::Kept] {
        let layers = layers(
            FrameBackground::TransparentBlack,
            Pipeline::RefusedCapture,
            target,
        );
        assert!(
            layers
                .iter()
                .all(|layer| matches!(layer, StaticLayer::Missed)),
            "BUG: the capture must be refused on every frame, got {layers:?}"
        );
        assert_matches_full_paint(
            FrameBackground::TransparentBlack,
            Pipeline::RefusedCapture,
            target,
        );
    }
}

/// A damage-tracked reuse copies only the damaged rects of the layer,
/// and must still replace what the target held there.
#[test]
fn a_damage_tracked_reuse_over_transparency_paints_as_a_full_frame() {
    let layers = layers(
        FrameBackground::TransparentBlack,
        Pipeline::Cached,
        Target::Damaged,
    );
    assert!(
        layers[1..]
            .iter()
            .all(|layer| matches!(layer, StaticLayer::Blitted)),
        "every later frame must reuse the layer under damage, got {layers:?}"
    );
    assert_matches_full_paint(
        FrameBackground::TransparentBlack,
        Pipeline::Cached,
        Target::Damaged,
    );
}

/// A failed copy under damage misses like a whole-frame one,
/// rather than blending the damaged rects over the previous frame.
#[test]
fn a_failed_raw_copy_under_damage_paints_in_full() {
    let layers = layers(
        FrameBackground::TransparentBlack,
        Pipeline::RawCopyFails,
        Target::Damaged,
    );
    assert!(
        layers
            .iter()
            .all(|layer| matches!(layer, StaticLayer::Missed)),
        "a transparent layer must never be blended in, got {layers:?}"
    );
    assert_matches_full_paint(
        FrameBackground::TransparentBlack,
        Pipeline::RawCopyFails,
        Target::Damaged,
    );
}
