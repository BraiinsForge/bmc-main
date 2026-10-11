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

use super::*;
use crate::test_harness::GlHarness;

/// 1×1 transparent RGBA PNG, encoded each call to keep the test
/// self-contained (no embedded fixture bytes to drift).
fn minimal_png() -> Vec<u8> {
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgba([0, 0, 0, 0]));
    let mut buf = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, ImageFormat::Png)
        .expect("BUG: PNG encode should succeed");
    buf.into_inner()
}

/// Solid-color RGB JPEG of the given size, encoded each call.
fn solid_jpeg(w: u32, h: u32) -> Vec<u8> {
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgb};

    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(w, h, Rgb([20, 120, 200]));
    let mut buf = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(img)
        .write_to(&mut buf, ImageFormat::Jpeg)
        .expect("BUG: JPEG encode should succeed");
    buf.into_inner()
}

#[test]
fn jpeg_far_larger_than_viewport_bounds_to_it() {
    let jpeg = solid_jpeg(4000, 3000);
    let (rgba, w, h) = decode_scaled_to_fit(&jpeg, 640, 480).expect("BUG: large JPEG decode");
    assert!(w <= 640 && h <= 480, "not bounded: {w}x{h}");
    assert_eq!(rgba.len(), (w * h * 4) as usize);
    // 4:3 source into a 4:3 viewport fills it exactly.
    assert_eq!((w, h), (640, 480));
}

#[test]
fn jpeg_smaller_than_viewport_is_not_upscaled() {
    let jpeg = solid_jpeg(100, 80);
    let (_rgba, w, h) = decode_scaled_to_fit(&jpeg, 640, 480).expect("BUG: small JPEG decode");
    assert_eq!((w, h), (100, 80));
}

/// Solid-color RGBA WebP of the given size, encoded each call.
fn solid_webp(w: u32, h: u32) -> Vec<u8> {
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(w, h, Rgba([20, 120, 200, 255]));
    let mut buf = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, ImageFormat::WebP)
        .expect("BUG: WebP encode should succeed");
    buf.into_inner()
}

#[test]
fn non_jpeg_decodes_within_bounds() {
    let png = minimal_png();
    let (rgba, w, h) = decode_scaled_to_fit(&png, 640, 480).expect("BUG: PNG decode");
    assert!(w <= 640 && h <= 480);
    assert_eq!(rgba.len(), (w * h * 4) as usize);
}

/// WebP has no scale-on-load, so it full-decodes first
/// and only the resize afterwards bounds it.
#[test]
fn webp_larger_than_viewport_bounds_to_it() {
    let webp = solid_webp(1200, 900);
    let (rgba, w, h) = decode_scaled_to_fit(&webp, 640, 480).expect("BUG: WebP decode");
    assert!(w <= 640 && h <= 480, "not bounded: {w}x{h}");
    assert_eq!(rgba.len(), (w * h * 4) as usize);
}

/// Solid-color RGBA PNG of the given size, encoded each call.
fn solid_png(w: u32, h: u32) -> Vec<u8> {
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(w, h, Rgba([20, 120, 200, 255]));
    let mut buf = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, ImageFormat::Png)
        .expect("BUG: PNG encode should succeed");
    buf.into_inner()
}

#[test]
fn register_refuses_source_past_pixel_budget() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    // 2049x2049 is one row/col past the 2048x2048 (MAX_DECODE_IMAGE_PIXELS) cap.
    let png = solid_png(2049, 2049);
    assert!(
        reg.register("crate::over_budget", &png, &mut canvas, ImageFlags::empty())
            .is_none(),
        "a source past the pixel budget must be refused, not decoded"
    );
}

#[test]
fn evict_removes_tag_id_and_image() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    let png = minimal_png();

    let id = reg
        .register("crate::bmp", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: register should succeed");
    assert!(reg.get(id).is_some());

    assert_eq!(reg.evict_prefix("crate::bmp", &mut canvas), 1);
    assert!(reg.get(id).is_none());
    // Idempotent: a second evict on the same tag is a no-op.
    assert_eq!(reg.evict_prefix("crate::bmp", &mut canvas), 0);
}

#[test]
fn evict_prefix_only_touches_matching_tags() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    let png = minimal_png();

    let _ = reg
        .register("a:1", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: a:1");
    let _ = reg
        .register("a:2", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: a:2");
    let id_b = reg
        .register("b:1", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: b:1");

    assert_eq!(reg.evict_prefix("a", &mut canvas), 2);
    assert!(reg.get(id_b).is_some());
}

#[test]
fn evict_prefix_respects_segment_boundaries() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    let png = minimal_png();

    let id_foo = reg
        .register("foo", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: foo");
    let id_foobar = reg
        .register("foobar", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: foobar");
    let id_foo_child = reg
        .register("foo:child", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: foo:child");

    assert_eq!(reg.evict_prefix("foo", &mut canvas), 2);
    assert!(reg.get(id_foo).is_none());
    assert!(reg.get(id_foo_child).is_none());
    assert!(reg.get(id_foobar).is_some());
}

#[test]
fn register_after_evict_reuses_released_id() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    let png = minimal_png();

    let id1 = reg
        .register("ephemeral", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: first register");
    assert_eq!(reg.evict_prefix("ephemeral", &mut canvas), 1);
    let id2 = reg
        .register("ephemeral", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: re-register");
    assert_eq!(id1, id2);
}

#[test]
fn clear_restarts_bitmap_id_allocation() {
    let mut registry = BitmapRegistry::new();
    let first = registry
        .reserve("first", ImageFlags::empty())
        .expect("BUG: first reservation should succeed");
    let _second = registry
        .reserve("second", ImageFlags::empty())
        .expect("BUG: second reservation should succeed");

    registry.reservations.clear();

    assert_eq!(
        registry.reserve("replacement", ImageFlags::empty()),
        Some(first)
    );
}

#[test]
fn suspend_compressed_bitmap_releases_payload_and_restores_reservation() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    let png = minimal_png();

    let id = reg
        .register("widget:bitmap", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: compressed registration should succeed");
    let image_id = reg.get(id).expect("BUG: compressed image should exist");
    assert_eq!(reg.tag_state("widget:bitmap"), AssetTagState::Resident(id));
    assert!(reg.resident_bytes() > 0);

    assert_eq!(
        reg.suspend_exact("widget:bitmap", &mut canvas),
        AssetSuspendResult::Suspended(id)
    );
    assert_eq!(reg.tag_state("widget:bitmap"), AssetTagState::Suspended(id));
    assert!(canvas.image_info(image_id).is_err());
    assert_eq!(reg.resident_bytes(), 0);
    assert!(reg.get(id).is_none());
    assert_eq!(
        reg.suspend_exact("widget:bitmap", &mut canvas),
        AssetSuspendResult::AlreadySuspended(id)
    );

    assert_eq!(
        reg.register("widget:bitmap", &png, &mut canvas, ImageFlags::NEAREST),
        Some(id)
    );
    assert_eq!(reg.tag_state("widget:bitmap"), AssetTagState::Resident(id));
}

#[test]
fn exact_reservation_and_suspension_preserve_bitmap_id_and_sampling() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut registry = BitmapRegistry::new();
    let id = registry
        .reserve("widget:bitmap", ImageFlags::NEAREST)
        .expect("BUG: bitmap reservation should succeed");

    assert_eq!(
        registry.suspend_exact("widget:bitmap", &mut canvas),
        AssetSuspendResult::AlreadySuspended(id)
    );
    assert_eq!(registry.reserve("widget:bitmap", ImageFlags::empty()), None);
    assert_eq!(
        registry.register(
            "widget:bitmap",
            &minimal_png(),
            &mut canvas,
            ImageFlags::NEAREST,
        ),
        Some(id)
    );
    assert_eq!(
        registry.suspend_exact("widget:bitmap", &mut canvas),
        AssetSuspendResult::Suspended(id)
    );
    assert_eq!(
        registry.suspend_exact("missing", &mut canvas),
        AssetSuspendResult::Unknown
    );
}

#[test]
fn suspend_rgba_bitmap_releases_payload_without_sampling_copy() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    let rgba = [10, 20, 30, 255];

    let id = reg
        .register_rgba("widget:rgba", &rgba, 1, 1, &mut canvas, ImageFlags::empty())
        .expect("BUG: RGBA registration should succeed");
    let image_id = reg.get(id).expect("BUG: RGBA image should exist");
    assert_eq!(reg.tag_state("widget:rgba"), AssetTagState::Resident(id));
    assert!(reg.resident_bytes() > 0);

    assert_eq!(
        reg.suspend_exact("widget:rgba", &mut canvas),
        AssetSuspendResult::Suspended(id)
    );
    assert_eq!(reg.tag_state("widget:rgba"), AssetTagState::Suspended(id));
    assert!(canvas.image_info(image_id).is_err());
    assert_eq!(reg.resident_bytes(), 0);
    assert!(reg.get(id).is_none());
    assert_eq!(
        reg.suspend_exact("widget:rgba", &mut canvas),
        AssetSuspendResult::AlreadySuspended(id)
    );

    assert_eq!(
        reg.register_rgba("widget:rgba", &rgba, 1, 1, &mut canvas, ImageFlags::NEAREST,),
        Some(id)
    );
    assert_eq!(reg.tag_state("widget:rgba"), AssetTagState::Resident(id));
}

#[test]
fn repeated_suspend_and_restore_preserves_allocator_state() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    let png = minimal_png();

    let id = reg
        .register("widget:bitmap", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: registration should succeed");
    for _ in 0..2 {
        assert_eq!(
            reg.suspend_exact("widget:bitmap", &mut canvas),
            AssetSuspendResult::Suspended(id)
        );
        assert_eq!(
            reg.register("widget:bitmap", &png, &mut canvas, ImageFlags::NEAREST),
            Some(id)
        );
    }

    let next = reg
        .register("widget:next", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: next registration should succeed");
    assert_eq!(next.to_wire(), id.to_wire() + 1);
}

#[test]
fn failed_compressed_restore_keeps_reservation_and_allocator_state() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    let png = minimal_png();

    let id = reg
        .register("widget:bitmap", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: registration should succeed");
    assert_eq!(
        reg.suspend_exact("widget:bitmap", &mut canvas),
        AssetSuspendResult::Suspended(id)
    );

    assert_eq!(
        reg.register("widget:bitmap", &[], &mut canvas, ImageFlags::NEAREST),
        None
    );
    assert_eq!(reg.tag_state("widget:bitmap"), AssetTagState::Suspended(id));

    let next = reg
        .register("widget:next", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: next registration should succeed");
    assert_eq!(next.to_wire(), id.to_wire() + 1);
}

#[test]
fn evict_removes_suspended_bitmap_reservation() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    let png = minimal_png();

    let id = reg
        .register("widget:bitmap", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: registration should succeed");
    assert_eq!(
        reg.suspend_exact("widget:bitmap", &mut canvas),
        AssetSuspendResult::Suspended(id)
    );

    assert_eq!(reg.evict_prefix("widget:bitmap", &mut canvas), 1);
    assert_eq!(reg.tag_state("widget:bitmap"), AssetTagState::Unknown);

    let next = reg
        .register("widget:bitmap", &png, &mut canvas, ImageFlags::empty())
        .expect("BUG: re-registration should succeed");
    assert_eq!(next, id);
}

#[test]
fn rgba_replacement_updates_and_preserves_restoration_flags() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut reg = BitmapRegistry::new();
    let png = minimal_png();
    let rgba = [10, 20, 30, 255];

    let id = reg
        .register("widget:bitmap", &png, &mut canvas, ImageFlags::NEAREST)
        .expect("BUG: compressed registration should succeed");
    let original = reg.get(id).expect("BUG: original image should exist");
    assert_eq!(
        reg.register_rgba(
            "widget:bitmap",
            &rgba,
            1,
            1,
            &mut canvas,
            ImageFlags::empty(),
        ),
        Some(id)
    );
    assert!(canvas.image_info(original).is_err());
    let replacement = reg.get(id).expect("BUG: replacement image should exist");
    assert_eq!(
        canvas
            .image_info(replacement)
            .expect("BUG: replacement image info")
            .flags(),
        ImageFlags::empty()
    );

    assert_eq!(
        reg.register_rgba(
            "widget:bitmap",
            &[10, 20, 30],
            1,
            1,
            &mut canvas,
            ImageFlags::NEAREST,
        ),
        None
    );
    assert_eq!(reg.get(id), Some(replacement));
    assert_eq!(
        canvas
            .image_info(replacement)
            .expect("BUG: replacement image info after failed update")
            .flags(),
        ImageFlags::empty()
    );

    assert_eq!(
        reg.suspend_exact("widget:bitmap", &mut canvas),
        AssetSuspendResult::Suspended(id)
    );
    assert_eq!(
        reg.register("widget:bitmap", &png, &mut canvas, ImageFlags::NEAREST),
        Some(id)
    );
    let restored = reg.get(id).expect("BUG: restored image should exist");
    assert_eq!(
        canvas
            .image_info(restored)
            .expect("BUG: restored image info")
            .flags(),
        ImageFlags::empty()
    );
}
