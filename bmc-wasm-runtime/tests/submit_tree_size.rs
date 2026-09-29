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

//! A tree submitted at a size other than the widget surface's is refused.
//!
//! The static layer is allocated at the tree's size while the raw blit
//! covers the whole viewport, so an accepted mismatch would stretch the
//! static half over the buffer.

#![cfg(target_os = "linux")]

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, TargetContents, WasmWidgetRuntime};

#[path = "common/asset_fixtures.rs"]
#[expect(dead_code, reason = "this binary needs none of the asset fixtures")]
mod asset_fixtures;
mod common;
#[path = "common/pixel_readback.rs"]
mod pixel_readback;
use asset_fixtures::{renderer_ptr, wat_string_literal};
use common::headless_egl;

const SURFACE_WIDTH: u32 = 320;
const SURFACE_HEIGHT: u32 = 240;
const GREEN: [u8; 4] = [0, 255, 0, 255];
/// What `begin_frame` clears the surface to.
const CLEARED: [u8; 4] = [0, 0, 0, 255];
/// Screen (80, 60): inside the smallest rect a test claims, 160x120.
/// `read_pixels` counts y from the bottom.
const PROBE_X: i32 = 80;
const PROBE_Y: i32 = 180;

/// A widget submitting a static green rect covering the size it claims.
fn claimed_size_wat(width: u16, height: u16) -> String {
    let node = bmc_wasm_sdk::canvas(
        bmc_wasm_sdk::PropsData::default(),
        [bmc_wasm_sdk::Draw::rect(
            0.0,
            0.0,
            f32::from(width),
            f32::from(height),
            Color::from_rgb(0, 255, 0),
        )],
    );
    let tree = bmc_wasm_sdk::serialize_node_to_bytes(&node);
    format!(
        r#"
        (module
          (import "env" "host_submit_tree"
            (func $submit_tree (param i32 i32 i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32)
            i32.const 0
            i32.const {tree_len}
            i32.const {width}
            i32.const {height}
            call $submit_tree))
        "#,
        data = wat_string_literal(&tree),
        tree_len = tree.len(),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

/// The probe pixel after each of two frames of a widget claiming `width`x`height`.
///
/// A widget at the surface's size captures the static layer on the first frame
/// and reuses it on the second.
fn probe_pixel_per_frame(gl: &headless_egl::HeadlessGl, width: u16, height: u16) -> [[u8; 4]; 2] {
    let wasm = wat::parse_str(claimed_size_wat(width, height)).expect("BUG: probe WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let mut renderer =
        unsafe { FemtoVgRenderer::new(&mut proc, SURFACE_WIDTH, SURFACE_HEIGHT, gl.fbo_id, 0) }
            .expect("BUG: probe renderer must construct");
    let mut runtime = WasmWidgetRuntime::new(
        &wasm,
        SURFACE_WIDTH,
        SURFACE_HEIGHT,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(SURFACE_WIDTH, SURFACE_HEIGHT),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: probe runtime must construct");

    std::array::from_fn(|_| {
        renderer.begin_frame(SURFACE_WIDTH, SURFACE_HEIGHT, 1.0);
        let status = runtime
            .with_renderer(renderer_ptr(&mut renderer), |runtime| {
                runtime.render(16, TargetContents::Cleared)
            })
            .expect("BUG: probe render must not trap");
        assert_eq!(status, RenderStatus::Ok);
        renderer.flush();
        gl.read_pixel(PROBE_X, PROBE_Y)
    })
}

#[test]
fn a_tree_at_the_surface_size_is_painted() {
    let Some(gl) = headless_egl::try_init(SURFACE_WIDTH, SURFACE_HEIGHT) else {
        return;
    };
    assert_eq!(
        probe_pixel_per_frame(&gl, 320, 240),
        [GREEN; 2],
        "a tree matching the surface must paint on both frames, \
         or the refusal test proves nothing"
    );
}

#[test]
fn a_tree_at_another_size_is_refused() {
    let Some(gl) = headless_egl::try_init(SURFACE_WIDTH, SURFACE_HEIGHT) else {
        return;
    };
    assert_eq!(
        probe_pixel_per_frame(&gl, 160, 120),
        [CLEARED; 2],
        "a tree claiming half the surface must be dropped on every frame, \
         neither stretched over it nor drawn at its claimed size"
    );
}
