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

//! Regressions for hard-separated multiline text. cosmic-text reports each
//! glyph's byte offset relative to its own hard line, so a later line's offsets
//! must be shifted by that line's start before they can index the concatenated
//! span text or the span-style table — otherwise a later line slices a
//! multi-byte char boundary (panic), renders the first line's characters, or
//! inherits an earlier span's style. These exercise the real `FemtoVgRenderer`
//! GL path end to end (shape → draw → read back pixels).

#![expect(clippy::cast_precision_loss)]

use super::FemtoVgRenderer;
use crate::renderer::prelude::*;
use crate::test_harness::{GlHarness, create_readback_fbo, read_pixels_top_down};
use crate::tree::{SpanData, TextStyle};
use bmc_wasm_protocol::{AutoFit, Color};

pub(super) const W: u32 = 240;
pub(super) const H: u32 = 110;
/// Text origin of every render below. The box is inset by this much on all
/// four sides, giving `MAX_W` × `BOX_H`.
const TEXT_INSET: f32 = 2.0;
const MAX_W: f32 = 236.0;
const BOX_H: f32 = 106.0;

pub(super) fn span(text: &str, color: Option<Color>) -> SpanData {
    SpanData {
        text: text.to_owned(),
        weight: None,
        color,
        size: None,
        italic: false,
        underline: false,
        strikethrough: false,
    }
}

fn white_style() -> TextStyle {
    TextStyle {
        size: 28,
        color: Color::from_rgb(255, 255, 255),
        ..TextStyle::default()
    }
}

/// Row bands that isolate each line's ink: rows `0..line1_max` carry only
/// the first line, `line2_min..H` only the second.
///
/// Derived from the style the tests actually draw with rather than
/// hand-computed, so a change to `TextStyle::default().line_height` moves
/// the bands instead of leaving every assertion below silently measuring
/// the wrong rows. cosmic-text centres the glyph box inside the line box,
/// so the blank gap between the two lines' ink is exactly the leading —
/// half above the line boundary and half below.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn line_bands() -> (usize, usize) {
    let style = white_style();
    let font = style.size as f32;
    let line_height = font * style.line_height;
    let half_leading = (line_height - font) / 2.0;
    (
        (TEXT_INSET + half_leading + font) as usize,
        (TEXT_INSET + line_height + half_leading) as usize,
    )
}

/// Run `draw` against a fresh headless renderer targeting an offscreen FBO
/// and return the rendered pixels (row 0 = top). Each call is a full,
/// independent GL context — deterministic under Mesa llvmpipe.
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

/// `drop_shadow` hands the canvas back to `frame_target`, so a capture that
/// does not own that field loses the shadowed static draw and everything
/// queued after it to the screen, and the blit then wipes them.
#[test]
fn a_shadowed_static_draw_stays_inside_the_layer() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");

    renderer.begin_frame(W, H, 1.0);
    assert!(
        renderer.begin_static_layer("shadow-test", W, H),
        "BUG: the layer must be allocatable on the test driver"
    );
    renderer.drop_shadow(
        0.0,
        0.0,
        W,
        H,
        0.0,
        0.0,
        4.0,
        Color::from_rgb(0, 0, 0),
        &mut |r: &mut dyn Renderer| {
            r.fill_rect(8.0, 8.0, 32.0, 32.0, Color::from_rgb(0, 255, 0));
        },
    );
    renderer.fill_rect(8.0, 8.0, 32.0, 32.0, Color::from_rgb(0, 255, 0));
    renderer.end_static_layer("shadow-test");
    renderer.flush();

    let screen = read_pixels_top_down(&harness.gl, fbo, W, H);
    assert!(
        screen.iter().all(|px| px[1] < 40),
        "the capture leaked onto the screen instead of staying in the layer"
    );
}

/// The whole point of `dpi_scale`: the caller keeps drawing in logical
/// pixels while the target holds `×dpi` of them. femtovg gives no such
/// scaling on its own — `set_size` spends `dpi` on tessellation tolerance —
/// so a frame that forgets the transform draws at 1× into the corner of its
/// target, which is what a widget preview zoomed past 1:1 used to show.
#[test]
fn a_dpi_scaled_frame_still_draws_in_logical_coordinates() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (_fbo, fbo_id) = create_readback_fbo(&harness.gl, W * 2, H * 2);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");

    renderer.begin_frame(W, H, 2.0);

    let transform = renderer.canvas.transform().0;
    assert!(
        (transform[0] - 2.0).abs() < f32::EPSILON && (transform[3] - 2.0).abs() < f32::EPSILON,
        "logical coordinates must be premultiplied onto the device ratio, got {transform:?}",
    );
}

/// Render `draw` at twice the device scale into a target of that size, and
/// return the physical pixels. Without the scale the drawing covers a
/// quarter of the target and the far corner stays black.
fn render_at_2x(draw: impl FnOnce(&mut FemtoVgRenderer)) -> Vec<[u8; 4]> {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, W * 2, H * 2);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    renderer.begin_frame(W, H, 2.0);
    draw(&mut renderer);
    renderer.flush();
    let pixels = read_pixels_top_down(&harness.gl, fbo, W * 2, H * 2);
    drop(renderer);
    pixels
}

/// The pixel one in from the far corner, where a frame drawn at 1× into a
/// 2× target has nothing.
fn far_corner(pixels: &[[u8; 4]]) -> [u8; 4] {
    let (w, h) = ((W * 2) as usize, (H * 2) as usize);
    pixels[(h - 2) * w + (w - 2)]
}

#[test]
fn a_dpi_scaled_frame_fills_its_whole_target() {
    let pixels = render_at_2x(|renderer| {
        renderer.fill_rect(0.0, 0.0, W as f32, H as f32, Color::from_rgb(255, 255, 255));
    });

    assert!(
        lit(far_corner(&pixels)),
        "a logical-size fill must reach the far corner of a 2× target",
    );
}

#[test]
fn a_dpi_scaled_drop_shadow_fills_its_whole_target() {
    // The shadow's offscreen pass resets the canvas, which drops the scale
    // unless it is put back — and the reset is inside `drop_shadow`, so no
    // caller can compensate for it.
    let pixels = render_at_2x(|renderer| {
        renderer.drop_shadow(
            0.0,
            0.0,
            W,
            H,
            0.0,
            0.0,
            0.0,
            Color::from_rgb(0, 0, 0),
            &mut |inner: &mut dyn Renderer| {
                inner.fill_rect(0.0, 0.0, W as f32, H as f32, Color::from_rgb(255, 255, 255));
            },
        );
    });

    assert!(
        lit(far_corner(&pixels)),
        "a shadowed draw must reach the far corner of a 2× target too",
    );
}

pub(super) fn lit(px: [u8; 4]) -> bool {
    u16::from(px[0]) + u16::from(px[1]) + u16::from(px[2]) > 96
}

fn is_reddish(px: [u8; 4]) -> bool {
    px[0] > 150 && px[1] < 100 && px[2] < 100
}

fn is_greenish(px: [u8; 4]) -> bool {
    px[1] > 140 && px[0] < 120 && px[2] < 120
}

/// Count pixels matching `pred` within rows `[y0, y1)`.
fn count_rows(px: &[[u8; 4]], y0: usize, y1: usize, pred: impl Fn([u8; 4]) -> bool) -> usize {
    let w = W as usize;
    px[y0 * w..y1 * w].iter().filter(|p| pred(**p)).count()
}

/// Both text rows must carry real glyph coverage. Guards the pixel-equality
/// and colour-absence assertions below, which all hold trivially if a line
/// renders nothing at all. Measured coverage is 468–1019 lit px per line, so
/// this floor sits ~5× below the real values.
fn assert_both_lines_rendered(px: &[[u8; 4]]) {
    const MIN_LIT: usize = 100;
    let (line1_max, line2_min) = line_bands();
    let first = count_rows(px, 0, line1_max, lit);
    let second = count_rows(px, line2_min, H as usize, lit);
    assert!(
        first > MIN_LIT,
        "BUG: first line rendered nothing ({first} lit px)",
    );
    assert!(
        second > MIN_LIT,
        "BUG: second line rendered nothing ({second} lit px)",
    );
}

/// Count pixels that differ between two renders within rows `[y0, y1)`.
fn diff_rows(a: &[[u8; 4]], b: &[[u8; 4]], y0: usize, y1: usize) -> usize {
    let w = W as usize;
    a[y0 * w..y1 * w]
        .iter()
        .zip(&b[y0 * w..y1 * w])
        .filter(|(pa, pb)| pa != pb)
        .count()
}

/// A multi-byte grapheme near the start, followed by a hard break, once made
/// a later line's glyph offsets slice the concatenated span text on a
/// non-char boundary — a panic that tears down the widget slot. Both the
/// paragraph and autofit paths must survive it.
#[test]
fn multibyte_hard_break_does_not_panic() {
    let style = white_style();
    let spans = [span("“Fact”\nSecond line", None)];
    let px = render(|r| r.draw_paragraph(&style, &spans, TEXT_INSET, TEXT_INSET, MAX_W));
    // Per line, not per frame: a whole-frame check passes when only the first
    // line survives, which is the very regression this guards.
    assert_both_lines_rendered(&px);

    // Autofit funnels through the same paragraph draw; must not panic either.
    let _ = render(|r| {
        r.draw_autofit_text(
            TEXT_INSET,
            TEXT_INSET,
            MAX_W,
            BOX_H,
            "“Fact”\nSecond line",
            &style,
            AutoFit::Shrink,
            8,
            28,
        );
    });
}

/// The second line must render its own characters, not the bytes sitting at
/// the same offsets in the first line. Two paragraphs share an identical
/// second line but differ on the first; with the bug the second line copies
/// the first line's glyphs, so the bottom region diverges.
#[test]
fn later_line_renders_its_own_characters() {
    let style = white_style();
    let first = [span("AAAA\nZZZZ", None)];
    let second = [span("MMMM\nZZZZ", None)];
    let a = render(|r| r.draw_paragraph(&style, &first, TEXT_INSET, TEXT_INSET, MAX_W));
    let b = render(|r| r.draw_paragraph(&style, &second, TEXT_INSET, TEXT_INSET, MAX_W));

    // Sanity: both lines actually rendered, and the first lines really do
    // differ. Without this the equality check below passes vacuously if the
    // second line disappears from both renders.
    assert_both_lines_rendered(&a);
    assert_both_lines_rendered(&b);
    let (line1_max, line2_min) = line_bands();
    assert!(
        diff_rows(&a, &b, 0, line1_max) > 20,
        "BUG: test setup — first lines should differ",
    );
    // The shared second line must be pixel-identical (small AA slack).
    let diffs = diff_rows(&a, &b, line2_min, H as usize);
    assert!(
        diffs < 20,
        "BUG: second line depends on the first line's text ({diffs} px differ)",
    );
}

/// Span styling must follow the text across a hard break. First span is red
/// and ends with the newline; the second span (green) owns the second line.
/// With the bug the second line inherits the first span's red.
#[test]
fn span_style_survives_hard_break() {
    let style = white_style();
    let red = Color::from_rgb(255, 40, 40);
    let green = Color::from_rgb(40, 220, 40);
    let spans = [span("Red\n", Some(red)), span("Green", Some(green))];
    let px = render(|r| r.draw_paragraph(&style, &spans, TEXT_INSET, TEXT_INSET, MAX_W));

    let (line1_max, line2_min) = line_bands();
    assert!(
        count_rows(&px, 0, line1_max, is_reddish) > 10,
        "BUG: first line should be red",
    );
    assert!(
        count_rows(&px, line2_min, H as usize, is_greenish) > 10,
        "BUG: second line should render its own (green) span",
    );
    let leaked = count_rows(&px, line2_min, H as usize, is_reddish);
    assert_eq!(
        leaked, 0,
        "BUG: second line leaked the first span's red color ({leaked} px)",
    );
}
