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

//! A static layer only pays off if nobody can tell it is there,
//! so a capture that did not complete must never be offered as one.

use super::FemtoVgRenderer;
use crate::renderer::FrameClear;
use crate::renderer::prelude::*;
use crate::test_harness::{GlHarness, create_readback_fbo};

const W: u32 = 96;
const H: u32 = 64;
const KEY: &str = "overlay:static";

/// An eviction mid-capture takes the layer with it,
/// and nothing the capture drew may be offered as one.
#[test]
fn a_capture_that_loses_its_layer_reports_it() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");

    renderer.begin_frame_with_clear(W, H, 1.0, FrameClear::TransparentBlack);
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

    renderer.begin_frame_with_clear(W, H, 1.0, FrameClear::OpaqueBlack);
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
