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

use super::*;
use crate::PrevDrawValues;
use crate::tree::{HostAnimationDef, HostTransitionDef};
use bmc_wasm_protocol::{
    ArcCap, ArcFill, ArcSegments, AutoFit, ColorSpace, Fill, LoopMode, TextStyle, colors::Color,
};

type Transitions = HashMap<TransitionStateKey, TransitionState>;

fn canvas() -> Rect {
    Rect::new(0.0, 0.0, 1280.0, 480.0)
}

fn square(x: f32, y: f32, side: f32) -> DrawCommand {
    DrawCommand::Rect {
        x,
        y,
        w: side,
        h: side,
        fill: Fill::Solid(Color::from_rgb(1, 2, 3)),
    }
}

fn no_icons(_: SvgId) -> Option<SvgContent> {
    None
}

fn damage(draw: &DrawCommand) -> Option<Rect> {
    canvas_draw_damage(draw, canvas(), &HashMap::new(), 0, &no_icons).region()
}

#[test]
fn a_still_draw_damages_its_box_plus_the_aa_fringe() {
    let rect = damage(&square(10.0, 20.0, 30.0)).expect("BUG: a plain rect is bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (9.0, 19.0, 32.0, 32.0),
        "no transform, no expansion"
    );
}

#[test]
fn a_quarter_turn_sweeps_the_corner_arc_not_the_whole_disc() {
    // A corner square rotating a quarter turn about the canvas centre: the
    // disc would cover every side, a quarter turn only two.
    let draw = DrawCommand::Rotated {
        angle: FRAC_PI_2,
        inner: Box::new(square(0.0, 0.0, 100.0)),
    };
    let square_canvas = Rect::new(0.0, 0.0, 400.0, 400.0);
    let rect = canvas_draw_damage(&draw, square_canvas, &HashMap::new(), 0, &no_icons)
        .region()
        .expect("BUG: a rotation of a rect is bounded");
    assert!(rect.x >= 190.0, "left edge stays right of centre: {rect:?}");
    assert!(rect.y <= 10.0, "reaches the top edge: {rect:?}");
    assert!(
        rect.w * rect.h < square_canvas.w * square_canvas.h * 0.4,
        "a quarter turn must cost far less than the disc: {rect:?}"
    );
}

#[test]
fn a_full_turn_covers_the_disc_through_the_pivot() {
    let mut draw = square(0.0, 0.0, 100.0);
    draw = DrawCommand::Modified {
        animations: vec![HostAnimationDef {
            property: AnimProperty::Rotate,
            from: 0.0,
            to: TAU,
            duration_ms: 1_000,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(draw),
    };
    let square_canvas = Rect::new(0.0, 0.0, 400.0, 400.0);
    let rect = canvas_draw_damage(&draw, square_canvas, &HashMap::new(), 0, &no_icons)
        .region()
        .expect("BUG: a rotate animation is bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (0.0, 0.0, 400.0, 400.0),
        "a full turn reaches every corner"
    );
}

#[test]
fn an_outer_translate_moves_a_centered_draw() {
    let centred = DrawCommand::Centered {
        inner: Box::new(square(0.0, 0.0, 30.0)),
    };
    let still = damage(&centred).expect("BUG: a centred square is bounded");

    let animated = DrawCommand::Modified {
        animations: vec![HostAnimationDef {
            property: AnimProperty::TranslateY,
            from: 20.0,
            to: 40.0,
            duration_ms: 1_000,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(centred),
    };
    let moved = damage(&animated).expect("BUG: the animated wrapper is bounded");

    assert_eq!(
        (moved.x, moved.y, moved.w, moved.h),
        (still.x, still.y + 20.0, still.w, still.h + 20.0),
        "the bound covers the centred square at both ends of the translate"
    );
}

#[test]
fn a_nested_centered_replaces_its_parents_placement() {
    let centred = || DrawCommand::Centered {
        inner: Box::new(square(0.0, 0.0, 30.0)),
    };
    let orbited = DrawCommand::Orbit {
        radius: 50.0,
        angle: 0.0,
        inner: Box::new(centred()),
    };

    let alone = damage(&centred()).expect("BUG: a centred square is bounded");
    let nested = damage(&orbited).expect("BUG: an orbited centred square is bounded");

    assert_eq!(
        (nested.x, nested.y, nested.w, nested.h),
        (alone.x, alone.y, alone.w, alone.h),
        "the inner Centered positions from the canvas, not from the orbit"
    );
}

#[test]
fn a_translate_animation_covers_both_ends() {
    let draw = DrawCommand::Modified {
        animations: vec![HostAnimationDef {
            property: AnimProperty::TranslateX,
            from: -50.0,
            to: 100.0,
            duration_ms: 1_000,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(square(200.0, 20.0, 30.0)),
    };
    let rect = damage(&draw).expect("BUG: a translate animation is bounded");
    assert_eq!(
        (rect.x, rect.w),
        (149.0, 182.0),
        "from the leftmost reach to the rightmost"
    );
}

fn scaling(from: f32, to: f32) -> HostAnimationDef {
    HostAnimationDef {
        property: AnimProperty::Scale,
        from,
        to,
        duration_ms: 1_000,
        delay_ms: 0,
        easing: Easing::Linear,
        loop_mode: LoopMode::Forever,
    }
}

#[test]
fn a_scale_animation_covers_its_largest_reach() {
    let draw = DrawCommand::Modified {
        animations: vec![scaling(1.0, 2.0)],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(square(100.0, 100.0, 20.0)),
    };
    let rect = damage(&draw).expect("BUG: a scale animation is bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (89.0, 89.0, 42.0, 42.0),
        "2x about the centre of a 20x20 at (100, 100)"
    );
}

/// Bounding by the larger alone strands the pixels between 2x and 4x, where
/// the previous frame then stays on screen.
#[test]
fn two_scale_animations_compose_rather_than_taking_the_larger() {
    let draw = DrawCommand::Modified {
        animations: vec![scaling(1.0, 2.0), scaling(1.0, 2.0)],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(square(100.0, 100.0, 20.0)),
    };
    let rect = damage(&draw).expect("BUG: scale animations are bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (69.0, 69.0, 82.0, 82.0),
        "4x about the centre, not the 2x either one reaches alone"
    );
}

/// A factor under 1 keeps the draw inside its own box, so it must not pull
/// the bound in below what the enlarging factor beside it reaches.
#[test]
fn a_shrinking_scale_does_not_narrow_the_bound() {
    let draw = DrawCommand::Modified {
        animations: vec![scaling(1.0, 2.0), scaling(1.0, 0.5)],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(square(100.0, 100.0, 20.0)),
    };
    let rect = damage(&draw).expect("BUG: scale animations are bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (89.0, 89.0, 42.0, 42.0),
        "the 2x reach survives the shrink beside it"
    );
}

#[test]
fn a_mirroring_scale_covers_its_reflected_extent() {
    let draw = DrawCommand::Modified {
        animations: vec![scaling(-2.0, 1.0)],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(square(100.0, 100.0, 20.0)),
    };
    let rect = damage(&draw).expect("BUG: scale animations are bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (89.0, 89.0, 42.0, 42.0),
        "-2x mirrors across the same extent 2x reaches"
    );
}

/// An elastic scale passes its upper endpoint by 37%, and the eased value
/// is what the renderer multiplies the box by. Bounding at the endpoint
/// leaves the overshoot ring unrepainted on damage-tracked frames.
#[test]
fn an_overshooting_easing_widens_the_bound_past_its_endpoints() {
    let mut anim = scaling(1.0, 2.0);
    anim.easing = Easing::EaseOutElastic;
    let draw = DrawCommand::Modified {
        animations: vec![anim],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(square(100.0, 100.0, 20.0)),
    };
    let rect = damage(&draw).expect("BUG: scale animations are bounded");

    // The eased value is the *scale*, not the box:
    // 1 → 2 at 1.374 of the way reaches 2.374x.
    // Bounding at the 2x endpoint gives 40px, which this must reject.
    let (_, eased) = crate::animation::easing_extremes(Easing::EaseOutElastic);
    let reached = 20.0 * (1.0 + eased);
    assert!(
        rect.w >= reached && rect.h >= reached,
        "a {}x scale on a 20px box needs {reached}px of damage, got {rect:?}",
        1.0 + eased
    );
    assert!(
        rect.x <= 110.0 - reached / 2.0 && rect.y <= 110.0 - reached / 2.0,
        "the widened box must stay centred on the draw, got {rect:?}"
    );
}

/// The renderer keeps the easing selected at the last retarget, so damage
/// must follow stored state even if the tree's current definition changes.
#[test]
fn damage_uses_the_active_transition_easing_after_the_definition_changes() {
    let inner = square(200.0, 20.0, 30.0);
    let mut recorded = extract_draw_values(&inner);
    recorded.x -= 40.0;
    let mut transitions = HashMap::new();
    transitions.insert(
        (0_u16, 7_u32),
        TransitionState {
            from: recorded,
            target: extract_draw_values(&inner),
            duration_ms: 500,
            easing: Easing::EaseOutBack,
            elapsed_ms: 0,
            last_seen_frame: 0,
        },
    );
    let draw = DrawCommand::Modified {
        animations: Vec::new(),
        transition: Some(HostTransitionDef {
            id_hash: 7,
            duration_ms: 500,
            easing: Easing::Linear,
        }),
        color_space: ColorSpace::default(),
        inner: Box::new(inner),
    };
    let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0, &no_icons)
        .region()
        .expect("BUG: a transition with recorded state is bounded");

    // Delta is -40 px, so the overshoot lands 40 × 0.101 past x=200.
    let (_, eased) = crate::animation::easing_extremes(Easing::EaseOutBack);
    let overshoot = 40.0 * (eased - 1.0);
    assert!(
        rect.x + rect.w >= 230.0 + overshoot,
        "the bound must reach {overshoot}px past the target's right edge, got {rect:?}"
    );
    assert!(
        rect.x <= 160.0,
        "and still cover where it came from, got {rect:?}"
    );
}

/// `render_draw_inner`'s `AutofitText` arm scales its box off `(x, y)`
/// rather than about the centre every other leaf uses, so a centred bound
/// stops half the growth short and strands the right and bottom edges.
#[test]
fn a_scaled_autofit_box_is_bounded_from_its_own_corner() {
    let inner = DrawCommand::AutofitText {
        x: 100.0,
        y: 100.0,
        box_width: 100.0,
        box_height: 20.0,
        mode: AutoFit::Shrink,
        min_size: 8,
        max_size: 40,
        text: "wide".to_owned(),
        style: TextStyle::default(),
    };
    let draw = DrawCommand::Modified {
        animations: vec![scaling(1.0, 2.0)],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(inner),
    };
    let rect = damage(&draw).expect("BUG: an autofit box is bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (99.0, 99.0, 202.0, 42.0),
        "2x about the corner of a 100x20 box at (100, 100)"
    );
}

/// The renderer scales the transition-interpolated stroke too — `ew =
/// width * scale` — so the margin standing for that stroke has to scale
/// with it or the thick end of the ring falls outside the repaint.
#[test]
fn a_scaled_arc_transition_scales_its_stroke_margin_too() {
    let inner = DrawCommand::Arc {
        cx: 200.0,
        cy: 200.0,
        radius: 50.0,
        start_angle: 0.0,
        end_angle: TAU,
        width: 10.0,
        fill: ArcFill::Solid(Color::from_rgb(255, 255, 255)),
        segments: ArcSegments::Continuous,
        cap: ArcCap::Butt,
    };
    let mut recorded = extract_draw_values(&inner);
    recorded.arc_width = 40.0;
    let mut transitions = HashMap::new();
    transitions.insert(
        (0_u16, 9_u32),
        TransitionState {
            from: recorded,
            target: extract_draw_values(&inner),
            duration_ms: 500,
            easing: Easing::Linear,
            elapsed_ms: 0,
            last_seen_frame: 0,
        },
    );
    let draw = DrawCommand::Modified {
        animations: vec![scaling(1.0, 2.0)],
        transition: Some(HostTransitionDef {
            id_hash: 9,
            duration_ms: 500,
            easing: Easing::Linear,
        }),
        color_space: ColorSpace::default(),
        inner: Box::new(inner),
    };
    let square_canvas = Rect::new(0.0, 0.0, 640.0, 640.0);
    let rect = canvas_draw_damage(&draw, square_canvas, &transitions, 0, &no_icons)
        .region()
        .expect("BUG: an arc transition is bounded");

    // Half-extent: (radius + recorded stroke / 2) x 2 = (50 + 20) x 2.
    assert!(
        rect.x <= 60.0 && rect.x + rect.w >= 340.0,
        "the 2x ring at its thickest reaches 140px from the centre, got {rect:?}"
    );
}

#[test]
fn a_transition_covers_the_span_between_its_recorded_ends() {
    let inner = square(200.0, 20.0, 30.0);
    let mut recorded = extract_draw_values(&inner);
    recorded.x -= 40.0;
    let mut transitions = HashMap::new();
    transitions.insert(
        (0_u16, 7_u32),
        TransitionState {
            from: recorded,
            target: extract_draw_values(&inner),
            duration_ms: 500,
            easing: Easing::Linear,
            elapsed_ms: 0,
            last_seen_frame: 0,
        },
    );
    let draw = DrawCommand::Modified {
        animations: Vec::new(),
        transition: Some(HostTransitionDef {
            id_hash: 7,
            duration_ms: 500,
            easing: Easing::Linear,
        }),
        color_space: ColorSpace::default(),
        inner: Box::new(inner),
    };
    let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0, &no_icons)
        .region()
        .expect("BUG: a transition with recorded state is bounded");
    assert_eq!(
        (rect.x, rect.w),
        (159.0, 72.0),
        "covers where it came from as well as where it is going"
    );
}

/// Wrap `inner` in a transition whose recorded start is `from`.
fn transitioning(inner: DrawCommand, from: PrevDrawValues) -> (DrawCommand, Transitions) {
    let mut transitions = HashMap::new();
    transitions.insert(
        (0_u16, 7_u32),
        TransitionState {
            from,
            target: extract_draw_values(&inner),
            duration_ms: 500,
            easing: Easing::Linear,
            elapsed_ms: 0,
            last_seen_frame: 0,
        },
    );
    let draw = DrawCommand::Modified {
        animations: Vec::new(),
        transition: Some(HostTransitionDef {
            id_hash: 7,
            duration_ms: 500,
            easing: Easing::Linear,
        }),
        color_space: ColorSpace::default(),
        inner: Box::new(inner),
    };
    (draw, transitions)
}

#[test]
fn a_thinning_arc_stroke_keeps_the_reach_of_its_thicker_end() {
    let inner = DrawCommand::Arc {
        cx: 200.0,
        cy: 200.0,
        radius: 50.0,
        start_angle: 0.0,
        end_angle: 1.0,
        width: 4.0,
        fill: ArcFill::Solid(Color::from_rgb(1, 2, 3)),
        segments: ArcSegments::Continuous,
        cap: ArcCap::Round,
    };
    let mut from = extract_draw_values(&inner);
    from.arc_width = 24.0;
    let (draw, transitions) = transitioning(inner, from);
    let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0, &no_icons)
        .region()
        .expect("BUG: an arc transition is bounded");
    // The sector starts at 12 o'clock, so its left edge is the centre line
    // less the 12px half of the 24px stroke and the fringe.
    assert_eq!(
        (rect.x, rect.w),
        (187.0, 69.0),
        "the widest stroke in the span sets the reach"
    );
}

/// The 0..0.5 rad sector of a radius-50 ring about (200, 200), scaled `from..to`.
fn scaled_sector(from: f32, to: f32) -> DrawCommand {
    DrawCommand::Modified {
        animations: vec![scaling(from, to)],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(DrawCommand::Arc {
            cx: 200.0,
            cy: 200.0,
            radius: 50.0,
            start_angle: 0.0,
            end_angle: 0.5,
            width: 4.0,
            fill: ArcFill::Solid(Color::from_rgb(1, 2, 3)),
            segments: ArcSegments::Continuous,
            cap: ArcCap::Butt,
        }),
    }
}

/// The renderer shrinks an arc's radius about the ring's centre,
/// moving a slice of it out of a sector box scaled about its own centre.
#[test]
fn a_shrinking_arc_scales_its_sector_about_the_ring_centre() {
    let rect = damage(&scaled_sector(0.5, 1.0)).expect("BUG: a scaled arc is bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (197.0, 147.0, 30.0, 34.0),
        "the sector at half and at full size, not the 106 px wide ring"
    );
}

/// A scale through zero flips the arc across the ring's centre.
#[test]
fn a_flipping_arc_mirrors_its_sector_about_the_ring_centre() {
    let rect = damage(&scaled_sector(-1.0, 1.0)).expect("BUG: a scaled arc is bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (173.0, 147.0, 54.0, 106.0),
        "the sector mirrored about (200, 200)"
    );
}

/// The renderer moves the radius alone, so the stroke keeps its width all the way in.
#[test]
fn an_arc_collapsing_onto_its_centre_damages_every_radius_it_passes() {
    let inner = DrawCommand::Arc {
        cx: 200.0,
        cy: 200.0,
        radius: 0.0,
        start_angle: 0.0,
        end_angle: 1.0,
        width: 4.0,
        fill: ArcFill::Solid(Color::from_rgb(1, 2, 3)),
        segments: ArcSegments::Continuous,
        cap: ArcCap::Round,
    };
    let mut from = extract_draw_values(&inner);
    from.w = 80.0;
    let (draw, transitions) = transitioning(inner, from);
    let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0, &no_icons)
        .region()
        .expect("BUG: an arc transition is bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (197.0, 117.0, 74.0, 86.0),
        "the 0..1 rad sector at radius 80 down to the centre, padded by the 2 px half stroke"
    );
}

#[test]
fn a_sphere_transition_falls_back_to_the_whole_canvas() {
    let inner = DrawCommand::Sphere {
        x: 10.0,
        y: 10.0,
        w: 100.0,
        h: 100.0,
        bitmap_id: None,
        atmosphere: false,
        center_lat: 0.0,
        center_lon: 0.0,
        zoom: 3.0,
        light_lat: 0.0,
        light_lon: 0.0,
    };
    let from = extract_draw_values(&inner);
    let (draw, transitions) = transitioning(inner, from);
    assert!(
        matches!(
            canvas_draw_damage(&draw, canvas(), &transitions, 0, &no_icons),
            DrawDamage::WholeCanvas
        ),
        "a 3D scale and position project outside the declared box"
    );
}

#[test]
fn an_unrecorded_transition_falls_back_to_the_whole_canvas() {
    let draw = DrawCommand::Modified {
        animations: Vec::new(),
        transition: Some(HostTransitionDef {
            id_hash: 7,
            duration_ms: 500,
            easing: Easing::Linear,
        }),
        color_space: ColorSpace::default(),
        inner: Box::new(square(200.0, 20.0, 30.0)),
    };
    assert!(
        damage(&draw).is_none(),
        "with no recorded ends the span is unknown"
    );
}

#[test]
fn a_shadow_and_unmeasured_text_fall_back_to_the_whole_canvas() {
    assert!(
        damage(&DrawCommand::Shadow {
            dx: 0.0,
            dy: 0.0,
            blur: 6.0,
            color: Color::from_rgb(0, 0, 0),
            inner: Box::new(square(10.0, 10.0, 20.0)),
        })
        .is_none(),
        "a shadow composites a canvas-sized layer"
    );
    assert!(
        damage(&DrawCommand::Text {
            x: 0.0,
            y: 0.0,
            text: "x".to_owned(),
            style: TextStyle::default(),
        })
        .is_none(),
        "text extents need shaping"
    );
}

#[test]
fn damage_is_clipped_to_the_canvas() {
    let rect = damage(&square(-100.0, -100.0, 150.0)).expect("BUG: a plain rect is bounded");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (0.0, 0.0, 51.0, 51.0),
        "the walk scissors each canvas to its own bounds"
    );
}

#[test]
fn a_draw_entirely_outside_the_canvas_damages_nothing() {
    assert!(matches!(
        canvas_draw_damage(
            &square(-100.0, -100.0, 10.0),
            canvas(),
            &HashMap::new(),
            0,
            &no_icons
        ),
        DrawDamage::Nothing
    ));
}

#[test]
fn a_clock_hand_costs_a_quarter_of_a_deck_screen() {
    // The shape the module exists for: a clock's second hand, easing 6° a
    // second about its own centre.
    let side = 338.0;
    let pivot_offset = side / 2.0;
    let inner = square(
        1280.0 / 2.0 - pivot_offset,
        480.0 / 2.0 - pivot_offset,
        side,
    );
    let rotated = DrawCommand::Rotated {
        angle: 0.0,
        inner: Box::new(inner),
    };
    let mut recorded = extract_draw_values(&rotated);
    recorded.rotation = -6.0_f32.to_radians();
    let mut transitions = HashMap::new();
    transitions.insert(
        (0_u16, 1_u32),
        TransitionState {
            from: recorded,
            target: extract_draw_values(&rotated),
            duration_ms: 200,
            easing: Easing::Linear,
            elapsed_ms: 0,
            last_seen_frame: 0,
        },
    );
    let draw = DrawCommand::Modified {
        animations: Vec::new(),
        transition: Some(HostTransitionDef {
            id_hash: 1,
            duration_ms: 200,
            easing: Easing::Linear,
        }),
        color_space: ColorSpace::default(),
        inner: Box::new(rotated),
    };
    let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0, &no_icons)
        .region()
        .expect("BUG: a hand is bounded");
    let share = rect.w * rect.h / (1280.0 * 480.0);
    assert!(
        (0.20..0.30).contains(&share),
        "a hand should damage roughly a quarter of the screen, got {share} from {rect:?}"
    );
}

fn icon(x: f32, y: f32, w: f32, h: f32) -> DrawCommand {
    DrawCommand::Svg {
        x,
        y,
        w,
        h,
        color: Color::from_rgb(1, 2, 3),
        icon_id: Some(SvgId::from_wire(1).expect("BUG: fixture SVG ID must be non-zero")),
        anti_alias: true,
        fills: Vec::new(),
    }
}

/// Content of an icon with a 100×100 viewBox that fills and never strokes.
fn filled(bounds: Rect) -> SvgContent {
    SvgContent {
        bounds,
        viewbox_w: 100.0,
        viewbox_h: 100.0,
        stroke: None,
        fills: true,
    }
}

/// The top-right tenth of the viewBox on each axis.
const TOP_RIGHT_SLIVER: Rect = Rect {
    x: 90.0,
    y: 0.0,
    w: 10.0,
    h: 10.0,
};

fn icon_damage(draw: &DrawCommand, content: SvgContent) -> Rect {
    canvas_draw_damage(draw, canvas(), &HashMap::new(), 0, &|_| Some(content))
        .region()
        .expect("BUG: an icon is bounded")
}

fn scaled_sliver(from: f32, to: f32) -> Rect {
    let draw = DrawCommand::Modified {
        animations: vec![scaling(from, to)],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(icon(100.0, 100.0, 100.0, 100.0)),
    };
    icon_damage(&draw, filled(TOP_RIGHT_SLIVER))
}

fn covers(rect: Rect, x: (f32, f32), y: (f32, f32)) -> bool {
    rect.x <= x.0 && rect.x + rect.w >= x.1 && rect.y <= y.0 && rect.y + rect.h >= y.1
}

#[test]
fn a_still_icon_damages_only_what_its_paths_paint() {
    let rect = icon_damage(&icon(100.0, 100.0, 100.0, 100.0), filled(TOP_RIGHT_SLIVER));
    // The sliver covers x 190..200, y 100..110, widened by a fill's fringe
    // at femtovg's extrusion cap: 1.5 × √600 ≈ 36.7 px.
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (152.0, 62.0, 86.0, 86.0));
}

#[test]
fn an_icon_without_content_damages_its_box() {
    let rect = canvas_draw_damage(
        &icon(100.0, 100.0, 100.0, 100.0),
        canvas(),
        &HashMap::new(),
        0,
        &no_icons,
    )
    .region()
    .expect("BUG: an icon is bounded");
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (99.0, 99.0, 102.0, 102.0));
}

/// The renderer scales the icon about its box centre, (150, 150) here,
/// which moves the sliver rather than growing it in place.
#[test]
fn a_shrinking_icon_covers_its_content_pulled_towards_the_box_centre() {
    let rect = scaled_sliver(1.0, 0.5);
    assert!(
        covers(rect, (170.0, 200.0), (100.0, 130.0)),
        "the sliver sits at x 170..175, y 125..130 at 0.5x: {rect:?}"
    );
}

/// A factor that cannot turn negative only slides the sliver along its ray
/// from the centre, so the bound need not reach the mirrored side.
#[test]
fn a_shrinking_icon_stays_on_its_own_side_of_the_box_centre() {
    let rect = scaled_sliver(1.0, 0.5);
    assert!(
        rect.x > 120.0 && rect.y + rect.h < 180.0,
        "the sliver never crosses x 170 or y 130 at 0.5x..1x, nor its margin x 120 or y 180: {rect:?}"
    );
}

#[test]
fn a_growing_icon_covers_its_content_pushed_away_from_the_box_centre() {
    let rect = scaled_sliver(1.0, 2.0);
    assert!(
        covers(rect, (190.0, 250.0), (50.0, 110.0)),
        "the sliver sits at x 230..250, y 50..70 at 2x: {rect:?}"
    );
}

#[test]
fn a_mirroring_icon_covers_its_content_across_the_box_centre() {
    let rect = scaled_sliver(-1.0, 1.0);
    assert!(
        covers(rect, (100.0, 200.0), (100.0, 200.0)),
        "the sliver sits at x 100..110, y 190..200 at -1x: {rect:?}"
    );
}

#[test]
fn a_width_transition_scales_the_content_about_the_box_centre() {
    let inner = icon(100.0, 100.0, 100.0, 100.0);
    let mut from = extract_draw_values(&inner);
    from.w = 200.0;
    let (draw, transitions) = transitioning(inner, from);
    let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0, &|_| {
        Some(filled(TOP_RIGHT_SLIVER))
    })
    .region()
    .expect("BUG: a width transition is bounded");
    assert!(
        covers(rect, (190.0, 250.0), (50.0, 110.0)),
        "starting at twice the width puts the sliver at x 230..250, y 50..70: {rect:?}"
    );
}

/// The clock's second hand, compiled and registered as the widget ships it:
/// a 4-unit sliver of its 398-unit square viewBox,
/// pivoting 198 units down and ticking 6° about the centre of a 638×480 canvas.
#[test]
fn a_letterboxed_clock_hand_damages_its_sweep_not_its_square() {
    let canvas = Rect::new(0.0, 0.0, 638.0, 480.0);
    let mut registry = crate::gpu::svg::SvgRegistry::new();
    let svg = include_str!("../../../widgets-wasm/clock/assets/analog/hand-second.svg");
    let id = registry
        .register("hand-second", &bmc_svg_compiler::compile_svg(svg))
        .expect("BUG: the second hand registers");
    let hand = registry
        .get(id)
        .and_then(|icon| icon.content)
        .expect("BUG: the second hand paints");
    assert_eq!(
        (hand.viewbox_w, hand.viewbox_h),
        (398.0, 398.0),
        "the placement below assumes the hand's 398-unit square"
    );
    let rotated = DrawCommand::Rotated {
        angle: 0.0,
        inner: Box::new(icon(319.0 - 199.0, 240.0 - 198.0, 398.0, 398.0)),
    };
    let mut from = extract_draw_values(&rotated);
    from.rotation = -6.0_f32.to_radians();
    let (draw, transitions) = transitioning(rotated, from);

    let tight = canvas_draw_damage(&draw, canvas, &transitions, 0, &|_| Some(hand))
        .region()
        .expect("BUG: a hand is bounded");
    let boxed = canvas_draw_damage(&draw, canvas, &transitions, 0, &no_icons)
        .region()
        .expect("BUG: a hand is bounded");

    assert!(
        tight.x <= 317.0 && tight.x + tight.w >= 321.0 && tight.y <= 42.0,
        "covers the hand at rest: {tight:?}"
    );
    assert!(
        tight.w * tight.h < boxed.w * boxed.h * 0.2,
        "the sweep must cost a fraction of the square's: {tight:?} vs {boxed:?}"
    );
}

/// Past 2²⁰ px the rect's sums lose sub-pixel precision and can cancel.
#[test]
fn an_icon_reaching_far_off_its_box_damages_the_whole_canvas() {
    let far = filled(Rect::new(0.0, 0.0, 1e9, 1e9));
    let damage = canvas_draw_damage(
        &icon(100.0, 100.0, 100.0, 100.0),
        canvas(),
        &HashMap::new(),
        0,
        &|_| Some(far),
    );
    assert!(matches!(damage, DrawDamage::WholeCanvas), "got {damage:?}");
}

/// A shrink cancels a growth composed onto it, so the draw never leaves its box.
#[test]
fn a_shrink_and_a_growth_that_cancel_leave_the_box_unscaled() {
    let half = HostAnimationDef {
        from: 0.5,
        to: 0.5,
        ..scaling(1.0, 1.0)
    };
    let double = HostAnimationDef {
        from: 2.0,
        to: 2.0,
        ..scaling(1.0, 1.0)
    };
    let draw = DrawCommand::Modified {
        animations: vec![half, double],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(square(100.0, 100.0, 20.0)),
    };
    let rect = damage(&draw).expect("BUG: scale animations are bounded");
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (99.0, 99.0, 22.0, 22.0));
}
