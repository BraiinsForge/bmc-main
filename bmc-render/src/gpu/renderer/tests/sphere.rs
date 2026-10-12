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

use super::{FemtoVgRenderer, SphereRendererState};
use crate::renderer::{AssetSuspendResult, AssetTagState, Renderer};
use crate::test_harness::{GlHarness, create_readback_fbo, read_pixels_top_down};
use glow::HasContext;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use std::io::Cursor;

#[test]
fn mesh_reservation_does_not_initialize_gpu_renderer() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, 0, 0) }
        .expect("BUG: renderer init failed");

    let id = renderer
        .reserve_mesh("widget:mesh")
        .expect("BUG: first mesh reservation must allocate an ID");

    assert!(
        renderer.mesh_renderer.is_none(),
        "a reservation without payload must not allocate the mesh atlas"
    );
    assert_eq!(
        renderer.mesh_tag_state("widget:mesh"),
        AssetTagState::Suspended(id)
    );
}

/// Encode a 1×1 RGBA PNG with the given pixel; minimum payload that
/// rides through `BitmapRegistry::register`'s decode+upload path.
fn one_px_png(rgba: [u8; 4]) -> Vec<u8> {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgba(rgba));
    let mut buf = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, ImageFormat::Png)
        .expect("BUG: PNG encode should succeed");
    buf.into_inner()
}

/// Drain any buffered GL errors so a subsequent `gl.get_error()` reflects
/// only the operation under test.
/// Bounded so a lost context fails the test rather than hanging it.
fn drain_gl_errors(gl: &glow::Context) {
    const MAX_DRAIN: usize = 8;
    for _ in 0..MAX_DRAIN {
        if unsafe { gl.get_error() } == glow::NO_ERROR {
            return;
        }
    }
}

#[test]
fn zero_and_subpixel_spheres_do_not_initialize_the_cached_renderer() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, 0, 0) }
        .expect("BUG: renderer init failed");
    let bitmap_id = renderer
        .register_bitmap("widget-42:sphere", &one_px_png([255, 0, 0, 255]))
        .expect("BUG: bitmap registration should succeed");

    renderer.draw_sphere(
        0.0,
        0.0,
        0.0,
        64.0,
        bitmap_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    assert!(
        matches!(renderer.sphere, SphereRendererState::Uninitialized),
        "a layout placeholder must not choose the cached offscreen size"
    );

    renderer.draw_sphere(
        0.0,
        0.0,
        0.5,
        64.0,
        bitmap_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    assert!(
        matches!(renderer.sphere, SphereRendererState::Uninitialized),
        "a subpixel scale-in frame must not latch sphere initialization failure"
    );

    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        bitmap_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    assert!(
        matches!(renderer.sphere, SphereRendererState::Ready(_)),
        "the first drawable sphere must initialize the renderer"
    );
}

#[test]
fn failed_sphere_initialization_is_not_retried_each_frame() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, 64, 64);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let bitmap_id = renderer
        .register_bitmap("widget-42:sphere", &one_px_png([255, 0, 0, 255]))
        .expect("BUG: bitmap registration should succeed");
    renderer.sphere = SphereRendererState::Failed;

    renderer.begin_frame(64, 64, 1.0);
    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        bitmap_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    renderer.flush();

    assert!(matches!(renderer.sphere, SphereRendererState::Failed));
    let pixels = read_pixels_top_down(&harness.gl, fbo, 64, 64);
    assert_eq!(pixels[32 * 64 + 32], [255, 0, 0, 255]);
}

#[test]
fn sphere_projection_preserves_geographic_orientation_and_silhouette() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, 64, 64);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let quadrants = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];
    let bitmap_id = renderer
        .register_bitmap_rgba_nearest("widget-42:quadrants", &quadrants, 2, 2)
        .expect("BUG: quadrant texture registration should succeed");

    renderer.begin_frame(64, 64, 1.0);
    renderer.draw_sphere(
        0.0, 0.0, 64.0, 64.0, bitmap_id, 0.0, 0.0, 2.0, 0.0, 0.0, false,
    );
    renderer.flush();
    let pixels = read_pixels_top_down(&harness.gl, fbo, 64, 64);
    let north_west = pixels[24 * 64 + 24];
    assert!(
        north_west[0] > 240 && north_west[1] < 16 && north_west[2] < 16,
        "north-west must sample the red lon<0, lat>0 quadrant, got {north_west:?}"
    );
    let corner = pixels[0];
    assert!(
        corner[..3].iter().all(|channel| *channel < 16),
        "outside the projected sphere must remain black, got {corner:?}"
    );

    renderer.begin_frame(64, 64, 1.0);
    renderer.draw_sphere(
        0.0, 0.0, 64.0, 64.0, bitmap_id, 45.0, 90.0, 2.0, 45.0, 90.0, false,
    );
    renderer.flush();
    let pixels = read_pixels_top_down(&harness.gl, fbo, 64, 64);
    let center = pixels[32 * 64 + 32];
    assert!(
        center[1] > 240 && center[0] < 16 && center[2] < 16,
        "45° N, 90° E must rotate the green quadrant to center, got {center:?}"
    );
}

/// Regression for the use-after-delete documented on MR !324: when
/// `BitmapSlot::set` evicts+re-registers the sphere's source bitmap,
/// the next `draw_sphere` must rebind to the fresh GL texture instead
/// of sampling the deleted one.
#[test]
fn sphere_rebind_after_bitmap_evict_does_not_use_deleted_texture() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, 64, 64);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, fbo_id, 0) }
        .expect("BUG: renderer init failed");

    let png_a = one_px_png([255, 0, 0, 255]);
    let png_b = one_px_png([0, 255, 0, 255]);

    let id_a = renderer
        .register_bitmap("sphere:test", &png_a)
        .expect("BUG: register A");
    drain_gl_errors(&harness.gl);

    renderer.begin_frame(64, 64, 1.0);
    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        id_a,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    renderer.flush();
    let err = unsafe { harness.gl.get_error() };
    assert_eq!(
        err,
        glow::NO_ERROR,
        "first draw_sphere produced GL 0x{err:04X}"
    );
    assert_eq!(renderer.sphere_bitmap_id, Some(id_a));
    let pixels = read_pixels_top_down(&harness.gl, fbo, 64, 64);
    let center = pixels[32 * 64 + 32];
    assert!(
        center[0] > 240 && center[1] < 16 && center[2] < 16,
        "initial red texture must be sampled at sphere center, got {center:?}"
    );

    assert_eq!(renderer.evict_prefix("sphere:test"), 1);
    let id_b = renderer
        .register_bitmap("sphere:test", &png_b)
        .expect("BUG: register B");
    assert_eq!(id_a, id_b, "released bitmap ID should be reused");

    drain_gl_errors(&harness.gl);
    renderer.begin_frame(64, 64, 1.0);
    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        id_b,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    renderer.flush();
    let err = unsafe { harness.gl.get_error() };
    assert_eq!(
        err,
        glow::NO_ERROR,
        "draw_sphere after evict+rebind produced GL 0x{err:04X} (use-after-delete)",
    );
    assert_eq!(
        renderer.sphere_bitmap_id,
        Some(id_b),
        "BUG: rebind tracker must follow the new BitmapId",
    );
    let pixels = read_pixels_top_down(&harness.gl, fbo, 64, 64);
    let center = pixels[32 * 64 + 32];
    assert!(
        center[1] > 240 && center[0] < 16 && center[2] < 16,
        "replacement green texture must be sampled at sphere center, got {center:?}"
    );
}

#[test]
fn sphere_rebinds_resident_rgba_replacement_with_same_bitmap_id() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, 64, 64);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let tag = "widget-42:sphere";

    let bitmap_id = renderer
        .register_bitmap_rgba(tag, &[255, 0, 0, 255], 1, 1)
        .expect("BUG: initial RGBA registration should succeed");
    let old_image_id = renderer
        .bitmap_registry
        .get(bitmap_id)
        .expect("BUG: initial bitmap must have an image");

    renderer.begin_frame(64, 64, 1.0);
    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        bitmap_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    renderer.flush();
    let pixels = read_pixels_top_down(&harness.gl, fbo, 64, 64);
    let center = pixels[32 * 64 + 32];
    assert!(
        center[0] > 240 && center[1] < 16 && center[2] < 16,
        "initial red texture must be sampled at sphere center, got {center:?}"
    );

    let replacement_id = renderer
        .register_bitmap_rgba(tag, &[0, 0, 255, 255], 1, 1)
        .expect("BUG: replacement RGBA registration should succeed");
    let replacement_image_id = renderer
        .bitmap_registry
        .get(replacement_id)
        .expect("BUG: replacement bitmap must have an image");
    assert_eq!(
        replacement_id, bitmap_id,
        "RGBA replacement must preserve its tag reservation"
    );
    assert_ne!(
        replacement_image_id, old_image_id,
        "RGBA replacement must install a new FemtoVG image"
    );
    assert!(
        renderer.canvas.get_native_texture(old_image_id).is_err(),
        "RGBA replacement must delete the previous native texture"
    );

    renderer.begin_frame(64, 64, 1.0);
    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        replacement_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    renderer.flush();
    let pixels = read_pixels_top_down(&harness.gl, fbo, 64, 64);
    let center = pixels[32 * 64 + 32];
    assert!(
        center[2] > 240 && center[0] < 16 && center[1] < 16,
        "replacement blue texture must be sampled at sphere center, got {center:?}"
    );
}

#[test]
fn sphere_rebinds_resident_nearest_rgba_replacement() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, 64, 64);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let tag = "widget-42:nearest-sphere";

    let bitmap_id = renderer
        .register_bitmap_rgba_nearest(tag, &[255, 0, 0, 255], 1, 1)
        .expect("BUG: initial nearest RGBA registration should succeed");
    renderer.begin_frame(64, 64, 1.0);
    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        bitmap_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    renderer.flush();
    assert_eq!(renderer.sphere_bitmap_id, Some(bitmap_id));

    let replacement_id = renderer
        .register_bitmap_rgba_nearest(tag, &[0, 0, 255, 255], 1, 1)
        .expect("BUG: replacement nearest RGBA registration should succeed");
    assert_eq!(
        replacement_id, bitmap_id,
        "nearest RGBA replacement must preserve its tag reservation"
    );

    renderer.begin_frame(64, 64, 1.0);
    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        replacement_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    renderer.flush();
    let pixels = read_pixels_top_down(&harness.gl, fbo, 64, 64);
    let center = pixels[32 * 64 + 32];
    assert!(
        center[2] > 240 && center[0] < 16 && center[1] < 16,
        "replacement blue texture must be sampled at sphere center, got {center:?}"
    );
}

#[test]
fn sphere_rebinds_bitmap_restored_after_suspension() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, 64, 64);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let tag = "widget-42:sphere";
    let initial_png = one_px_png([255, 0, 0, 255]);
    let restored_png = one_px_png([0, 0, 255, 255]);

    let bitmap_id = renderer
        .register_bitmap(tag, &initial_png)
        .expect("BUG: bitmap registration should succeed");
    let old_image_id = renderer
        .bitmap_registry
        .get(bitmap_id)
        .expect("BUG: registered bitmap must have an image");

    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        bitmap_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    assert_eq!(renderer.sphere_bitmap_id, Some(bitmap_id));

    assert_eq!(
        renderer.suspend_bitmap(tag),
        AssetSuspendResult::Suspended(bitmap_id)
    );
    assert_eq!(
        renderer.bitmap_tag_state(tag),
        AssetTagState::Suspended(bitmap_id)
    );
    assert!(renderer.bitmap_registry.get(bitmap_id).is_none());
    assert!(renderer.canvas.get_native_texture(old_image_id).is_err());
    assert_eq!(renderer.sphere_bitmap_id, None);

    let restored_bitmap_id = renderer
        .register_bitmap(tag, &restored_png)
        .expect("BUG: suspended bitmap restoration should succeed");
    let restored_image_id = renderer
        .bitmap_registry
        .get(restored_bitmap_id)
        .expect("BUG: restored bitmap must have an image");
    assert_eq!(restored_bitmap_id, bitmap_id);
    assert_ne!(restored_image_id, old_image_id);
    assert_eq!(
        renderer.bitmap_tag_state(tag),
        AssetTagState::Resident(bitmap_id)
    );

    drain_gl_errors(&harness.gl);
    renderer.begin_frame(64, 64, 1.0);
    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        restored_bitmap_id,
        0.0,
        0.25,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    renderer.flush();
    let err = unsafe { harness.gl.get_error() };
    assert_eq!(
        err,
        glow::NO_ERROR,
        "draw_sphere after bitmap restoration produced GL 0x{err:04X}"
    );
    assert_eq!(renderer.sphere_bitmap_id, Some(restored_bitmap_id));

    let pixels = read_pixels_top_down(&harness.gl, fbo, 64, 64);
    let center = pixels[32 * 64 + 32];
    assert!(
        center[2] > 240 && center[0] < 16 && center[1] < 16,
        "restored blue texture must be sampled at sphere center, got {center:?}"
    );
}

#[test]
fn evicting_sphere_bitmap_namespace_clears_cached_binding() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, 0, 0) }
        .expect("BUG: renderer init failed");
    let tag = "widget-42:sphere";
    let png = one_px_png([255, 0, 0, 255]);
    let bitmap_id = renderer
        .register_bitmap(tag, &png)
        .expect("BUG: bitmap registration should succeed");

    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        bitmap_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    assert_eq!(renderer.sphere_bitmap_id, Some(bitmap_id));

    assert_eq!(renderer.evict_prefix("widget-42"), 1);
    assert_eq!(renderer.bitmap_tag_state(tag), AssetTagState::Unknown);
    assert!(renderer.bitmap_registry.get(bitmap_id).is_none());
    assert_eq!(renderer.sphere_bitmap_id, None);
}

/// The sphere rasterises into an FBO of its own, and leaving either that or
/// the default framebuffer bound sends the rest of the frame somewhere the
/// caller never asked for.
#[test]
fn sphere_pass_restores_the_frames_framebuffer() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, 64, 64);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let bitmap_id = renderer
        .register_bitmap("widget-42:sphere", &one_px_png([255, 0, 0, 255]))
        .expect("BUG: bitmap registration should succeed");

    renderer.begin_frame(64, 64, 1.0);
    // The sphere is smaller than the frame, so a leaked viewport is visible
    // as a different rectangle rather than an identical one.
    unsafe { harness.gl.viewport(0, 0, 128, 96) };
    let state = || unsafe {
        let mut viewport = [0; 4];
        harness
            .gl
            .get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
        (
            harness.gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING),
            viewport,
        )
    };
    let before = state();
    assert_eq!(
        before,
        (fbo_id.cast_signed(), [0, 0, 128, 96]),
        "the frame must start on the renderer's target and viewport"
    );

    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        bitmap_id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );

    assert_eq!(
        state(),
        before,
        "the sphere's offscreen pass must hand back the frame's framebuffer and viewport"
    );
}

/// `draw_sphere` with an unregistered (or already-evicted) `BitmapId`
/// must skip the GL path entirely rather than sampling whatever
/// texture was last bound.
#[test]
fn sphere_skips_draw_on_registry_miss() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), 64, 64, 0, 0) }
        .expect("BUG: renderer init failed");

    let png = one_px_png([255, 255, 255, 255]);
    let id = renderer
        .register_bitmap("sphere:gone", &png)
        .expect("BUG: register");
    assert_eq!(renderer.evict_prefix("sphere:gone"), 1);

    drain_gl_errors(&harness.gl);
    renderer.draw_sphere(
        0.0,
        0.0,
        64.0,
        64.0,
        id,
        0.0,
        0.0,
        2.0,
        f32::NAN,
        f32::NAN,
        false,
    );
    let err = unsafe { harness.gl.get_error() };
    assert_eq!(
        err,
        glow::NO_ERROR,
        "draw_sphere on missing bitmap produced GL 0x{err:04X}"
    );
    assert_eq!(
        renderer.sphere_bitmap_id, None,
        "BUG: registry miss must invalidate the cached binding",
    );
}
