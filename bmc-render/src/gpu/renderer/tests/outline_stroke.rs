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

//! A layout outline against femtovg's own pixels: its ink must stay inside its box,
//! since damage reaches no further and a moving box would leave the rest behind.

#![expect(
    clippy::cast_precision_loss,
    clippy::integer_division,
    reason = "pixel coordinates on a 128 px test canvas"
)]

use super::FemtoVgRenderer;
use crate::interaction::Rect;
use crate::renderer::prelude::*;
use crate::test_harness::{GlHarness, create_readback_fbo, read_pixels_top_down};
use crate::tree::stroke_inside;
use bmc_wasm_protocol::colors::WHITE;

const W: u32 = 128;
const H: u32 = 128;

/// Boxes in logical pixels, on the pixel grid and off it, down to the smallest with room.
const BOXES: [(f32, f32, f32, f32); 4] = [
    (10.0, 12.0, 40.0, 30.0),
    (20.3, 30.6, 25.2, 18.4),
    (5.5, 5.5, 3.0, 2.5),
    (40.7, 8.2, 3.6, 50.0),
];

/// Every pixel the outline of `rect` lights, as `(column, row)` centres in physical pixels.
fn lit_by_outline(rect: Rect, dpi: u32) -> Vec<(f32, f32)> {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    renderer.begin_frame(W / dpi, H / dpi, dpi as f32);
    let ink = stroke_inside(rect).expect("BUG: every fixture box has room for an outline");
    renderer.stroke_rect(ink.x, ink.y, ink.w, ink.h, 1.0, WHITE);
    renderer.flush();
    let pixels = read_pixels_top_down(&harness.gl, fbo, W, H);
    drop(renderer);
    pixels
        .iter()
        .enumerate()
        .filter(|(_, p)| p[..3].iter().any(|&c| c > 0))
        .map(|(i, _)| ((i % W as usize) as f32 + 0.5, (i / W as usize) as f32 + 0.5))
        .collect()
}

#[test]
fn an_outline_stays_inside_its_box() {
    for dpi in [1, 2] {
        for (x, y, w, h) in BOXES {
            let rect = Rect::new(x, y, w, h);
            let lit = lit_by_outline(rect, dpi);
            let scale = dpi as f32;
            let outside: Vec<_> = lit
                .iter()
                .filter(|&&(px, py)| {
                    px < x * scale || px > (x + w) * scale || py < y * scale || py > (y + h) * scale
                })
                .collect();
            assert!(
                !lit.is_empty(),
                "the outline of {rect:?} at dpi {dpi} lights nothing"
            );
            assert!(
                outside.is_empty(),
                "the outline of {rect:?} at dpi {dpi} lights {} px outside it, first {:?}",
                outside.len(),
                outside.first()
            );
        }
    }
}

#[test]
fn a_box_with_no_room_inside_strokes_nothing() {
    assert!(stroke_inside(Rect::new(5.0, 5.0, 2.0, 40.0)).is_none());
}
