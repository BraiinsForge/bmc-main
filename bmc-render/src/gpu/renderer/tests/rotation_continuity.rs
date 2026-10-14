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

//! The cutoff must hold under a canvas transform,
//! which the command-level goldens in `gpu::text` cannot see:
//! they compare layout-space positions,
//! while the direct path composes its own skew
//! onto whatever transform the canvas already carries.
//! A wrong composition order, or a skew applied twice,
//! shows up in pixels and nowhere else.

#![expect(clippy::cast_precision_loss)]

use super::multiline_text::lit;
use super::{FemtoVgRenderer, sans_line_style};
use crate::gpu::text::{DIRECT_PATH_CUTOFF_PX, LineStyle, divergent_kerning_pair, snap};
use crate::renderer::prelude::*;
use crate::test_harness::{GlHarness, create_readback_fbo, read_pixels_top_down};
use femtovg::{Paint, Transform2D};

const W: u32 = 320;
const H: u32 = 320;

/// Fractional on purpose, so the snapping rule
/// (x truncated, y rounded) has something to bite on.
const TRANSLATE: (f32, f32) = (20.3, 15.7);
const ROTATE_DEGREES: f32 = 30.0;

/// Layout-space pen of the first glyph,
/// placed so every fixture's rotated ink clears the canvas edges.
const ORIGIN_X: f32 = 173.6;
const BASELINE_Y: f32 = 80.2;

/// A base letter and a combining acute, shaped as two glyphs.
/// The mark carries a shaper offset from its base,
/// and drawing it alone at that offset
/// is what shows whether the offset survives the transform.
/// A precomposable base (`e`, `o`, `n`) would shape to one glyph instead.
const ACCENT_PAIR: &str = "x\u{0301}";

/// Per pixel of nominal font size. The two paths rasterize with different
/// engines — swash into the cache, femtovg's own fill on the direct path —
/// so their ink never matches exactly; only the placement has to.
const TOLERANCE_PX: f32 = 1.5;

/// Mirrors the canvas calls `glyph_offset` makes,
/// so femtovg's premultiply order moves reference and render together.
fn outer_transform() -> Transform2D {
    let mut transform = Transform2D::identity();
    transform.premultiply(&Transform2D::translation(TRANSLATE.0, TRANSLATE.1));
    transform.premultiply(&Transform2D::rotation(ROTATE_DEGREES.to_radians()));
    transform
}

/// Centre of mass of the frame's ink, in pixel centres.
///
/// Over the whole mask, not per component: components do not map one to one
/// onto glyphs, which is why every frame here carries a single glyph.
fn ink_centroid(px: &[[u8; 4]]) -> (f32, f32) {
    let mut sum = (0.0_f64, 0.0_f64);
    let mut count = 0_usize;
    for (y, row) in px.chunks_exact(W as usize).enumerate() {
        for (x, p) in row.iter().enumerate() {
            if lit(*p) {
                sum.0 += x as f64 + 0.5;
                sum.1 += y as f64 + 0.5;
                count += 1;
            }
        }
    }
    assert!(count > 50, "BUG: the glyph rendered {count} lit px");
    #[expect(clippy::cast_possible_truncation, reason = "canvas is 320 px wide")]
    ((sum.0 / count as f64) as f32, (sum.1 / count as f64) as f32)
}

/// No ink may touch the frame edge: a clipped glyph moves its own centroid,
/// which would compare two differently truncated shapes
/// and pass or fail on the clipping rather than on the placement.
fn assert_inside(px: &[[u8; 4]]) {
    let (w, h) = (W as usize, H as usize);
    for (y, row) in px.chunks_exact(w).enumerate() {
        for (x, p) in row.iter().enumerate() {
            let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
            assert!(
                !(edge && lit(*p)),
                "BUG: ink reaches the canvas edge at ({x}, {y})",
            );
        }
    }
}

/// Draw glyph `index` of `text`, alone in a cleared frame, at the position
/// the full string's layout gives it, under the outer transform.
/// Returns its ink centroid's displacement from the transformed pen.
fn glyph_offset(text: &str, index: usize, italic: bool, size: f32) -> (f32, f32) {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");

    let style = LineStyle {
        italic,
        ..sans_line_style(size)
    };
    let (lines, _) = renderer.layout_line(style, text);
    let line = lines.first().expect("BUG: fixture shaped no line");
    let glyph = line
        .glyphs
        .get(index)
        .expect("BUG: fixture shaped fewer glyphs than the test indexes");
    let pen = snap(ORIGIN_X + glyph.x, BASELINE_Y + glyph.y);

    renderer.begin_frame(W, H, 1.0);
    renderer.save();
    renderer.translate(TRANSLATE.0, TRANSLATE.1);
    renderer.rotate(ROTATE_DEGREES.to_radians());
    crate::gpu::text::draw_line_glyphs(
        &mut renderer.canvas,
        &mut renderer.glyph_cache,
        &mut renderer.swash,
        &mut renderer.font_system,
        &renderer.font_table,
        &line.glyphs[index..=index],
        ORIGIN_X,
        BASELINE_Y,
        &Paint::color(femtovg::Color::white()),
        size,
    );
    renderer.restore();
    renderer.flush();

    let px = read_pixels_top_down(&harness.gl, fbo, W, H);
    drop(renderer);
    assert_inside(&px);
    let (cx, cy) = ink_centroid(&px);
    let (px_pen, py_pen) = outer_transform().transform_point(pen.0, pen.1);
    (cx - px_pen, cy - py_pen)
}

#[test]
fn cutoff_is_continuous_under_rotation() {
    let cached = DIRECT_PATH_CUTOFF_PX;
    let direct = DIRECT_PATH_CUTOFF_PX + 1.0;

    let mut probe = super::build_font_system();
    let kerning = divergent_kerning_pair(&mut probe, direct);
    drop(probe);

    let fixtures = [
        (kerning, 0, false),
        (kerning, 1, false),
        (ACCENT_PAIR, 0, false),
        (ACCENT_PAIR, 1, false),
        (kerning, 0, true),
    ];

    for (text, index, italic) in fixtures {
        let below = glyph_offset(text, index, italic, cached);
        let above = glyph_offset(text, index, italic, direct);
        for (axis, b, a) in [("x", below.0, above.0), ("y", below.1, above.1)] {
            let jump = (b / cached - a / direct) * cached;
            assert!(
                jump.abs() <= TOLERANCE_PX,
                "{text:?}[{index}] italic={italic}: {axis} jumps {jump} px \
                 across the cutoff under rotation",
            );
        }
    }
}
