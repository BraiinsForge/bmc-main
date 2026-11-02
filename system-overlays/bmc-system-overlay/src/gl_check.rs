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

//! Real-GL check that an overlay's trees paint the same through the static layer
//! as they do in full.

use std::time::{Duration, Instant};

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::prelude::*;
use bmc_render::repaint::TargetContents;
use bmc_render::test_harness::{GlHarness, create_readback_fbo, read_pixels_top_down};

use bmc_render::tree::StaticLayer;

use crate::{
    FrameKind, Layering, OverlayFrame, RenderOutcome, SURFACE_LAYERING, TreeUi, begin_surface_frame,
};

/// How far apart a script stamps its frames, fixed so both passes of a run animate alike.
pub const FRAME_STEP: Duration = Duration::from_millis(16);

pub const CAPTURED: StaticLayer = StaticLayer::Captured { empty: false };
pub const REUSED: StaticLayer = StaticLayer::Blitted;
pub const UNCACHED: StaticLayer = StaticLayer::Unused;
pub const MISSED: StaticLayer = StaticLayer::Missed;

type Frame = (Vec<[u8; 4]>, StaticLayer);

/// One pass of a script over headless GL: either through the static layer
/// or painting every frame in full, as the reference.
#[expect(missing_debug_implementations, reason = "holds the renderer")]
pub struct GlRun<'a> {
    renderer: &'a mut FemtoVgRenderer,
    read: &'a dyn Fn() -> Vec<[u8; 4]>,
    layering: Layering,
    size: (u32, u32),
    frames: Vec<Frame>,
    started: Instant,
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

    /// Paint a `kind` frame through `draw`, which renders an overlay for the frame it is handed,
    /// stamped by the run's clock and layered as the pass is, and record its pixels.
    ///
    /// # Panics
    ///
    /// Panics when the frame drew no tree.
    pub fn paint(
        &mut self,
        kind: FrameKind,
        draw: impl FnOnce(&mut dyn Renderer, OverlayFrame) -> RenderOutcome,
    ) {
        let frame = OverlayFrame {
            kind,
            layering: self.layering,
            now: self.now(),
        };
        begin_surface_frame(self.renderer, self.size, TargetContents::Cleared);
        let outcome = draw(self.renderer, frame);
        self.renderer.flush();
        let tree = outcome.tree.expect("BUG: a tree overlay reports its tree");
        self.frames.push(((self.read)(), tree.static_layer));
    }
}

/// Run `script` through the static layer and again painting in full,
/// and assert each frame of the first paints exactly the pixels of the second.
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
    let reference = run(size, Layering::Unlayered, &script)?;
    let layered = run(size, SURFACE_LAYERING, &script)?;
    assert_eq!(
        layered.len(),
        reference.len(),
        "BUG: the script paints the same frames on every pass"
    );
    for (frame, (_, outcome)) in reference.iter().enumerate() {
        assert_eq!(
            *outcome,
            StaticLayer::Unused,
            "reference frame {frame} must paint in full, or the comparison checks the layer against itself"
        );
    }
    for (frame, ((expected, _), (actual, outcome))) in reference.iter().zip(&layered).enumerate() {
        let differing = expected.iter().zip(actual).filter(|(e, a)| e != a).count();
        assert_eq!(
            differing, 0,
            "frame {frame} ({outcome:?}) differs from a full paint in {differing} pixels"
        );
    }
    Some(layered.into_iter().map(|(_, outcome)| outcome).collect())
}

fn run(
    size: (u32, u32),
    layering: Layering,
    script: &impl Fn(&mut GlRun<'_>),
) -> Option<Vec<Frame>> {
    let harness = GlHarness::new_or_skip()?;
    let (width, height) = size;
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, width, height);
    // SAFETY: the harness made its context current on this thread and outlives the renderer.
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), width, height, fbo_id, 0) }
        .expect("BUG: a renderer builds over the headless context");
    let read = || read_pixels_top_down(&harness.gl, fbo, width, height);
    let mut run = GlRun {
        renderer: &mut renderer,
        read: &read,
        layering,
        size,
        frames: Vec::new(),
        started: Instant::now(),
    };
    script(&mut run);
    Some(run.frames)
}
