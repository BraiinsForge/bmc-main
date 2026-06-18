// Copyright (C) 2026  Braiins Systems s.r.o.

//! Aliasing/install/use/clear/trap-path coverage for
//! `WasmWidgetRuntime::with_renderer`.
//!
//! Run under Miri:
//!     MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-strict-provenance" \
//!         cargo +nightly miri test --test with_renderer_aliasing

#![cfg(target_os = "linux")]

use std::fmt::Write as _;
use std::ptr::NonNull;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, WasmWidgetRuntime};
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

mod common;
use common::headless_egl;

fn probe_wat() -> &'static str {
    r#"
    (module
      (memory (export "memory") 1)
      (func (export "__bmc_sdk_init") (result i64)
        i64.const 131072)
      (func (export "render") (param i32)))
    "#
}

fn build(gl: &headless_egl::HeadlessGl) -> (WasmWidgetRuntime, bmc_render::gpu::FemtoVgRenderer) {
    let wasm = wat::parse_str(probe_wat()).expect("BUG: probe WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let renderer =
        unsafe { bmc_render::gpu::FemtoVgRenderer::new(&mut proc, 320, 240, gl.fbo_id, 0) }
            .expect("BUG: probe renderer must construct");
    let runtime = WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(320, 240),
        RuntimeConfig::default(),
    )
    .expect("BUG: probe runtime must construct");
    (runtime, renderer)
}

fn renderer_ptr(renderer: &mut bmc_render::gpu::FemtoVgRenderer) -> NonNull<dyn Renderer> {
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(*renderer);
    NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null")
}

#[test]
fn install_use_clear_cycle() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let (mut runtime, mut renderer) = build(&gl);

    renderer.begin_frame(320, 240, 1.0);
    let ptr = renderer_ptr(&mut renderer);
    let status = runtime
        .with_renderer(ptr, |rt| rt.render(16))
        .expect("BUG: probe render must succeed");
    assert!(matches!(status, bmc_wasm_runtime::RenderStatus::Ok));
    renderer.flush();

    // After exit, the pointer must be cleared; the next render scope must
    // re-install it.
    renderer.begin_frame(320, 240, 1.0);
    let ptr = renderer_ptr(&mut renderer);
    let status = runtime
        .with_renderer(ptr, |rt| rt.render(16))
        .expect("BUG: probe render must succeed");
    assert!(matches!(status, bmc_wasm_runtime::RenderStatus::Ok));
    renderer.flush();
}

/// WAT widget that paints one rectangle from `render` via `host_fill_rect`.
/// Exercises the parked-pointer reborrow path end-to-end.
fn painting_wat() -> &'static str {
    r#"
    (module
      (import "env" "host_fill_rect"
        (func $host_fill_rect (param i32 i32 i32 i32 i32)))
      (memory (export "memory") 1)
      (func (export "__bmc_sdk_init") (result i64)
        i64.const 131072)
      (func (export "render") (param i32)
        i32.const 0
        i32.const 0
        i32.const 10
        i32.const 10
        i32.const 4278190335
        call $host_fill_rect))
    "#
}

#[test]
fn host_import_reborrows_parked_pointer() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let wasm = wat::parse_str(painting_wat()).expect("BUG: painting WAT must parse");
    let mut proc = gl.proc_address();
    let mut renderer =
        unsafe { bmc_render::gpu::FemtoVgRenderer::new(&mut proc, 320, 240, gl.fbo_id, 0) }
            .expect("BUG: probe renderer must construct");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(320, 240),
        RuntimeConfig::default(),
    )
    .expect("BUG: probe runtime must construct");

    renderer.begin_frame(320, 240, 1.0);
    let ptr = renderer_ptr(&mut renderer);
    let status = runtime
        .with_renderer(ptr, |rt| rt.render(16))
        .expect("BUG: render must succeed");
    renderer.flush();
    assert!(matches!(status, bmc_wasm_runtime::RenderStatus::Ok));
}

#[test]
fn host_import_outside_render_scope_traps_guest() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let wasm = wat::parse_str(painting_wat()).expect("BUG: painting WAT must parse");
    let mut proc = gl.proc_address();
    let _renderer =
        unsafe { bmc_render::gpu::FemtoVgRenderer::new(&mut proc, 320, 240, gl.fbo_id, 0) }
            .expect("BUG: probe renderer must construct");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(320, 240),
        RuntimeConfig::default(),
    )
    .expect("BUG: probe runtime must construct");

    // No `with_renderer` bracket — the painting import inside `render` must trap
    // the guest, surfacing as `RenderStatus::Dead` (after fuel-strike accumulation)
    // or an immediate `Err` depending on how the runtime classifies host traps.
    // Either way, the host must NOT panic.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.render(16)));
    let result = outcome.expect("BUG: host must not panic on out-of-scope renderer access");
    match result {
        // Either wasmi trap reported as Err or fuel-strike accumulation to Dead — both acceptable.
        Err(_) | Ok(bmc_wasm_runtime::RenderStatus::Dead) => {}
        Ok(other) => panic!(
            "expected wasmi trap or Dead status, got {other:?} — renderer was accessed without trap"
        ),
    }
}

/// `with_renderer` is intentionally not panic-safe: if `f` panics, the
/// pointer is left set on `HostState`. The contract is that nothing
/// reachable from `HostState::drop` may then observe the stale pointer.
/// If a future `Drop` impl on `HostState` or anything it owns starts
/// reading `renderer_ptr` on cleanup, this test fails under Miri's
/// Tree-Borrows checker because the caller-owned renderer on the stack
/// has been dropped (or aliased) by the time the drop runs.
#[test]
fn panic_in_with_renderer_closure_leaves_drop_sound() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let (mut runtime, mut renderer) = build(&gl);

    renderer.begin_frame(320, 240, 1.0);
    let ptr = renderer_ptr(&mut renderer);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.with_renderer(ptr, |_rt| panic!("simulated host-import bug"));
    }));
    assert!(
        outcome.is_err(),
        "panic must propagate through with_renderer rather than being swallowed"
    );
    renderer.flush();
    // Drop the renderer first so the parked pointer dangles before
    // `HostState::drop` runs. A future regression that derefs
    // `renderer_ptr` from any Drop reachable from `HostState` then
    // trips Miri with use-after-free instead of producing a fresh
    // valid reborrow that might slip through.
    drop(renderer);
    drop(runtime);
}

fn one_px_png(rgba: [u8; 4]) -> Vec<u8> {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgba(rgba));
    let mut buf = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, ImageFormat::Png)
        .expect("BUG: PNG encode should succeed");
    buf.into_inner()
}

fn registering_wat(png: &[u8], tag: &str) -> String {
    let tag_len = tag.len();
    let png_offset = tag_len;
    let png_len = png.len();

    let mut blob = String::new();
    for b in tag.as_bytes() {
        write!(blob, "\\{b:02x}").expect("BUG: write to String cannot fail");
    }
    for b in png {
        write!(blob, "\\{b:02x}").expect("BUG: write to String cannot fail");
    }

    format!(
        r#"
        (module
          (import "env" "host_register_bitmap"
            (func $host_register_bitmap (param i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{blob}")
          (global $registered (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64)
            i64.const 131072)
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

fn build_with_renderer(gl: &headless_egl::HeadlessGl) -> FemtoVgRenderer {
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    unsafe { FemtoVgRenderer::new(&mut proc, 64, 64, gl.fbo_id, 0) }
        .expect("BUG: renderer must construct")
}

#[test]
fn two_runtimes_share_one_renderer_without_cross_slot_bleeding() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let png = one_px_png([0, 255, 0, 255]);
    let wat_a = registering_wat(&png, "alpha");
    let wat_b = registering_wat(&png, "beta");
    let wasm_a = wat::parse_str(&wat_a).expect("BUG: WAT A must parse");
    let wasm_b = wat::parse_str(&wat_b).expect("BUG: WAT B must parse");

    let mut renderer = build_with_renderer(&gl);
    let mut rt_a = WasmWidgetRuntime::new(
        &wasm_a,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        RuntimeConfig::default(),
    )
    .expect("BUG: runtime A must construct");
    let mut rt_b = WasmWidgetRuntime::new(
        &wasm_b,
        64,
        64,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(64, 64),
        RuntimeConfig::default(),
    )
    .expect("BUG: runtime B must construct");

    let ns_a = rt_a.asset_namespace();
    let ns_b = rt_b.asset_namespace();

    renderer.begin_frame(64, 64, 1.0);
    let status = rt_a
        .with_renderer(renderer_ptr(&mut renderer), |rt| rt.render(16))
        .expect("BUG: render A must succeed");
    assert!(matches!(status, RenderStatus::Ok));
    renderer.flush();

    renderer.begin_frame(64, 64, 1.0);
    let status = rt_b
        .with_renderer(renderer_ptr(&mut renderer), |rt| rt.render(16))
        .expect("BUG: render B must succeed");
    assert!(matches!(status, RenderStatus::Ok));
    renderer.flush();

    let evicted_a = renderer.evict_prefix(&ns_a);
    assert_eq!(evicted_a, 1, "runtime A's bitmap must evict exactly once");

    let evicted_b = renderer.evict_prefix(&ns_b);
    assert_eq!(
        evicted_b, 1,
        "runtime B's bitmap must survive A's eviction sweep"
    );
}
