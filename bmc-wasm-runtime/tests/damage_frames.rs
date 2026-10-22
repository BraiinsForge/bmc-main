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

//! A damage-tracked frame matches a full repaint, pixel for pixel, every frame.
//!
//! One runtime repaints every frame into a cleared target. The other paints
//! the way the wasm host does: it keeps the target whenever the runtime says it
//! may, and alternates two colour attachments on one framebuffer, so a kept
//! target holds the frame before last, as the export buffers do.

#![cfg(target_os = "linux")]

use std::f32::consts::PI;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::FrameClear;
use bmc_render::renderer::prelude::*;
use bmc_ui::{
    AnimProperty, ArcCap, ArcSegments, Color, Draw, Easing, LoopMode, PropsData, canvas, encode,
};
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, TargetContents, WasmWidgetRuntime};
use glow::HasContext as _;

#[path = "common/asset_fixtures.rs"]
#[expect(
    dead_code,
    reason = "shared fixtures; this test needs only two of them"
)]
mod asset_fixtures;
mod common;
use asset_fixtures::{renderer_ptr, wat_string_literal};
use common::headless_egl;

// `u16` so every use converts losslessly, to `u32`, `f32` or `usize`.
const W: u16 = 320;
const H: u16 = 240;
const CENTRE: (f32, f32) = (160.0, 120.0);
const FRAME_MS: u32 = 16;
/// How long every transition and animation below runs.
const ANIMATION_MS: u32 = 500;
/// Past every animation below. The frames after they settle have nothing to damage,
/// so they are compared on the full-repaint path.
const REPLAYED_FRAMES: usize = 50;

const BACKGROUND: Color = Color::from_rgb(40, 60, 80);
const HAND: Color = Color::from_rgb(240, 240, 240);
const GAUGE: Color = Color::from_rgb(250, 160, 30);

/// Submits `trees[n]` on the `n`th render and the last one after,
/// asking for a frame only after the first,
/// so the frames past the second replay the cached tree while anything animates.
fn two_tree_wat(trees: &[Vec<u8>; 2]) -> String {
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
            i32.const 1
            global.get $frame
            i32.const 1
            i32.lt_u
            select
            i32.const 8
            i32.mul
            local.set $entry
            local.get $entry
            i32.load
            local.get $entry
            i32.load offset=4
            i32.const {W}
            i32.const {H}
            call $submit_tree
            global.get $frame
            i32.eqz
            if
              call $request_frame
            end
            global.get $frame
            i32.const 1
            i32.add
            global.set $frame))
        "#,
        data = wat_string_literal(&data),
        sdk = bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
    )
}

/// A framebuffer with a stencil and two colour textures to attach in turn.
struct RotatingTarget {
    fbo: glow::Framebuffer,
    textures: [glow::Texture; 2],
}

impl RotatingTarget {
    #[expect(
        clippy::cast_possible_wrap,
        reason = "test surface dimensions are tiny"
    )]
    fn new(gl: &glow::Context) -> Self {
        // SAFETY: HeadlessGl keeps its GL context current for the whole test.
        unsafe {
            let textures = [0, 1].map(|_| {
                let texture = gl.create_texture().expect("BUG: create_texture failed");
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    i32::from(W),
                    i32::from(H),
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::NEAREST as i32);
                }
                texture
            });
            let fbo = gl
                .create_framebuffer()
                .expect("BUG: create_framebuffer failed");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            let stencil = gl
                .create_renderbuffer()
                .expect("BUG: create_renderbuffer failed");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(stencil));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::STENCIL_INDEX8,
                i32::from(W),
                i32::from(H),
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::STENCIL_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(stencil),
            );
            let target = Self { fbo, textures };
            target.attach(gl, 0);
            target
        }
    }

    fn fbo_id(&self) -> u32 {
        self.fbo.0.get()
    }

    fn attach(&self, gl: &glow::Context, index: usize) {
        // SAFETY: HeadlessGl keeps its GL context current for the whole test.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(self.textures[index]),
                0,
            );
            assert_eq!(
                gl.check_framebuffer_status(glow::FRAMEBUFFER),
                glow::FRAMEBUFFER_COMPLETE,
                "BUG: the rotating target must stay complete"
            );
        }
    }

    fn read(&self, gl: &glow::Context) -> Vec<u8> {
        let mut pixels = vec![0_u8; usize::from(W) * usize::from(H) * 4];
        // SAFETY: HeadlessGl keeps its GL context current for the whole test.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.read_pixels(
                0,
                0,
                i32::from(W),
                i32::from(H),
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }
        pixels
    }
}

struct Painter {
    runtime: WasmWidgetRuntime,
    renderer: FemtoVgRenderer,
    target: RotatingTarget,
}

impl Painter {
    fn new(gl: &headless_egl::HeadlessGl, wasm: &[u8]) -> Self {
        let target = RotatingTarget::new(gl.as_ref());
        let mut proc = gl.proc_address();
        // SAFETY: HeadlessGl keeps the GL context current.
        let renderer = unsafe {
            FemtoVgRenderer::new(&mut proc, u32::from(W), u32::from(H), target.fbo_id(), 0)
        }
        .expect("BUG: renderer must construct");
        let runtime = WasmWidgetRuntime::new(
            wasm,
            u32::from(W),
            u32::from(H),
            bmc_ui::ViewportShape::Rectangular,
            common::test_display(u32::from(W), u32::from(H)),
            chrono::Local::now().fixed_offset(),
            RuntimeConfig::default(),
        )
        .expect("BUG: runtime must construct");
        Self {
            runtime,
            renderer,
            target,
        }
    }

    fn paint(&mut self, contents: TargetContents) {
        let clear = match contents {
            TargetContents::Preserved => FrameClear::Keep,
            TargetContents::Cleared => FrameClear::OpaqueBlack,
        };
        self.renderer
            .begin_frame_with_clear(u32::from(W), u32::from(H), 1.0, clear);
        let status = self
            .runtime
            .with_renderer(renderer_ptr(&mut self.renderer), |runtime| {
                runtime.render(FRAME_MS, contents)
            })
            .expect("BUG: render must not trap");
        assert!(
            matches!(status, RenderStatus::Ok),
            "every frame must render"
        );
        self.renderer.flush();
    }
}

/// Paint `trees` both ways and require identical pixels after every frame.
fn assert_damage_tracking_matches_full_repaints(trees: &[Vec<u8>; 2]) {
    let Some(gl) = headless_egl::try_init(u32::from(W), u32::from(H)) else {
        return;
    };
    // Both painters bring their own framebuffer rather than the harness's.
    let _force_use = gl.fbo_id;
    let wasm = wat::parse_str(two_tree_wat(trees)).expect("BUG: WAT must parse");
    let mut reference = Painter::new(&gl, &wasm);
    let mut tracked = Painter::new(&gl, &wasm);

    let mut frames = Vec::with_capacity(2 + REPLAYED_FRAMES);
    for frame in 0..2 + REPLAYED_FRAMES {
        reference.paint(TargetContents::Cleared);

        let buffer = frame % 2;
        tracked.target.attach(gl.as_ref(), buffer);
        let asked_for = tracked.runtime.wants_next_frame();
        let keeps_target = tracked
            .runtime
            .next_frame_preserves_target(f32::from(W), f32::from(H));
        frames.push((asked_for, keeps_target));
        let contents = if keeps_target {
            TargetContents::Preserved
        } else {
            TargetContents::Cleared
        };
        tracked.paint(contents);

        let expected = reference.target.read(gl.as_ref());
        let actual = tracked.target.read(gl.as_ref());
        let row_bytes = usize::from(W) * 4;
        // Readback rows run bottom-up.
        let differing: Vec<(usize, usize, &[u8; 4], &[u8; 4])> = expected
            .chunks_exact(row_bytes)
            .zip(actual.chunks_exact(row_bytes))
            .enumerate()
            .flat_map(|(row, (want, got))| {
                want.as_chunks::<4>()
                    .0
                    .iter()
                    .zip(got.as_chunks::<4>().0)
                    .enumerate()
                    .filter(|(_, (want, got))| want != got)
                    .map(move |(x, (want, got))| (x, usize::from(H) - 1 - row, want, got))
            })
            .collect();
        if let Some(&(x, y, want, got)) = differing.first() {
            panic!(
                "frame {frame} ({contents:?}, buffer {buffer}) differs from a full repaint \
                 in {} px, first at ({x}, {y}): expected {want:?}, got {got:?}",
                differing.len(),
            );
        }
    }
    assert_every_animating_frame_took_the_damage_path(&frames);
}

/// The frames before the damage path can start: the two guest frames,
/// then the first replay, which catches the second export buffer up on them.
const FULL_REPAINTS: usize = 3;

/// Every frame past [`FULL_REPAINTS`] that the runtime asked for kept its target,
/// so each animating replay went through the damage path the comparison checks.
///
/// The frames after everything settles are not asked for; the loop renders them anyway,
/// on the full-repaint path.
fn assert_every_animating_frame_took_the_damage_path(frames: &[(bool, bool)]) {
    let pattern: String = frames
        .iter()
        .map(|&(asked_for, kept)| match (asked_for, kept) {
            (_, true) => 'P',
            (true, false) => 'C',
            (false, false) => '.',
        })
        .collect();
    let replays = &frames[FULL_REPAINTS..];
    assert!(
        replays.iter().any(|&(asked_for, _)| asked_for),
        "{pattern}: nothing animated past the full repaints, so the damage path went untested"
    );
    assert!(
        replays.iter().all(|&(asked_for, kept)| kept || !asked_for),
        "{pattern}: an animating replay fell back to a full repaint the comparison cannot fault"
    );
}

fn tree(draws: Vec<Draw>) -> Vec<u8> {
    let props = PropsData {
        width: f32::from(W),
        height: f32::from(H),
        ..PropsData::default()
    };
    let mut all = vec![Draw::rect(0.0, 0.0, f32::from(W), f32::from(H), BACKGROUND)];
    all.extend(draws);
    encode(&canvas(props, all))
}

/// Hands pointing apart, rotating about the canvas centre, under a static disc.
fn clock(angles: [f32; 3]) -> Vec<u8> {
    let hands = angles
        .into_iter()
        .zip(["hour", "minute", "second"])
        .map(|(angle, id)| {
            let hand = Draw::rect(CENTRE.0 - 3.0, CENTRE.1 - 100.0, 6.0, 110.0, HAND);
            Draw::rotated(angle, hand).transition(id, ANIMATION_MS, Easing::EaseOut)
        });
    let mut draws: Vec<Draw> = hands.collect();
    draws.push(Draw::circle(CENTRE.0, CENTRE.1, 8.0, BACKGROUND));
    tree(draws)
}

fn gauge(end_angle: f32, easing: Easing) -> Vec<u8> {
    tree(vec![
        Draw::arc(
            CENTRE.0,
            CENTRE.1,
            80.0,
            0.0,
            end_angle,
            12.0,
            GAUGE,
            ArcSegments::Continuous,
            ArcCap::Round,
        )
        .transition("gauge", ANIMATION_MS, easing),
    ])
}

/// A thick butt-capped arc whose radius transitions to `radius`.
fn arc_of_radius(radius: f32) -> Vec<u8> {
    tree(vec![
        Draw::arc(
            CENTRE.0,
            CENTRE.1,
            radius,
            0.0,
            1.2,
            24.0,
            GAUGE,
            ArcSegments::Continuous,
            ArcCap::Butt,
        )
        .transition("ring", ANIMATION_MS, Easing::EaseOutCubic),
    ])
}

/// A short arc about 12 o'clock, shrinking towards the ring's centre once `shrinks`.
fn shrinking_arc(shrinks: bool) -> Vec<u8> {
    let arc = Draw::arc(
        CENTRE.0,
        CENTRE.1,
        80.0,
        -0.3,
        0.3,
        12.0,
        GAUGE,
        ArcSegments::Continuous,
        ArcCap::Round,
    );
    tree(vec![if shrinks {
        arc.animate(
            AnimProperty::Scale,
            1.0,
            0.35,
            ANIMATION_MS,
            Easing::EaseOutElastic,
            LoopMode::Once,
        )
    } else {
        arc
    }])
}

#[test]
fn moving_clock_hands_repaint_like_a_full_frame() {
    assert_damage_tracking_matches_full_repaints(&[
        clock([0.0, 0.4 * PI, 1.2 * PI]),
        clock([0.3, 0.9 * PI, 1.7 * PI]),
    ]);
}

/// Draining is the direction that needs the range back to where it started:
/// a filling gauge stays inside its target's sector.
#[test]
fn a_draining_gauge_repaints_like_a_full_frame() {
    assert_damage_tracking_matches_full_repaints(&[
        gauge(1.4, Easing::EaseOutCubic),
        gauge(0.6, Easing::EaseOutCubic),
    ]);
}

/// The overshoot drives the sweep below zero, which femtovg paints the long way round.
#[test]
fn a_gauge_emptying_past_zero_repaints_like_a_full_frame() {
    assert_damage_tracking_matches_full_repaints(&[
        gauge(0.6, Easing::EaseOutBack),
        gauge(0.0, Easing::EaseOutBack),
    ]);
}

/// A transition moves the radius but keeps the stroke's width,
/// so the frames near the centre paint as thick as the ones at the rim.
#[test]
fn an_arc_growing_its_radius_repaints_like_a_full_frame() {
    assert_damage_tracking_matches_full_repaints(&[arc_of_radius(20.0), arc_of_radius(90.0)]);
}

/// The renderer scales an arc about the ring's centre, which pulls the stroke
/// out of the sector box it was bounded by at full size.
#[test]
fn a_shrinking_arc_repaints_like_a_full_frame() {
    assert_damage_tracking_matches_full_repaints(&[shrinking_arc(false), shrinking_arc(true)]);
}
