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

//! Real-GL check that an overlay's trees paint the same through the static layer,
//! and repainting only what moved into a rotating buffer pair,
//! as they do in full.

use std::time::{Duration, Instant};

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::prelude::*;
use bmc_render::repaint::EXPORT_BUFFERS;
use bmc_render::test_harness::{GlHarness, RotatingTarget};

use bmc_render::tree::{Draw, Node, StaticLayer};
use bmc_render::ui::{AnimProperty, AnimationDef, Color, ColorSpace, Easing, LoopMode, PropsData};

use crate::{
    FrameKind, Layering, OverlayFrame, RenderOutcome, SURFACE_LAYERING, TreeFrameProfile, TreeUi,
    paint_surface_frame,
};

/// How far apart a script stamps its frames, fixed so every pass of a run animates alike.
pub const FRAME_STEP: Duration = Duration::from_millis(16);

pub const CAPTURED: StaticLayer = StaticLayer::Captured { empty: false };
pub const REUSED: StaticLayer = StaticLayer::Blitted;
pub const UNCACHED: StaticLayer = StaticLayer::Unused;
pub const MISSED: StaticLayer = StaticLayer::Missed;

type Frame = (Vec<[u8; 4]>, TreeFrameProfile);

/// How a pass of a script paints its frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Every frame in full, as the reference.
    Full,
    /// Through the static layer, over a cleared target.
    Layered,
    /// Through the static layer, into whichever of a rotating buffer pair is next,
    /// keeping it whenever the tree asks.
    DamageTracked,
}

/// One pass of a script over headless GL.
#[expect(missing_debug_implementations, reason = "holds the renderer")]
pub struct GlRun<'a> {
    renderer: &'a mut FemtoVgRenderer,
    gl: &'a glow::Context,
    target: &'a RotatingTarget,
    pass: Pass,
    size: (u32, u32),
    frames: Vec<Frame>,
    started: Instant,
    /// The export buffer the next frame paints, rotated only by a damage-tracked pass.
    next: usize,
}

impl GlRun<'_> {
    /// The instant the next frame is stamped with, [`FRAME_STEP`] after the last.
    #[must_use]
    pub fn now(&self) -> Instant {
        let painted = u32::try_from(self.frames.len()).expect("BUG: a script paints few frames");
        self.started + FRAME_STEP * painted
    }

    /// Drop `tree`'s layer behind its back, so its next frame misses.
    pub fn lose_layer(&mut self, tree: &TreeUi) {
        self.renderer
            .invalidate_static_layer(tree.static_layer_owner());
    }

    /// Hide `tree` as a host does: drop the cached tree and release its layer.
    pub fn hide(&mut self, tree: &mut TreeUi) {
        tree.clear();
        tree.release(self.renderer);
    }

    /// Paint a `kind` frame of `subject` through [`paint_surface_frame`], as a host does,
    /// into the pass's next buffer, and record its pixels.
    ///
    /// `draw` renders the overlay for the frame it is handed, stamped by the run's clock
    /// and layered as the pass is. `tree_of` reaches the tree it draws through,
    /// which only a damage-tracked pass asks whether the frame keeps its buffer.
    ///
    /// # Panics
    ///
    /// Panics when the frame drew no tree.
    pub fn paint<S: ?Sized>(
        &mut self,
        kind: FrameKind,
        subject: &mut S,
        tree_of: impl for<'a> Fn(&'a mut S) -> Option<&'a mut TreeUi>,
        draw: impl FnOnce(&mut S, &mut dyn Renderer, OverlayFrame) -> RenderOutcome,
    ) {
        let frame = OverlayFrame {
            kind,
            layering: match self.pass {
                Pass::Full => Layering::Unlayered,
                Pass::Layered | Pass::DamageTracked => SURFACE_LAYERING,
            },
            now: self.now(),
        };
        let tracked = self.pass == Pass::DamageTracked;
        let (gl, target, next) = (self.gl, self.target, self.next);
        let outcome = paint_surface_frame(
            subject,
            |subject| if tracked { tree_of(subject) } else { None },
            self.renderer,
            self.size,
            kind,
            |_| {
                target.attach(gl, next);
                Ok(())
            },
            |subject, renderer| draw(subject, renderer, frame),
        )
        .expect("BUG: a headless frame always binds");
        let tree = outcome.tree.expect("BUG: a tree overlay reports its tree");
        self.frames.push((self.target.read(self.gl), tree));
        if tracked {
            self.next = (self.next + 1) % EXPORT_BUFFERS;
        }
    }
}

/// Run `script` through the static layer,
/// again repainting only what moved into a rotating buffer pair,
/// and once more painting in full,
/// and assert each frame of the first two paints exactly the pixels of the last.
///
/// Returns what each layered frame did with the layer,
/// or `None` when no headless GL is available outside the `ci` profile.
///
/// # Panics
///
/// Panics on a differing frame, on a reference frame that touched the layer,
/// or when GL is missing where [`bmc_render::gl_skip::REQUIRE_EGL`] demands it.
#[must_use]
pub fn assert_layered_run_matches_full_paint(
    size: (u32, u32),
    script: impl Fn(&mut GlRun<'_>),
) -> Option<Vec<StaticLayer>> {
    let reference = reference_run(size, &script)?;
    let layered = run(size, Pass::Layered, &script)?;
    assert_matches_reference(&reference, &layered, Pass::Layered);
    let tracked = run(size, Pass::DamageTracked, &script)?;
    assert_matches_reference(&reference, &tracked, Pass::DamageTracked);
    Some(
        layered
            .into_iter()
            .map(|(_, profile)| profile.static_layer)
            .collect(),
    )
}

/// Run `script` repainting only what moved into a rotating buffer pair, and again
/// painting in full, and assert each frame of the first paints exactly the pixels of the second.
///
/// Returns how many pixels each damage-tracked frame repainted,
/// or `None` when no headless GL is available outside the `ci` profile.
///
/// # Panics
///
/// As [`assert_layered_run_matches_full_paint`].
#[must_use]
pub fn assert_damage_tracked_run_matches_full_paint(
    size: (u32, u32),
    script: impl Fn(&mut GlRun<'_>),
) -> Option<Vec<usize>> {
    let reference = reference_run(size, &script)?;
    let tracked = run(size, Pass::DamageTracked, &script)?;
    assert_matches_reference(&reference, &tracked, Pass::DamageTracked);
    Some(
        tracked
            .into_iter()
            .map(|(_, profile)| profile.repainted_px)
            .collect(),
    )
}

/// How many places [`assert_moving_damage_matches_full_paint`] moves its square to.
const SQUARE_MOVES: usize = 8;

/// The replays [`assert_moving_damage_matches_full_paint`] paints after each move.
const REPLAYS_PER_MOVE: usize = 3;

/// Run the tree `build` makes from what `setup` registers in each pass's renderer,
/// with a square over it that moves on every rebuild and holds still
/// while the frames between replay it, and assert every frame paints as a full paint does.
///
/// A replay after a move paints into the buffer that still shows the square
/// where it stood two frames before, so it matches only if its damage spans
/// both buffers' history. Every such replay must repaint only part of the surface.
///
/// Returns how many pixels each frame repainted,
/// or `None` when no headless GL is available outside the `ci` profile.
///
/// # Panics
///
/// As [`assert_layered_run_matches_full_paint`], when the tree's root has no room for the square,
/// or when a replay after a move repaints in full.
#[must_use]
pub fn assert_moving_damage_matches_full_paint<T>(
    size: (u32, u32),
    setup: impl Fn(&mut dyn Renderer) -> T,
    build: impl Fn(&T) -> Node,
) -> Option<Vec<usize>> {
    let repainted = assert_damage_tracked_run_matches_full_paint(size, |run| {
        let registered = setup(run.renderer);
        let mut tree = TreeUi::new(run.now());
        for step in 0..SQUARE_MOVES {
            let node = with_square(build(&registered), size, step);
            paint_node(run, &mut tree, &node, FrameKind::Fresh);
            for _ in 0..REPLAYS_PER_MOVE {
                paint_node(run, &mut tree, &node, FrameKind::AnimationOnly);
            }
        }
    })?;
    // The first move's frames catch the second buffer up on the captured layer.
    let surface = usize::try_from(size.0 * size.1).expect("BUG: a display surface fits usize");
    for (frame, pixels) in repainted.iter().enumerate().skip(REPLAYS_PER_MOVE + 1) {
        if frame % (REPLAYS_PER_MOVE + 1) != 0 {
            assert!(
                *pixels < surface,
                "replay {frame} repainted the whole surface: {repainted:?}"
            );
        }
    }
    Some(repainted)
}

/// `node` with [`square_at`] `step` drawn over the whole `size` surface:
/// on the root, when that is a canvas, or on a canvas of its own,
/// as the tree's other canvases may be too small to hold every place.
#[expect(clippy::cast_precision_loss, reason = "display sizes fit f32")]
fn with_square(mut node: Node, size: (u32, u32), step: usize) -> Node {
    if let Node::Canvas { draws, .. } = &mut node {
        draws.push(square_at(step));
        return node;
    }
    let (Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children)) =
        &mut node
    else {
        panic!("BUG: the tree's root is neither a canvas nor takes children to hold the square");
    };
    children.push(Node::Canvas {
        props: PropsData {
            inset_left: 0.0,
            inset_top: 0.0,
            width: size.0 as f32,
            height: size.1 as f32,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![square_at(step)],
    });
    node
}

/// A dynamic square at the `step`th place along the top edge.
#[expect(clippy::cast_precision_loss, reason = "a handful of steps")]
fn square_at(step: usize) -> Draw {
    let x = 40.0 + 100.0 * step as f32;
    Draw::Modified {
        // Animated so the walk counts it dynamic; one pixel of travel keeps it where it was put.
        animations: vec![AnimationDef {
            property: AnimProperty::TranslateX,
            from: x,
            to: x + 1.0,
            duration_ms: 1000,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(Draw::rect(
            0.0,
            0.0,
            24.0,
            24.0,
            Color::from_rgb(200, 30, 60),
        )),
    }
}

fn paint_node(run: &mut GlRun<'_>, tree: &mut TreeUi, node: &Node, kind: FrameKind) {
    let size = run.size;
    let intl = bmc_intl::formatter(bmc_intl::Language::default());
    run.paint(
        kind,
        tree,
        |tree| Some(tree),
        |tree, renderer, frame| {
            tree.render(frame, size, renderer, &intl, || node.clone())
                .expect("BUG: the tree renders")
                .outcome()
        },
    );
}

fn reference_run(size: (u32, u32), script: &impl Fn(&mut GlRun<'_>)) -> Option<Vec<Frame>> {
    let reference = run(size, Pass::Full, script)?;
    for (frame, (_, profile)) in reference.iter().enumerate() {
        assert_eq!(
            profile.static_layer,
            StaticLayer::Unused,
            "reference frame {frame} must paint in full, or the comparison checks the layer against itself"
        );
    }
    Some(reference)
}

fn assert_matches_reference(reference: &[Frame], frames: &[Frame], pass: Pass) {
    assert_eq!(
        frames.len(),
        reference.len(),
        "BUG: the script paints the same frames on every pass"
    );
    for (frame, ((expected, _), (actual, profile))) in reference.iter().zip(frames).enumerate() {
        let differing = expected.iter().zip(actual).filter(|(e, a)| e != a).count();
        assert_eq!(
            differing, 0,
            "{pass:?} frame {frame} ({profile:?}) differs from a full paint in {differing} pixels"
        );
    }
}

fn run(size: (u32, u32), pass: Pass, script: &impl Fn(&mut GlRun<'_>)) -> Option<Vec<Frame>> {
    let harness = GlHarness::new_or_skip()?;
    let (width, height) = size;
    let target = RotatingTarget::new(&harness.gl, width, height);
    // SAFETY: the harness made its context current on this thread and outlives the renderer.
    let mut renderer =
        unsafe { FemtoVgRenderer::new(harness.load_fn(), width, height, target.fbo_id(), 0) }
            .expect("BUG: a renderer builds over the headless context");
    let mut run = GlRun {
        renderer: &mut renderer,
        gl: &harness.gl,
        target: &target,
        pass,
        size,
        frames: Vec::new(),
        started: Instant::now(),
        next: 0,
    };
    script(&mut run);
    Some(run.frames)
}
