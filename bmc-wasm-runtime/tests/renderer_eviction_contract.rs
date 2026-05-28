// Copyright (C) 2026  Braiins Systems s.r.o.

//! Caller-side renderer-eviction contract for Stage 3 + Stage 5.
//!
//! Stage 3 invariant (locked in here):
//! - `HostState::evict_widget` only sweeps the audio side. Renderer-side
//!   assets stay alive in the caller-owned `FemtoVgRenderer` until the
//!   caller drops the renderer or explicitly calls `renderer.evict_prefix`.
//!
//! Stage 5 hand-off (what this test forces the future maintainer to wire):
//! - When the runtime drops, the host slot must call
//!   `renderer.evict_prefix(&runtime.asset_namespace())` to reclaim
//!   renderer-side atlas entries belonging to that widget. Otherwise the
//!   shared renderer leaks atlas slots on every widget death.
//!
//! Regression direction:
//! - If anyone accidentally adds a renderer-side sweep back into
//!   `HostState::evict_widget` or `WasmWidgetRuntime`'s `Drop`, the
//!   "asset still present after runtime drop" assertion below fails.

#![cfg(target_os = "linux")]

use std::fmt::Write as _;
use std::ptr::NonNull;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, WasmWidgetRuntime};
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

mod common;
use common::headless_egl;

/// Encode a 1×1 RGBA PNG; minimum payload that survives the bitmap
/// decode + atlas-upload path on the renderer.
fn one_px_png(rgba: [u8; 4]) -> Vec<u8> {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgba(rgba));
    let mut buf = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, ImageFormat::Png)
        .expect("BUG: PNG encode should succeed");
    buf.into_inner()
}

/// Hex-encode `bytes` into a `(data (i32.const 0) "\xx\xx…")` segment body.
fn hex_data_segment(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 4);
    for b in bytes {
        write!(s, "\\{b:02x}").expect("BUG: write to String cannot fail");
    }
    s
}

/// WAT widget that, on its first `render`, registers a single bitmap under
/// the guest-supplied tag `"album"` (host expands to
/// `<guest_id>:album` before storing in the renderer registry).
///
/// PNG bytes are baked into a `data` segment at offset `tag_len`, with the
/// tag string `"album"` at offset 0.
fn registering_wat(png: &[u8]) -> String {
    let tag = "album";
    let tag_len = tag.len();
    let png_offset = tag_len; // place PNG right after the tag in linear memory
    let png_len = png.len();

    // Tag bytes followed by PNG bytes. `data (i32.const 0)` is the active
    // initializer that copies both into linear memory before init runs.
    let mut blob = String::new();
    for b in tag.as_bytes() {
        write!(blob, "\\{b:02x}").expect("BUG: write to String cannot fail");
    }
    blob.push_str(&hex_data_segment(png));

    format!(
        r#"
        (module
          (import "env" "host_register_bitmap"
            (func $host_register_bitmap (param i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{blob}")
          (global $registered (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_version") (result i64)
            i64.const 65536)
          (func (export "render") (param i32)
            (if (i32.eqz (global.get $registered))
              (then
                (drop
                  (call $host_register_bitmap
                    (i32.const 0)
                    (i32.const {tag_len})
                    (i32.const {png_offset})
                    (i32.const {png_len})))
                (global.set $registered (i32.const 1))))))
        "#
    )
}

#[test]
fn renderer_keeps_widget_assets_alive_until_explicit_evict() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let png = one_px_png([255, 0, 0, 255]);
    let wat_src = registering_wat(&png);
    let wasm = wat::parse_str(&wat_src).expect("BUG: registering WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer construct");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        RuntimeConfig::default(),
    )
    .expect("BUG: runtime construct");

    // Capture the asset namespace BEFORE the runtime drops; the host-side
    // GuestId is the only handle we have on the prefix the renderer uses.
    let namespace = runtime.asset_namespace();

    // One render frame: the WAT registers the bitmap into the
    // caller-owned renderer through `host_register_bitmap`.
    renderer.begin_frame(64, 64, 1.0);
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(renderer);
    let ptr = NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null");
    let status = runtime
        .with_renderer(ptr, |rt| rt.render(16))
        .expect("BUG: render must succeed");
    assert!(matches!(status, RenderStatus::Ok));
    renderer.flush();

    // Drop the runtime. Stage 3 contract: this MUST NOT reach into the
    // caller-owned renderer to sweep atlas entries. The renderer-side
    // entry must still be present afterwards.
    drop(runtime);

    // No probe API exists for "is this tag registered?", so we use
    // `evict_prefix` as the introspection: a non-zero return proves the
    // entry was still present at the moment of the call.
    let evicted = renderer.evict_prefix(&namespace);
    assert!(
        evicted > 0,
        "BUG: dropping `WasmWidgetRuntime` swept renderer-side assets — \
         stage 3 requires the caller to retain them until an explicit \
         `renderer.evict_prefix(&namespace)` call (the stage-5 hand-off \
         this test guards)",
    );

    // And a second sweep proves it: nothing remains under the namespace.
    assert_eq!(
        renderer.evict_prefix(&namespace),
        0,
        "BUG: residual entries remained under {namespace:?} after the \
         first explicit eviction",
    );
}
