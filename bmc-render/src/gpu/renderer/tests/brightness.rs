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

use super::FemtoVgRenderer;
use crate::renderer::Renderer;
use crate::test_harness::{GlHarness, create_readback_fbo, read_pixels_top_down};
use crate::tree::{SpanData, TextStyle};
use bmc_wasm_protocol::colors::{TRANSPARENT, WHITE};
use bmc_wasm_protocol::{Fill, SVG_FLAG_HAS_FILL, SVG_OP_CLOSE, SVG_OP_LINE_TO, SVG_OP_MOVE_TO};

pub(super) const W: u32 = 64;
const H: u32 = 64;
const HALF: f32 = 0.5;

pub(super) fn render(draw: impl FnOnce(&mut FemtoVgRenderer)) -> Vec<[u8; 4]> {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    renderer.begin_frame(W, H, 1.0);
    draw(&mut renderer);
    renderer.flush();
    let pixels = read_pixels_top_down(&harness.gl, fbo, W, H);
    drop(renderer);
    pixels
}

/// Any pixel works: every draw here covers the whole frame.
fn center(px: &[[u8; 4]]) -> [u8; 4] {
    const PROBE: (usize, usize) = (32, 32);
    px[PROBE.1 * W as usize + PROBE.0]
}

/// White at [`HALF`], within a step of rounding.
fn assert_half_lit(px: [u8; 4], what: &str) {
    assert!(
        px[..3].iter().all(|c| (126..=128).contains(c)),
        "{what} paints white at half brightness, got {px:?}"
    );
}

/// A square filling its viewbox, drawn in its own white.
fn white_square_icon() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1.0_f32.to_le_bytes());
    buf.extend_from_slice(&1.0_f32.to_le_bytes());
    buf.extend_from_slice(&1_u16.to_le_bytes());
    buf.push(SVG_FLAG_HAS_FILL);
    buf.extend_from_slice(&WHITE.to_u32().to_le_bytes());
    buf.extend_from_slice(&5_u16.to_le_bytes());
    for (op, x, y) in [
        (SVG_OP_MOVE_TO, 0.0_f32, 0.0_f32),
        (SVG_OP_LINE_TO, 1.0, 0.0),
        (SVG_OP_LINE_TO, 1.0, 1.0),
        (SVG_OP_LINE_TO, 0.0, 1.0),
    ] {
        buf.push(op);
        buf.extend_from_slice(&x.to_le_bytes());
        buf.extend_from_slice(&y.to_le_bytes());
    }
    buf.push(SVG_OP_CLOSE);
    buf
}

pub(super) const FULL: (f32, f32, f32, f32) = (0.0, 0.0, W as f32, H as f32);

#[test]
fn a_solid_fill_dims() {
    let (x, y, w, h) = FULL;
    let px = render(|r| {
        r.set_brightness(HALF);
        r.fill_rect(x, y, w, h, WHITE);
    });
    assert_half_lit(center(&px), "a solid rect");
}

#[test]
fn a_gradient_fill_dims() {
    let (x, y, w, h) = FULL;
    let px = render(|r| {
        r.set_brightness(HALF);
        r.fill_rect_paint(x, y, w, h, &Fill::linear(0.0, WHITE, WHITE));
    });
    assert_half_lit(center(&px), "a gradient rect");
}

/// The case a wrapper in front of the backend could not reach:
/// with no tint, the colour lives in the icon.
#[test]
fn an_svg_dims_its_own_colours() {
    let (x, y, w, h) = FULL;
    let px = render(|r| {
        let icon = r
            .register_svg("square", &white_square_icon())
            .expect("BUG: the square icon must parse");
        r.set_brightness(HALF);
        r.draw_svg(x, y, w, h, TRANSPARENT, icon, false, &[]);
    });
    assert_half_lit(center(&px), "an untinted svg");
}

#[test]
fn a_bitmap_dims() {
    let (x, y, w, h) = FULL;
    let px = render(|r| {
        let white = vec![255; (W * H * 4) as usize];
        let bitmap = r
            .register_bitmap_rgba("white", &white, W, H)
            .expect("BUG: an RGBA bitmap must register");
        r.set_brightness(HALF);
        r.draw_bitmap(x, y, w, h, bitmap);
    });
    assert_half_lit(center(&px), "a bitmap");
}

/// A span's own colour dims like the paragraph's.
#[test]
fn a_span_colour_dims() {
    let brightest = |brightness: f32| {
        let style = TextStyle {
            size: 48,
            color: TRANSPARENT,
            ..TextStyle::default()
        };
        let spans = [SpanData {
            text: "H".to_owned(),
            weight: None,
            color: Some(WHITE),
            size: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }];
        let px = render(|r| {
            r.set_brightness(brightness);
            r.draw_paragraph(&style, &spans, 4.0, 4.0, W as f32);
        });
        px.iter()
            .map(|p| p[0])
            .max()
            .expect("BUG: the frame has pixels")
    };
    assert_eq!(brightest(1.0), 255, "the glyph's core is fully covered");
    assert!(
        (126..=128).contains(&brightest(HALF)),
        "the dimmed glyph peaks at half white"
    );
}

/// A walk that panicked mid-subtree cannot leave the next frame dimmed.
#[test]
fn each_frame_starts_undimmed() {
    let (x, y, w, h) = FULL;
    let px = render(|r| {
        r.set_brightness(HALF);
        r.begin_frame(W, H, 1.0);
        r.fill_rect(x, y, w, h, WHITE);
    });
    assert_eq!(center(&px), [255, 255, 255, 255]);
}
