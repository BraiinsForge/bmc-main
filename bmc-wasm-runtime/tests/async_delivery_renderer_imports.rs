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

//! Renderer-backed host imports must be available while async callbacks run.
//!
//! Regression coverage for dynamic assets such as media-control album art:
//! the guest callback is not `render()`, but it still legitimately registers
//! renderer-side bitmap assets before requesting the next frame.

#![cfg(target_os = "linux")]

use std::fmt::Write as _;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_wasm_runtime::{RuntimeConfig, WasmWidgetRuntime};
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

mod common;
use common::headless_egl;

fn one_px_png(rgba: [u8; 4]) -> Vec<u8> {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgba(rgba));
    let mut buf = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, ImageFormat::Png)
        .expect("BUG: PNG encode should succeed");
    buf.into_inner()
}

fn wat_string_literal(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4);
    for b in bytes {
        write!(out, "\\{b:02x}").expect("BUG: write to String cannot fail");
    }
    out
}

#[derive(Clone, Copy)]
enum CallbackAction {
    None,
    RegisterBitmap,
    DrawText,
    Button,
}

fn callback_action_wat(action: CallbackAction, tag_ptr: usize, tag_len: usize) -> String {
    match action {
        CallbackAction::None => String::new(),
        CallbackAction::RegisterBitmap => format!(
            r"
            (drop
              (call $host_register_bitmap
                (i32.const {tag_ptr})
                (i32.const {tag_len})
                (local.get $body_ptr)
                (local.get $body_len)))",
        ),
        CallbackAction::DrawText => format!(
            r"
            (call $host_draw_text
              (i32.const {tag_ptr})
              (i32.const {tag_len})
              (i32.const 0)
              (i32.const 0)
              (i32.const 12)
            (i32.const -1))",
        ),
        CallbackAction::Button => format!(
            r"
            (drop
              (call $host_button
                (i32.const {tag_ptr})
                (i32.const {tag_len})
                (i32.const {tag_ptr})
                (i32.const {tag_len})
                (i32.const 0)
                (i32.const 0)
                (i32.const 32)
                (i32.const 16)
                (i32.const 0)))",
        ),
    }
}

fn fetch_callback_wat(action: CallbackAction, initial_bitmap: Option<&[u8]>) -> String {
    let method = b"GET";
    let url = b"https://example.test/art.png";
    let tag = b"album_art";

    let method_ptr = 0;
    let url_ptr = method_ptr + method.len();
    let tag_ptr = url_ptr + url.len();

    let mut data = Vec::new();
    data.extend_from_slice(method);
    data.extend_from_slice(url);
    data.extend_from_slice(tag);
    let initial_bitmap_ptr = data.len();
    if let Some(initial_bitmap) = initial_bitmap {
        data.extend_from_slice(initial_bitmap);
    }
    let data = wat_string_literal(&data);
    let callback_action = callback_action_wat(action, tag_ptr, tag.len());
    let register_initial = initial_bitmap.map(|bytes| {
        format!(
            r#"
          (func (export "register_initial") (result i32)
            (call $host_register_bitmap
              (i32.const {tag_ptr})
              (i32.const {tag_len})
              (i32.const {initial_bitmap_ptr})
              (i32.const {initial_bitmap_len})))"#,
            tag_len = tag.len(),
            initial_bitmap_len = bytes.len(),
        )
    });

    format!(
        r#"
        (module
          (import "env" "host_fetch"
            (func $host_fetch
              (param i32 i32 i32 i32 i32 i32 i32 i32 i32)
              (result i32)))
          (import "env" "host_register_bitmap"
            (func $host_register_bitmap
              (param i32 i32 i32 i32)
              (result i32)))
          (import "env" "host_draw_text"
            (func $host_draw_text
              (param i32 i32 i32 i32 i32 i32)))
          (import "env" "host_button"
            (func $host_button
              (param i32 i32 i32 i32 i32 i32 i32 i32 i32)
              (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $callback_count (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64)
            i64.const {sdk})
          (func (export "__alloc") (param $len i32) (result i32)
            (i32.const 1024))
          (func (export "init")
            (drop
              (call $host_fetch
                (i32.const 10000)
                (i32.const {method_ptr})
                (i32.const {method_len})
                (i32.const {url_ptr})
                (i32.const {url_len})
                (i32.const 0)
                (i32.const 0)
                (i32.const 0)
                (i32.const 0))))
          (func (export "__on_fetch_response")
            (param $request_id i32)
            (param $status i32)
            (param $body_ptr i32)
            (param $body_len i32)
            (global.set $callback_count
              (i32.add (global.get $callback_count) (i32.const 1)))
            {callback_action})
          (func (export "callback_count") (result i32)
            (global.get $callback_count))
          {register_initial}
          (func (export "render") (param i32))
        )
        "#,
        method_len = method.len(),
        url_len = url.len(),
        callback_action = callback_action,
        register_initial = register_initial.unwrap_or_default(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

fn poll_fetch_delivery(
    runtime: &mut WasmWidgetRuntime,
    renderer: &mut FemtoVgRenderer,
) -> (bool, usize) {
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(*renderer);
    let ptr = NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut renderer_accessed = false;
    let gpu_acquisitions = std::cell::Cell::new(0);
    while runtime.has_pending_fetches() && Instant::now() < deadline {
        renderer_accessed |= runtime
            .poll_deliveries_with_renderer_and_gpu_access(ptr, || {
                gpu_acquisitions.set(gpu_acquisitions.get() + 1);
                Ok(())
            })
            .expect("BUG: fixture delivery must not trap");
        std::thread::yield_now();
    }
    renderer_accessed |= runtime
        .poll_deliveries_with_renderer_and_gpu_access(ptr, || {
            gpu_acquisitions.set(gpu_acquisitions.get() + 1);
            Ok(())
        })
        .expect("BUG: fixture delivery must not trap");
    assert_eq!(
        runtime.call_export_i32("callback_count"),
        Some(1),
        "the intercepted fetch must reach its guest callback"
    );
    (renderer_accessed, gpu_acquisitions.get())
}

#[test]
fn fetch_response_callback_can_register_renderer_asset() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let png = one_px_png([0, 255, 0, 255]);
    let wasm = wat::parse_str(fetch_callback_wat(CallbackAction::RegisterBitmap, None))
        .expect("BUG: callback WAT must parse");
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
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(move |_method, _url| Some((200, png.clone())))),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime construct");
    let namespace = runtime.asset_namespace();

    let (gpu_accessed, gpu_acquisitions) = poll_fetch_delivery(&mut runtime, &mut renderer);
    assert!(
        gpu_accessed,
        "delivery must report the renderer import so the host fences its GPU work"
    );
    assert_eq!(
        gpu_acquisitions, 1,
        "one upload must acquire GPU access once"
    );

    assert!(
        renderer.evict_prefix(&namespace) > 0,
        "fetch delivery callback failed to register a renderer-side asset",
    );
}

#[test]
fn fetch_response_callback_without_renderer_import_needs_no_gpu_fence() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let wasm = wat::parse_str(fetch_callback_wat(CallbackAction::None, None))
        .expect("BUG: callback WAT must parse");
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
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| Some((200, Vec::new())))),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime construct");

    let (gpu_accessed, gpu_acquisitions) = poll_fetch_delivery(&mut runtime, &mut renderer);
    assert!(
        !gpu_accessed,
        "a pure guest callback must not request a GPU completion fence"
    );
    assert_eq!(gpu_acquisitions, 0, "a pure callback must remain lock-free");
}

#[test]
fn fetch_response_callback_querying_a_resident_asset_needs_no_gpu_fence() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let png = one_px_png([0, 255, 0, 255]);
    let wasm = wat::parse_str(fetch_callback_wat(
        CallbackAction::RegisterBitmap,
        Some(&png),
    ))
    .expect("BUG: callback WAT must parse");
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
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(move |_method, _url| Some((200, png.clone())))),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime construct");
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(renderer);
    let ptr = NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null");
    let initial_id = runtime
        .with_renderer(ptr, |runtime| runtime.call_export_i32("register_initial"))
        .expect("BUG: fixture exports register_initial");
    assert_ne!(
        initial_id, 0,
        "the fixture must make the callback registration a resident lookup"
    );

    let (gpu_accessed, gpu_acquisitions) = poll_fetch_delivery(&mut runtime, &mut renderer);
    assert!(
        !gpu_accessed,
        "a resident asset lookup must not request a GPU completion fence"
    );
    assert_eq!(
        gpu_acquisitions, 0,
        "resident lookup must not acquire GPU access"
    );
}

#[test]
fn fetch_response_callback_drawing_text_acquires_gpu_access() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let wasm = wat::parse_str(fetch_callback_wat(CallbackAction::DrawText, None))
        .expect("BUG: callback WAT must parse");
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
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| Some((200, Vec::new())))),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime construct");

    let (gpu_accessed, gpu_acquisitions) = poll_fetch_delivery(&mut runtime, &mut renderer);
    assert!(
        gpu_accessed,
        "a drawing import during delivery must request a GPU completion fence"
    );
    assert_eq!(
        gpu_acquisitions, 1,
        "drawing imports in one callback must acquire GPU access once"
    );
}

#[test]
fn fetch_response_callback_drawing_button_acquires_gpu_access() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let wasm = wat::parse_str(fetch_callback_wat(CallbackAction::Button, None))
        .expect("BUG: callback WAT must parse");
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
        chrono::Local::now().fixed_offset(),
        RuntimeConfig {
            fetch_interceptor: Some(Box::new(|_method, _url| Some((200, Vec::new())))),
            ..RuntimeConfig::default()
        },
    )
    .expect("BUG: runtime construct");

    let (gpu_accessed, gpu_acquisitions) = poll_fetch_delivery(&mut runtime, &mut renderer);
    assert!(
        gpu_accessed,
        "a button import during delivery must request a GPU completion fence"
    );
    assert_eq!(
        gpu_acquisitions, 1,
        "a button import must acquire GPU access once"
    );
}
