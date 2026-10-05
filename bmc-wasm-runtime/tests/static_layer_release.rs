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

//! A widget's captured static layer is released once its tree loses its static half.

#![cfg(target_os = "linux")]

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, TargetContents, WasmWidgetRuntime};
use bmc_wasm_sdk::{AnimProperty, Draw, Easing, LoopMode, Node, PropsData};

#[path = "common/asset_fixtures.rs"]
#[expect(
    dead_code,
    reason = "shared fixtures; this test needs only two of them"
)]
mod asset_fixtures;
mod common;
use asset_fixtures::{renderer_ptr, wat_string_literal};
use common::headless_egl;

const WHITE: Color = Color::from_rgba(0xFF, 0xFF, 0xFF, 0xFF);

fn tree_bytes(node: &Node) -> Vec<u8> {
    bmc_wasm_sdk::serialize_node_to_bytes(node)
}

fn canvas_tree(draw: Draw) -> Vec<u8> {
    tree_bytes(&bmc_wasm_sdk::canvas(PropsData::default(), [draw]))
}

fn static_rect() -> Draw {
    Draw::rect(0.0, 0.0, 32.0, 32.0, WHITE)
}

fn animated_rect() -> Draw {
    static_rect().animate(
        AnimProperty::Alpha,
        1.0,
        0.5,
        10_000,
        Easing::Linear,
        LoopMode::Forever,
    )
}

/// Submits `trees[n]` on the `n`th render, from a table of `(ptr, len)` pairs at address 0.
///
/// Requests a frame each time, so a widget left animating still runs the guest
/// instead of replaying its cached tree.
fn tree_sequence_wat(trees: &[Vec<u8>]) -> String {
    let mut table = Vec::new();
    let mut ptr = trees.len() * 8;
    for tree in trees {
        for word in [ptr, tree.len()] {
            let word = u32::try_from(word).expect("BUG: fixture trees fit in one wasm page");
            table.extend_from_slice(&word.to_le_bytes());
        }
        ptr += tree.len();
    }
    let data: Vec<u8> = table.into_iter().chain(trees.concat()).collect();
    format!(
        r#"
        (module
          (import "env" "host_submit_tree"
            (func $submit_tree (param i32 i32 i32 i32)))
          (import "env" "host_request_frame" (func $request_frame))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (global $frame (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64) i64.const {sdk})
          (func (export "render") (param i32)
            (local $entry i32)
            global.get $frame
            i32.const 8
            i32.mul
            local.set $entry
            local.get $entry
            i32.load
            local.get $entry
            i32.load offset=4
            i32.const 320
            i32.const 240
            call $submit_tree
            call $request_frame
            global.get $frame
            i32.const 1
            i32.add
            global.set $frame))
        "#,
        data = wat_string_literal(&data),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

fn render_frame(runtime: &mut WasmWidgetRuntime, renderer: &mut FemtoVgRenderer) {
    renderer.begin_frame(320, 240, 1.0);
    let status = runtime
        .with_renderer(renderer_ptr(renderer), |runtime| {
            runtime.render(16, TargetContents::Cleared)
        })
        .expect("BUG: probe render must not trap");
    assert!(
        matches!(status, RenderStatus::Ok),
        "probe frame must render"
    );
    renderer.flush();
}

fn layer_blits(runtime: &WasmWidgetRuntime, renderer: &mut FemtoVgRenderer) -> bool {
    renderer.begin_frame(320, 240, 1.0);
    let blitted = renderer.blit_static_layer(&runtime.asset_namespace());
    renderer.flush();
    blitted
}

fn probe(gl: &headless_egl::HeadlessGl, trees: &[Vec<u8>]) -> (WasmWidgetRuntime, FemtoVgRenderer) {
    let wasm = wat::parse_str(tree_sequence_wat(trees)).expect("BUG: probe WAT must parse");
    let mut proc = gl.proc_address();
    // SAFETY: HeadlessGl keeps the GL context current.
    let renderer = unsafe { FemtoVgRenderer::new(&mut proc, 320, 240, gl.fbo_id, 0) }
        .expect("BUG: probe renderer must construct");
    let runtime = WasmWidgetRuntime::new(
        &wasm,
        320,
        240,
        bmc_wasm_protocol::ViewportShape::Rectangular,
        common::test_display(320, 240),
        chrono::Local::now().fixed_offset(),
        RuntimeConfig::default(),
    )
    .expect("BUG: probe runtime must construct");
    (runtime, renderer)
}

#[test]
fn a_tree_without_a_static_half_releases_the_captured_layer() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let (mut runtime, mut renderer) = probe(
        &gl,
        &[canvas_tree(static_rect()), canvas_tree(animated_rect())],
    );

    render_frame(&mut runtime, &mut renderer);
    assert!(
        layer_blits(&runtime, &mut renderer),
        "the static first frame must capture a layer, or the release below proves nothing"
    );

    render_frame(&mut runtime, &mut renderer);
    assert!(
        !layer_blits(&runtime, &mut renderer),
        "a guest frame with no static half must release the layer it no longer uses"
    );
}

#[test]
fn a_static_tree_after_a_release_captures_a_fresh_layer() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };
    let static_tree = canvas_tree(static_rect());
    let (mut runtime, mut renderer) = probe(
        &gl,
        &[
            static_tree.clone(),
            canvas_tree(animated_rect()),
            static_tree,
        ],
    );

    for _ in 0..3 {
        render_frame(&mut runtime, &mut renderer);
    }
    assert!(
        layer_blits(&runtime, &mut renderer),
        "a static tree hashing like the released one must recapture, not reuse the freed layer"
    );
}
