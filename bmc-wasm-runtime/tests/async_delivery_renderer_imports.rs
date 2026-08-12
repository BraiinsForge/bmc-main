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

fn fetch_callback_registers_bitmap_wat() -> String {
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
    let data = wat_string_literal(&data);

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
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
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
            (drop
              (call $host_register_bitmap
                (i32.const {tag_ptr})
                (i32.const {tag_len})
                (local.get $body_ptr)
                (local.get $body_len))))
          (func (export "render") (param i32))
        )
        "#,
        method_len = method.len(),
        url_len = url.len(),
        tag_len = tag.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
    )
}

#[test]
fn fetch_response_callback_can_register_renderer_asset() {
    let Some(gl) = headless_egl::try_init(64, 64) else {
        return;
    };
    let png = one_px_png([0, 255, 0, 255]);
    let wasm = wat::parse_str(fetch_callback_registers_bitmap_wat())
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

    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(renderer);
    let ptr = NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null");
    runtime
        .poll_deliveries_with_renderer(ptr)
        .expect("BUG: fixture delivery must not trap");

    assert!(
        renderer.evict_prefix(&namespace) > 0,
        "fetch delivery callback failed to register a renderer-side asset",
    );
}
