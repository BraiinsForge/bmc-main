// Copyright (C) 2026  Braiins Systems s.r.o.

//! Synthetic single-widget caller-owned-renderer render loop.
//!
//! Regression coverage: if a future refactor accidentally re-introduces
//! internal renderer ownership inside `WasmWidgetRuntime`, this test
//! stops compiling or stops passing.

#![cfg(target_os = "linux")]

use std::ptr::NonNull;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, WasmWidgetRuntime};

mod common;
use common::headless_egl;

/// WAT widget that paints one rectangle from `render` via `host_fill_rect`,
/// so each frame fires a host-import reborrow through `renderer_ptr`.
///
/// Without this, the test only exercises `with_renderer`'s install/clear
/// signature — the parked-pointer deref path lives behind the imports and
/// would never run. With it, ten iterations of the host-import reborrow are
/// the regression surface: if a future refactor breaks the parked-pointer
/// dispatch, this test fails along with the install-once aliasing test.
fn painting_wat() -> String {
    format!(
        r#"
    (module
      (import "env" "host_fill_rect"
        (func $host_fill_rect (param i32 i32 i32 i32 i32)))
      (memory (export "memory") 1)
      (func (export "__bmc_sdk_init") (result i64)
        i64.const {})
      (func (export "render") (param i32)
        i32.const 0
        i32.const 0
        i32.const 10
        i32.const 10
        i32.const 4278190335
        call $host_fill_rect))
    "#,
        bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

#[test]
fn loop_of_ten_frames_succeeds() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let wasm = wat::parse_str(painting_wat()).expect("BUG: painting WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer = unsafe { FemtoVgRenderer::new(&mut proc, 320, 240, gl.fbo_id, 0) }
        .expect("BUG: renderer construct");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(320, 240),
        RuntimeConfig::default(),
    )
    .expect("BUG: runtime construct");

    for _ in 0..10 {
        renderer.begin_frame(320, 240, 1.0);
        let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(renderer);
        let ptr = NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null");
        let status = runtime
            .with_renderer(ptr, |rt| rt.render(16))
            .expect("BUG: render must succeed");
        assert!(matches!(status, RenderStatus::Ok));
        renderer.flush();
    }
}
