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

//! Damage against femtovg's own pixels.
//!
//! The damage tests elsewhere check the bound's arithmetic;
//! these draw real icons through the renderer, rotated, stretched
//! and placed off the pixel grid, and require every lit pixel
//! to fall inside the rect `damage` reports for the draw.

#![expect(
    clippy::cast_precision_loss,
    clippy::integer_division,
    reason = "pixel coordinates on a 384 px test canvas"
)]

use std::collections::HashMap;

use super::FemtoVgRenderer;
use crate::damage::{DrawDamage, canvas_draw_damage};
use crate::interaction::Rect;
use crate::renderer::RenderTarget;
use crate::renderer::prelude::*;
use crate::test_harness::{GlHarness, create_readback_fbo, read_pixels_top_down};
use crate::tree::{AnimationContext, AnimationDef, Draw, DynamicHalf, EmitMode};
use bmc_ui::colors::WHITE;
use bmc_ui::{AnimProperty, ColorSpace, Easing, LoopMode};

const W: u32 = 384;
const H: u32 = 384;

/// Placements each fixture is drawn at: box `(x, y, w, h)` off the pixel grid,
/// some stretched, some under a rotation about the canvas centre,
/// and some small enough that femtovg flattens a curve into few chords.
const PLACEMENTS: [((f32, f32, f32, f32), f32); 9] = [
    ((200.3, 60.6, 12.0, 12.0), 0.0),
    ((250.6, 40.3, 20.0, 20.0), 0.0),
    ((300.2, 300.7, 8.0, 8.0), 0.0),
    ((60.3, 80.7, 100.0, 100.0), 0.0),
    ((40.5, 30.25, 200.0, 200.0), 0.0),
    ((30.2, 140.6, 300.0, 90.0), 0.0),
    ((170.4, 170.8, 40.0, 40.0), 0.0),
    ((90.3, 92.7, 200.0, 200.0), 0.4),
    ((60.8, 120.5, 260.0, 130.0), 2.0),
];

/// How a frame draws the placements: at a dpi scale,
/// which shrinks them to the same physical pixels,
/// and under a scale animation held at one factor.
#[derive(Debug, Clone, Copy)]
struct Frame {
    dpi: u32,
    scale: f32,
}

const FRAMES: [Frame; 4] = [
    Frame { dpi: 1, scale: 1.0 },
    Frame { dpi: 2, scale: 1.0 },
    Frame { dpi: 1, scale: 0.6 },
    Frame { dpi: 1, scale: 1.5 },
];

/// A needle: a fill corner far sharper than any miter limit bevels.
const NEEDLE: &str = r#"<path d="M10 48 L90 50 L10 52 Z" fill="white"/>"#;

/// A body whose tip is a quadratic hairpin, a curve doubling back on itself.
const HAIRPIN: &str = r#"<path d="M48 90 L48 40 Q50 -80 52 40 L52 90 Z" fill="white"/>"#;

/// A needle drawn from its tip, so its sharpest join is the one the close makes.
const NEEDLE_FROM_TIP: &str = r#"<path d="M90 50 L10 48 L10 52 Z" fill="white"/>"#;

/// A circle of four cubics, joined smoothly.
const ROUND: &str = r#"<path d="M50 10 C72 10 90 28 90 50 C90 72 72 90 50 90 C28 90 10 72 10 50 C10 28 28 10 50 10 Z" fill="white"/>"#;

/// A stroked loop.
const LOOP: &str =
    r#"<path d="M20 50 C95 5 5 5 80 50" fill="none" stroke="white" stroke-width="6"/>"#;

/// A stroked cusp.
const CUSP: &str =
    r#"<path d="M20 80 C80 20 20 20 80 80" fill="none" stroke="white" stroke-width="6"/>"#;

/// A stroked zigzag, its turns sharp enough to bevel.
const ZIGZAG: &str =
    r#"<path d="M10 20 L90 25 L10 30" fill="none" stroke="white" stroke-width="4"/>"#;

/// A straight stroke, which femtovg never joins.
const STRAIGHT: &str = r#"<path d="M50 10 L50 90" fill="none" stroke="white" stroke-width="4"/>"#;

/// A flat-topped hexagon with a 0.002-unit residue at its top-right vertex,
/// pointing about 71° above the x axis.
const HEXAGON_RESIDUE: &str = r#"<path d="M90 50 L70 84.641 L30 84.641 L10 50 L30 15.359 L70 15.359 L70.00065 15.3571 Z" fill="white"/>"#;

/// A needle with a 0.001-unit residue at its tip, under femtovg's merge distance
/// at every placement: drawn at the pinned rev, merged once femtovg merges repeated points.
const NEEDLE_RESIDUE: &str = r#"<path d="M10 48 L90 50 L90 50.001 L10 52 Z" fill="white"/>"#;

/// A stroked right angle pointing along x, which a box stretched along x sharpens.
const CHEVRON: &str =
    r#"<path d="M30 20 L60 50 L30 80" fill="none" stroke="white" stroke-width="4"/>"#;

/// A wide stroke with a residue below an f32 ulp at its screen position,
/// which femtovg's screen-space rounding can turn to any direction.
const STROKE_RESIDUE: &str = r#"<path d="M10 50 L50 50 L50.0001 49.9999 L63.68 87.59" fill="none" stroke="white" stroke-width="30"/>"#;

/// A fill with a point far outside its viewBox, a wedge across the whole canvas.
const FAR_POINT: &str = r#"<path d="M10 10 L1e9 1e9 L90 10 Z" fill="white"/>"#;

/// A fill whose closing edge folds back onto a short first segment.
const FOLDED_CLOSE: &str = r#"<path d="M50 50 L49.7 50 L90 90 L10 90 Z" fill="white"/>"#;

fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100">{body}</svg>"#
    )
}

/// Draw `body` at `placement` in `frame` through the renderer's own draw path,
/// returning its pixels and the damage reported for the draw, in physical pixels.
fn draw(
    body: &str,
    ((x, y, w, h), angle): ((f32, f32, f32, f32), f32),
    frame: Frame,
) -> (Vec<[u8; 4]>, Rect) {
    let dpi = frame.dpi as f32;
    let (x, y, w, h) = (x / dpi, y / dpi, w / dpi, h / dpi);
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let (fbo, fbo_id) = create_readback_fbo(&harness.gl, W, H);
    let mut renderer = unsafe { FemtoVgRenderer::new(harness.load_fn(), W, H, fbo_id, 0) }
        .expect("BUG: renderer init failed");
    let icon_id = renderer
        .register_svg("fixture", &bmc_svg_compiler::compile_svg(&svg(body)))
        .expect("BUG: the fixture icon registers");
    let icon = Draw::Svg {
        x,
        y,
        w,
        h,
        color: WHITE,
        icon_id: Some(icon_id),
        anti_alias: true,
        fills: Vec::new(),
    };
    let scaled = Draw::Modified {
        animations: vec![AnimationDef {
            property: AnimProperty::Scale,
            from: frame.scale,
            to: frame.scale,
            duration_ms: 1_000,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(icon),
    };
    let command = Draw::Rotated {
        angle,
        inner: Box::new(scaled),
    };
    let canvas = Rect::new(0.0, 0.0, W as f32 / dpi, H as f32 / dpi);
    let damage = match canvas_draw_damage(&command, canvas, &HashMap::new(), 0, &|id| {
        renderer.svg_content(id)
    }) {
        DrawDamage::Region(rect) => rect,
        DrawDamage::Nothing | DrawDamage::WholeCanvas => canvas,
    };

    let (mut animation_states, mut transition_states) = (HashMap::new(), HashMap::new());
    let mut anim_ctx = AnimationContext {
        emit: EmitMode::default(),
        animation_states: &mut animation_states,
        transition_states: &mut transition_states,
        delta_ms: 0,
        frame_counter: 0,
        draw_counter: 0,
        canvas_index: 0,
        draw_in_canvas: 0,
        mesh_slot_counter: 0,
        dynamic_half: DynamicHalf::default(),
        layer_painted: false,
        has_active: false,
        now_unix_secs: 0,
        outlines: crate::tree::Outlines::default(),
    };
    renderer.begin_frame(W / frame.dpi, H / frame.dpi, dpi);
    let mut target = RenderTarget::new(&mut renderer, None);
    crate::components::draw::render_draw_command(
        &mut target,
        &command,
        canvas.x,
        canvas.y,
        canvas.w,
        canvas.h,
        &mut anim_ctx,
    );
    renderer.flush();
    let pixels = read_pixels_top_down(&harness.gl, fbo, W, H);
    drop(renderer);
    let damage = Rect::new(
        damage.x * dpi,
        damage.y * dpi,
        damage.w * dpi,
        damage.h * dpi,
    );
    (pixels, damage)
}

/// Every lit pixel's `(column, row)`: white over the frame's opaque black.
fn lit(pixels: &[[u8; 4]]) -> impl Iterator<Item = (usize, usize)> + '_ {
    pixels
        .iter()
        .enumerate()
        .filter(|(_, p)| p[..3].iter().any(|&c| c > 0))
        .map(|(i, _)| (i % W as usize, i / W as usize))
}

fn assert_contained(name: &str, body: &str) {
    for (frame, placement) in FRAMES
        .into_iter()
        .flat_map(|frame| PLACEMENTS.map(|placement| (frame, placement)))
    {
        assert_contained_at(name, body, placement, frame);
    }
}

fn assert_contained_at(
    name: &str,
    body: &str,
    placement: ((f32, f32, f32, f32), f32),
    frame: Frame,
) {
    let (pixels, damage) = draw(body, placement, frame);
    let outside: Vec<_> = lit(&pixels)
        .filter(|&(x, y)| {
            let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
            x < damage.x || x > damage.x + damage.w || y < damage.y || y > damage.y + damage.h
        })
        .collect();
    assert!(
        outside.is_empty(),
        "{name} at {placement:?} in {frame:?} lights {} px outside {damage:?}, first {:?}",
        outside.len(),
        outside.first()
    );
}

#[test]
fn a_needle_stays_inside_its_damage() {
    assert_contained("needle", NEEDLE);
}

#[test]
fn a_needle_closed_at_its_tip_stays_inside_its_damage() {
    assert_contained("needle from its tip", NEEDLE_FROM_TIP);
}

#[test]
fn a_hairpin_tip_stays_inside_its_damage() {
    assert_contained("hairpin", HAIRPIN);
}

#[test]
fn a_filled_curve_stays_inside_its_damage() {
    assert_contained("round", ROUND);
}

#[test]
fn a_stroked_loop_stays_inside_its_damage() {
    assert_contained("loop", LOOP);
}

#[test]
fn a_stroked_cusp_stays_inside_its_damage() {
    assert_contained("cusp", CUSP);
}

#[test]
fn a_stroked_zigzag_stays_inside_its_damage() {
    assert_contained("zigzag", ZIGZAG);
}

#[test]
fn a_straight_stroke_stays_inside_its_damage() {
    assert_contained("straight", STRAIGHT);
}

#[test]
fn a_stroked_chevron_stays_inside_its_damage() {
    assert_contained("chevron", CHEVRON);
}

#[test]
fn a_hexagon_with_a_residue_stays_inside_its_damage() {
    assert_contained("hexagon", HEXAGON_RESIDUE);
}

#[test]
fn a_needle_with_a_residue_under_the_merge_distance_stays_inside_its_damage() {
    assert_contained("needle with a residue", NEEDLE_RESIDUE);
}

#[test]
fn a_stroke_with_a_residue_under_an_ulp_stays_inside_its_damage() {
    assert_contained("stroke with a residue", STROKE_RESIDUE);
}

/// Rotated, so the far point lands back across the canvas.
#[test]
fn a_fill_reaching_far_outside_its_viewbox_stays_inside_its_damage() {
    assert_contained_at(
        "far point",
        FAR_POINT,
        ((170.3, 20.7, 34.0, 340.0), 2.356),
        FRAMES[0],
    );
}

#[test]
fn a_fill_folding_onto_a_short_start_stays_inside_its_damage() {
    assert_contained("folded", FOLDED_CLOSE);
}

/// A concave spike turning back by all but `α` at its tip, whose edges run
/// long enough that femtovg never bevels the inner join there.
/// `wound_back` flips which side of the tip gets the wider half of the fringe.
fn spike(sin_half_angle: f32, wound_back: bool) -> String {
    let cos_half_angle = (1.0 - sin_half_angle * sin_half_angle).sqrt();
    let (left, right) = (50.0 - 70.0 * sin_half_angle, 50.0 + 70.0 * sin_half_angle);
    let (start, end) = if wound_back {
        (right, left)
    } else {
        (left, right)
    };
    let (base, notch) = (20.0 + 70.0 * cos_half_angle, 10.0 + 70.0 * cos_half_angle);
    format!(r#"<path d="M{start} {base} L50 20 L{end} {base} L50 {notch} Z" fill="white"/>"#)
}

/// femtovg extrudes such a tip `min(1 / sin(α/2), cap · sin(α/2))` fringe widths,
/// so only half-angles under `asin(1 / √cap)` show where its join cap sits.
/// A sweep through them fails once the cap outgrows `FEMTOVG_EXTRUSION_SCALE_CAP`,
/// and its furthest reach, 30.7 px at a cap of 600, pins the cap itself.
#[test]
fn a_spike_at_femtovgs_join_cap_stays_inside_its_damage() {
    const SCALE: f32 = 3.0;
    let placement = ((42.3, 41.7, 100.0 * SCALE, 100.0 * SCALE), 0.0);
    let tip_row = placement.0.1 + 20.0 * SCALE;
    let mut furthest = 0.0_f32;
    for inverse_sine in (20..=80_u16).map(f32::from) {
        for wound_back in [false, true] {
            let (pixels, damage) =
                draw(&spike(1.0 / inverse_sine, wound_back), placement, FRAMES[0]);
            let outside: Vec<_> = lit(&pixels)
                .filter(|&(x, y)| {
                    let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
                    x < damage.x
                        || x > damage.x + damage.w
                        || y < damage.y
                        || y > damage.y + damage.h
                })
                .collect();
            assert!(
                outside.is_empty(),
                "a spike with sin(α/2) = 1/{inverse_sine} lights {} px outside {damage:?}, first {:?}",
                outside.len(),
                outside.first()
            );
            let top = lit(&pixels)
                .map(|(_, y)| y)
                .min()
                .expect("BUG: the spike paints");
            furthest = furthest.max(tip_row - top as f32);
        }
    }
    assert!(
        (28.0..=32.0).contains(&furthest),
        "femtovg's join cap of 600 reaches about 30.7 px past the tip, got {furthest} px"
    );
}
