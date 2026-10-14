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

use super::{FemtoVgRenderer, PhysicalSize, PoolSlot, RenderTargetProbe, gl_name};
use crate::renderer::prelude::*;
use crate::test_harness::{
    GlHarness, create_colour_only_fbo, create_packed_stencil_fbo, create_readback_fbo,
    read_pixels_top_down,
};
use crate::test_tracing::counting_warns;
use bmc_wasm_protocol::{Color, Fill};
use glow::HasContext;

/// A failed probe has to reach `begin_static_layer`: left ungated, the
/// capture pass runs into a broken framebuffer, `end_static_layer` marks
/// the layer captured anyway, and later frames blit content that was never
/// rasterised.
#[test]
fn a_failed_render_target_probe_refuses_static_layers() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, 0, 0) }
        .expect("BUG: renderer init failed");

    renderer.render_to_texture = RenderTargetProbe::FramebufferIncomplete(0);

    assert!(
        !renderer.begin_static_layer("widget:static", 64, 64),
        "an unusable offscreen target must not open a static layer"
    );
    renderer.end_static_layer("widget:static");
    assert!(
        !renderer.blit_static_layer("widget:static"),
        "nothing was captured, so nothing may be blitted"
    );
}

#[test]
fn a_gl_name_past_i32_max_is_reinterpreted_not_dropped() {
    assert_eq!(
        gl_name(-1),
        std::num::NonZeroU32::new(u32::MAX),
        "a name GL returned as a negative GLint is still that name"
    );
    assert_eq!(gl_name(0), None, "zero is GL's \"no object\"");
}

#[test]
fn a_physical_size_rejects_what_no_layer_can_be() {
    assert!(
        PhysicalSize::new(1280_u32, 480).is_some(),
        "the display size must pool"
    );
    assert!(
        PhysicalSize::new(0_u32, 480).is_none(),
        "a zero-width stencil serves nothing"
    );
    assert!(
        PhysicalSize::new(-1_i32, 480).is_none(),
        "GL's signed sizes must not wrap into a key"
    );
    assert!(
        PhysicalSize::new(65_536_u32, 480).is_none(),
        "a size past u16 must not truncate into a smaller key"
    );
}

#[test]
fn static_layers_land_on_the_screen_targets_stencil() {
    const SIZE: u32 = 64;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, SIZE, SIZE);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), SIZE, SIZE, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let screen = Some(screen_stencil(&harness.gl, fbo, &renderer));

    let first = capture_layer_stencil(&mut renderer, "widget:one", SIZE, SIZE);
    let second = capture_layer_stencil(&mut renderer, "widget:two", SIZE, SIZE);

    assert_eq!(first, screen, "the first layer kept a stencil of its own");
    assert_eq!(second, screen, "the second layer kept a stencil of its own");
    assert_eq!(
        renderer.stencil_pool.len(),
        1,
        "two layers of one size must not pool two stencils"
    );
}

#[test]
fn static_layers_share_a_packed_screen_stencil() {
    const SIZE: u32 = 64;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_packed_stencil_fbo(&harness.gl, SIZE, SIZE);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), SIZE, SIZE, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let screen = Some(screen_stencil(&harness.gl, fbo, &renderer));

    let attached = capture_layer_stencil(&mut renderer, "widget:packed", SIZE, SIZE);

    assert_eq!(
        attached, screen,
        "a packed depth-stencil screen target must still lend its stencil"
    );
}

/// A recapture finds the layer already on the shared stencil.
/// Running the swap again there would shrink the shared stencil itself,
/// which for a display-sized layer is the screen target's own.
#[test]
fn recapturing_a_shared_layer_leaves_the_shared_stencil_alone() {
    const SIZE: u32 = 64;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, SIZE, SIZE);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), SIZE, SIZE, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let screen = screen_stencil(&harness.gl, fbo, &renderer);

    capture_layer_stencil(&mut renderer, "widget:again", SIZE, SIZE);
    let attached = capture_layer_stencil(&mut renderer, "widget:again", SIZE, SIZE);

    assert_eq!(
        attached,
        Some(screen),
        "the recaptured layer must stay on the shared stencil"
    );
    assert_eq!(
        renderbuffer_size(&harness.gl, screen),
        physical(SIZE, SIZE),
        "a recapture shrank the screen target's stencil"
    );
}

#[test]
fn the_swap_shrinks_the_stencil_femtovg_allocated() {
    const SIZE: u32 = 64;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, SIZE, SIZE);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), SIZE, SIZE, fbo_id, 0) }
        .expect("BUG: renderer init failed");

    bind_new_layer_framebuffer(&mut renderer, SIZE);
    // SAFETY: the harness context is current and the retarget has run.
    let femtovg_stencil = unsafe { renderer.attached_stencil() }
        .expect("BUG: femtovg allocates a stencil with the framebuffer");
    assert_eq!(
        renderbuffer_size(&harness.gl, femtovg_stencil),
        physical(SIZE, SIZE),
        "BUG: femtovg's per-image stencil is no longer full size, \
         so there is nothing left to save by sharing"
    );

    // SAFETY: the harness context is current,
    // and the retarget above left the layer's framebuffer bound.
    unsafe { renderer.share_layer_stencil(physical(SIZE, SIZE)) };

    assert_eq!(
        renderbuffer_size(&harness.gl, femtovg_stencil),
        physical(1, 1),
        "the displaced stencil still holds its memory, so nothing was saved"
    );
}

#[test]
fn an_already_incomplete_layer_does_not_refuse_its_size() {
    const SIZE: u32 = 64;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, SIZE, SIZE);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), SIZE, SIZE, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    bind_new_layer_framebuffer(&mut renderer, SIZE);
    make_bound_framebuffer_incomplete(&harness.gl, SIZE);

    // SAFETY: as above.
    unsafe { renderer.share_layer_stencil(physical(SIZE, SIZE)) };

    assert!(
        matches!(
            renderer.stencil_pool.get(&physical(SIZE, SIZE)),
            Some(PoolSlot::Borrowed(_))
        ),
        "an incomplete layer must not cost its size the shared stencil"
    );
}

/// The path every widget narrower than the display takes.
#[test]
fn a_layer_the_screen_stencil_cannot_serve_pools_its_own() {
    const FRAME: u32 = 64;
    const LAYER: u32 = 32;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, FRAME, FRAME);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), FRAME, FRAME, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let screen = screen_stencil(&harness.gl, fbo, &renderer);

    let pooled = capture_layer_stencil(&mut renderer, "widget:small", FRAME, LAYER)
        .expect("BUG: the layer framebuffer must carry a stencil");

    assert_ne!(
        pooled, screen,
        "a 32x32 layer cannot share the 64x64 screen stencil"
    );
    assert_eq!(
        renderbuffer_size(&harness.gl, pooled),
        physical(LAYER, LAYER),
        "the pooled stencil must match the layer it serves"
    );
    assert_eq!(
        renderer.stencil_pool.len(),
        2,
        "screen size plus layer size"
    );

    drop(renderer);
    // SAFETY: the harness context is current on this thread.
    unsafe {
        assert!(
            !harness.gl.is_renderbuffer(pooled),
            "the renderer must free the stencil it allocated"
        );
        assert!(
            harness.gl.is_renderbuffer(screen),
            "the screen target's stencil belongs to the embedder"
        );
    }
}

#[test]
fn a_refused_size_frees_its_stencil_and_is_not_offered_again() {
    const FRAME: u32 = 64;
    const LAYER: u32 = 32;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, FRAME, FRAME);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), FRAME, FRAME, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    // A colour format on the stencil attachment is incomplete on any GL,
    // which stands in for a driver refusing the swap.
    let side = i32::try_from(LAYER).expect("BUG: the test size fits GLsizei");
    // SAFETY: the harness context is current on this thread.
    let unusable = unsafe {
        let rbo = harness
            .gl
            .create_renderbuffer()
            .expect("BUG: create_renderbuffer failed");
        harness.gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
        harness
            .gl
            .renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, side, side);
        harness.gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        rbo
    };
    renderer.stencil_pool.insert(
        physical(LAYER, LAYER),
        PoolSlot::Owned {
            rbo: unusable,
            serving: false,
        },
    );

    let (first, warns) =
        counting_warns(|_| capture_layer_stencil(&mut renderer, "widget:first", FRAME, LAYER));
    let first = first.expect("BUG: the layer framebuffer must carry a stencil");

    assert_eq!(
        warns, 1,
        "a refusal gives up the saving for that size and must be reported"
    );
    assert_ne!(
        first, unusable,
        "the refused layer must be back on femtovg's stencil"
    );
    // SAFETY: the harness context is current on this thread.
    assert!(
        !unsafe { harness.gl.is_renderbuffer(unusable) },
        "a refused stencil must be freed, not left allocated with nothing attached"
    );
    assert!(
        matches!(
            renderer.stencil_pool.get(&physical(LAYER, LAYER)),
            Some(PoolSlot::Refused { kept: None })
        ),
        "the refusal must be remembered for that size"
    );

    capture_layer_stencil(&mut renderer, "widget:second", FRAME, LAYER);

    assert!(
        matches!(
            renderer.stencil_pool.get(&physical(LAYER, LAYER)),
            Some(PoolSlot::Refused { kept: None })
        ),
        "a later layer of a refused size must not allocate and retry the swap"
    );
}

#[test]
fn a_refusal_keeps_the_stencil_earlier_layers_draw_through() {
    const FRAME: u32 = 64;
    const LAYER: u32 = 32;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, FRAME, FRAME);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), FRAME, FRAME, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let shared = capture_layer_stencil(&mut renderer, "widget:shared", FRAME, LAYER)
        .expect("BUG: the layer framebuffer must carry a stencil");
    // A colour format makes the next swap incomplete, as a driver refusing it would.
    let side = i32::try_from(LAYER).expect("BUG: the test size fits GLsizei");
    // SAFETY: the harness context is current on this thread.
    unsafe {
        harness
            .gl
            .bind_renderbuffer(glow::RENDERBUFFER, Some(shared));
        harness
            .gl
            .renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, side, side);
        harness.gl.bind_renderbuffer(glow::RENDERBUFFER, None);
    }

    capture_layer_stencil(&mut renderer, "widget:refused", FRAME, LAYER);

    assert!(
        matches!(
            renderer.stencil_pool.get(&physical(LAYER, LAYER)),
            Some(PoolSlot::Refused { kept: Some(rbo) }) if *rbo == shared
        ),
        "the refusal must keep the stencil the first layer is still attached to"
    );
    // SAFETY: the harness context is current on this thread.
    assert!(
        unsafe { harness.gl.is_renderbuffer(shared) },
        "a stencil a layer still draws through must not be freed"
    );

    drop(renderer);
    // SAFETY: the harness context is current on this thread.
    assert!(
        !unsafe { harness.gl.is_renderbuffer(shared) },
        "the kept stencil must be freed with the renderer"
    );
}

#[test]
fn refusing_a_size_again_keeps_the_stencil_it_kept() {
    const SIZE: u32 = 64;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, SIZE, SIZE);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), SIZE, SIZE, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    // SAFETY: the harness context is current on this thread.
    let kept = unsafe {
        let rbo = harness
            .gl
            .create_renderbuffer()
            .expect("BUG: create_renderbuffer failed");
        harness.gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
        harness.gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        rbo
    };
    renderer
        .stencil_pool
        .insert(physical(SIZE, SIZE), PoolSlot::Refused { kept: Some(kept) });

    renderer.refuse_stencil_size(physical(SIZE, SIZE));

    assert!(
        matches!(
            renderer.stencil_pool.get(&physical(SIZE, SIZE)),
            Some(PoolSlot::Refused { kept: Some(rbo) }) if *rbo == kept
        ),
        "a second refusal must carry the kept stencil over"
    );
    // SAFETY: the harness context is current on this thread.
    assert!(
        unsafe { harness.gl.is_renderbuffer(kept) },
        "a second refusal freed a stencil earlier layers still draw through"
    );
}

#[test]
fn a_stale_shared_stencil_leaves_the_layer_on_femtovgs() {
    const SIZE: u32 = 64;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, SIZE, SIZE);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), SIZE, SIZE, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    // A name GL never generated fails the attach exactly as a deleted one does.
    // A real deleted renderbuffer cannot stand in for it:
    // Mesa hands the freed name straight to femtovg's next stencil.
    let stale = glow::NativeRenderbuffer(
        std::num::NonZeroU32::new(u32::MAX).expect("BUG: u32::MAX is non-zero"),
    );
    renderer
        .stencil_pool
        .insert(physical(SIZE, SIZE), PoolSlot::Borrowed(stale));

    let attached = capture_layer_stencil(&mut renderer, "widget:stale", SIZE, SIZE)
        .expect("BUG: the layer framebuffer must carry a stencil");

    assert_eq!(
        renderbuffer_size(&harness.gl, attached),
        physical(SIZE, SIZE),
        "the stencil the layer still uses was shrunk out from under it"
    );
    assert!(
        matches!(
            renderer.stencil_pool.get(&physical(SIZE, SIZE)),
            Some(PoolSlot::Refused { kept: None })
        ),
        "a size whose shared stencil cannot attach must not be offered again"
    );
    // SAFETY: the harness context is current on this thread.
    assert_eq!(
        unsafe { harness.gl.get_error() },
        glow::NO_ERROR,
        "the failed attach must not leave its error for an unrelated call"
    );
}

#[test]
fn a_layer_the_pool_could_not_serve_is_shared_on_a_later_capture() {
    const FRAME: u32 = 64;
    const LAYER: u32 = 32;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, FRAME, FRAME);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), FRAME, FRAME, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    // llvmpipe won't fail the allocation,
    // so `Refused` stands in only to make the pool come up empty.
    // In production it never clears:
    // a failed allocation inserts nothing, which the removal below mimics.
    renderer
        .stencil_pool
        .insert(physical(LAYER, LAYER), PoolSlot::Refused { kept: None });
    let femtovg_stencil = capture_layer_stencil(&mut renderer, "widget:retry", FRAME, LAYER)
        .expect("BUG: the layer framebuffer must carry a stencil");
    assert_eq!(
        renderbuffer_size(&harness.gl, femtovg_stencil),
        physical(LAYER, LAYER),
        "BUG: with nothing to offer, the first capture must keep femtovg's stencil"
    );

    renderer.stencil_pool.remove(&physical(LAYER, LAYER));
    let attached = capture_layer_stencil(&mut renderer, "widget:retry", FRAME, LAYER)
        .expect("BUG: the layer framebuffer must carry a stencil");

    assert_ne!(
        attached, femtovg_stencil,
        "a later capture must offer the shared stencil again"
    );
    assert_eq!(
        renderbuffer_size(&harness.gl, femtovg_stencil),
        physical(1, 1),
        "once shared, femtovg's stencil must give its memory back"
    );
}

#[test]
fn display_sized_layers_share_one_stencil_without_a_screen_stencil() {
    const SIZE: u32 = 64;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_colour_only_fbo(&harness.gl, SIZE, SIZE);
    let (mut renderer, warns) = counting_warns(|_| {
        unsafe { FemtoVgRenderer::new(harness.load_fn(), SIZE, SIZE, fbo_id, 0) }
            .expect("BUG: renderer init failed")
    });
    assert_eq!(
        warns, 1,
        "a missing screen stencil costs a full-size stencil and must be reported"
    );
    assert!(
        renderer.stencil_pool.is_empty(),
        "BUG: nothing was adopted, so nothing may be pooled"
    );

    let first = capture_layer_stencil(&mut renderer, "widget:one", SIZE, SIZE);
    let second = capture_layer_stencil(&mut renderer, "widget:two", SIZE, SIZE);

    assert!(
        matches!(
            renderer.stencil_pool.get(&physical(SIZE, SIZE)),
            Some(PoolSlot::Owned { rbo, serving: true })
                if first == Some(*rbo) && second == Some(*rbo)
        ),
        "with no screen stencil to borrow, both layers must share one pooled stencil"
    );
}

/// The shared stencil has to be a working stencil:
/// a concave fill reading one that is stale, unattached or the wrong size
/// covers its own notch.
#[test]
fn a_layer_on_the_shared_stencil_still_fills_a_concave_path() {
    const SIZE: u32 = 64;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, SIZE, SIZE);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), SIZE, SIZE, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let screen = screen_stencil(&harness.gl, fbo, &renderer);

    let attached = capture_layer_drawing(&mut renderer, "widget:concave", SIZE, SIZE, |r| {
        r.fill_path_paint(
            &[
                (8.0, 8.0),
                (56.0, 8.0),
                (56.0, 24.0),
                (24.0, 24.0),
                (24.0, 56.0),
                (8.0, 56.0),
            ],
            &Fill::Solid(Color::from_rgb(0, 255, 0)),
            false,
        );
    });
    assert_eq!(
        attached,
        Some(screen),
        "the L must go through the shared stencil, not femtovg's own"
    );

    renderer.begin_frame(SIZE, SIZE, 1.0);
    assert!(
        renderer.blit_static_layer("widget:concave"),
        "BUG: the captured layer must be blittable"
    );
    renderer.flush();

    let pixels = read_pixels_top_down(&harness.gl, fbo, SIZE, SIZE);
    let at = |x: usize, y: usize| pixels[y * SIZE as usize + x];
    assert!(at(48, 16)[1] > 200, "the arm of the L is missing");
    assert!(at(16, 48)[1] > 200, "the leg of the L is missing");
    assert!(at(48, 48)[1] < 40, "the notch of the L was filled in");
}

/// Capture an empty layer under `key`
/// and report the stencil renderbuffer its framebuffer ends up with,
/// read while the capture holds it bound.
fn capture_layer_stencil(
    renderer: &mut FemtoVgRenderer,
    key: &str,
    frame: u32,
    layer: u32,
) -> Option<glow::Renderbuffer> {
    capture_layer_drawing(renderer, key, frame, layer, |_| {})
}

/// [`capture_layer_stencil`], with `draw` run inside the capture.
fn capture_layer_drawing(
    renderer: &mut FemtoVgRenderer,
    key: &str,
    frame: u32,
    layer: u32,
    draw: impl FnOnce(&mut FemtoVgRenderer),
) -> Option<glow::Renderbuffer> {
    renderer.begin_frame(frame, frame, 1.0);
    assert!(
        renderer.begin_static_layer(key, layer, layer),
        "BUG: the layer must be allocatable on the test driver"
    );
    // SAFETY: the capture in progress leaves its own framebuffer bound.
    let stencil = unsafe { renderer.attached_stencil() };
    draw(renderer);
    renderer.end_static_layer(key);
    renderer.flush();
    stencil
}

/// Retarget to a fresh render-target image, which is what makes femtovg build its framebuffer,
/// as `begin_static_layer` does before the swap.
fn bind_new_layer_framebuffer(renderer: &mut FemtoVgRenderer, size: u32) {
    let image = renderer
        .canvas
        .create_image_empty(
            size as usize,
            size as usize,
            femtovg::PixelFormat::Rgba8,
            femtovg::ImageFlags::FLIP_Y,
        )
        .expect("BUG: render target image allocation failed");
    renderer
        .canvas
        .set_render_target(femtovg::RenderTarget::Image(image));
    renderer.canvas.flush();
}

fn screen_stencil(
    gl: &glow::Context,
    fbo: glow::Framebuffer,
    renderer: &FemtoVgRenderer,
) -> glow::Renderbuffer {
    // SAFETY: the harness context is current on this thread.
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        renderer.attached_stencil()
    }
    .expect("BUG: the harness screen target must carry a stencil")
}

/// Attach a colour format to the bound framebuffer's depth attachment,
/// which is incomplete on any GL.
fn make_bound_framebuffer_incomplete(gl: &glow::Context, size: u32) {
    let side = i32::try_from(size).expect("BUG: the test size fits GLsizei");
    // SAFETY: the harness context is current on this thread.
    unsafe {
        let colour = gl
            .create_renderbuffer()
            .expect("BUG: create_renderbuffer failed");
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(colour));
        gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, side, side);
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(colour),
        );
    }
}

fn physical(width: u32, height: u32) -> PhysicalSize {
    PhysicalSize::new(width, height).expect("BUG: test sizes fit a stencil")
}

fn renderbuffer_size(gl: &glow::Context, rbo: glow::Renderbuffer) -> PhysicalSize {
    // SAFETY: the harness context is current on this thread.
    let size = unsafe {
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
        let size = PhysicalSize::new(
            gl.get_renderbuffer_parameter_i32(glow::RENDERBUFFER, glow::RENDERBUFFER_WIDTH),
            gl.get_renderbuffer_parameter_i32(glow::RENDERBUFFER, glow::RENDERBUFFER_HEIGHT),
        );
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        size
    };
    size.expect("BUG: GL reported a size no stencil can have")
}

/// femtovg leaves an incomplete layer framebuffer bound,
/// so the capture must check completeness, not only that the binding left the screen.
#[test]
fn an_incomplete_layer_framebuffer_is_refused_at_that_size() {
    const KEY: &str = "widget:static";
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, 64, 64);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let oversized = oversized_layer_width(&harness.gl);
    renderer.begin_frame(64, 64, 1.0);

    assert!(
        !renderer.begin_static_layer(KEY, oversized, 1),
        "an incomplete layer framebuffer must not open a capture"
    );
    assert_eq!(
        unsafe { harness.gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING) },
        fbo_id.cast_signed(),
        "the frame's framebuffer must be bound again"
    );
    assert_eq!(
        renderer.incomplete_static_layers.get(KEY),
        Some(&physical(oversized, 1)),
        "the failure must be remembered at its size, or every frame retries it"
    );
    assert!(
        !renderer.begin_static_layer(KEY, oversized, 1),
        "the same size must stay refused rather than retried"
    );
    assert!(
        !renderer.static_layers.contains_key(KEY),
        "a refused layer must not keep an image"
    );
    assert!(
        !renderer.blit_static_layer(KEY),
        "nothing was captured, so nothing may be blitted"
    );
}

/// The failure is one layer's at one size, not the driver's,
/// so it must not cost other widgets or a resized one their cache.
#[test]
fn an_incomplete_layer_framebuffer_spares_other_layers_and_resizes() {
    const KEY: &str = "widget:static";
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, 64, 64);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let oversized = oversized_layer_width(&harness.gl);
    renderer.begin_frame(64, 64, 1.0);
    assert!(
        !renderer.begin_static_layer(KEY, oversized, 1),
        "a layer past the texture limit must fail to open"
    );

    assert!(
        renderer.begin_static_layer("widget:other", 64, 64),
        "another widget's layer must still open"
    );
    renderer.end_static_layer("widget:other");
    assert!(
        renderer.begin_static_layer(KEY, 64, 64),
        "a resized layer must be retried"
    );
    renderer.end_static_layer(KEY);
}

/// Refusing a layer at the size it failed at
/// must not strand the image it was captured into at another size.
#[test]
fn refusing_a_layer_at_its_failed_size_frees_its_other_capture() {
    const KEY: &str = "widget:static";
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, 64, 64);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let oversized = oversized_layer_width(&harness.gl);
    renderer.begin_frame(64, 64, 1.0);
    assert!(
        !renderer.begin_static_layer(KEY, oversized, 1),
        "a layer past the texture limit must fail to open"
    );
    assert!(
        renderer.begin_static_layer(KEY, 64, 64),
        "a resized layer must be retried"
    );
    renderer.end_static_layer(KEY);

    assert!(
        !renderer.begin_static_layer(KEY, oversized, 1),
        "the failed size must stay refused"
    );
    assert!(
        !renderer.static_layers.contains_key(KEY),
        "the capture at the other size must be freed"
    );
}

/// Past the first retarget, the incomplete framebuffer is femtovg's
/// and carries the pooled stencil, so neither is the refusal's to delete.
#[test]
fn a_later_incomplete_retarget_leaves_the_pooled_stencil_alone() {
    const KEY: &str = "widget:static";
    const SIZE: u32 = 64;
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, SIZE, SIZE);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), SIZE, SIZE, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let pooled = capture_layer_drawing(&mut renderer, KEY, SIZE, SIZE, |_| {
        make_bound_framebuffer_incomplete(&harness.gl, SIZE);
    })
    .expect("BUG: the layer framebuffer must carry a stencil");

    renderer.begin_frame(SIZE, SIZE, 1.0);
    assert!(
        !renderer.begin_static_layer(KEY, SIZE, SIZE),
        "an incomplete layer framebuffer must not open a capture"
    );
    assert!(
        unsafe { harness.gl.is_renderbuffer(pooled) },
        "the refusal must not delete the stencil the pool still hands out"
    );
}

/// femtovg allocates a layer texture without checking the driver's limit,
/// so a layer one pixel past it gets an incomplete framebuffer.
fn oversized_layer_width(gl: &glow::Context) -> u32 {
    // SAFETY: the harness context is current on this thread.
    let max = unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) };
    let width = u32::try_from(max).expect("BUG: GL reports a positive texture limit") + 1;
    assert!(
        PhysicalSize::new(width, 1).is_some(),
        "BUG: no layer size lies past the test driver's texture limit"
    );
    width
}

#[test]
fn deleting_an_orphaned_framebuffer_frees_its_stencil_renderbuffer() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, 0, 0) }
        .expect("BUG: renderer init failed");
    let gl = &harness.gl;
    let (fbo, stencil) = unsafe {
        let fbo = gl
            .create_framebuffer()
            .expect("BUG: create_framebuffer failed");
        let stencil = gl
            .create_renderbuffer()
            .expect("BUG: create_renderbuffer failed");
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(stencil));
        gl.renderbuffer_storage(glow::RENDERBUFFER, glow::STENCIL_INDEX8, 4, 4);
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::STENCIL_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(stencil),
        );
        (fbo, stencil)
    };

    // SAFETY: the harness context is current, with a framebuffer no one owns bound.
    unsafe { renderer.delete_orphaned_framebuffer() };

    let alive = unsafe { (gl.is_framebuffer(fbo), gl.is_renderbuffer(stencil)) };
    assert_eq!(
        alive,
        (false, false),
        "both the framebuffer and its stencil must be deleted"
    );
}

/// The gate is only safe if the probe itself is right: a false negative
/// silently costs every device the static-layer cache. llvmpipe supports
/// RGBA8 offscreen targets, so anything but `Working` is a probe bug.
#[test]
fn the_render_target_probe_reports_working_on_a_conformant_driver() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, 0, 0) }
        .expect("BUG: renderer init failed");

    assert_eq!(renderer.render_to_texture(), RenderTargetProbe::Working);
}
