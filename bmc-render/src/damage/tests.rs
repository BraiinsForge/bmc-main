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

fn damage(draw: &DrawCommand) -> Option<Rect> {
    canvas_draw_damage(draw, canvas(), &HashMap::new(), 0).region()
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
    let rect = canvas_draw_damage(&draw, square_canvas, &HashMap::new(), 0)
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
    let rect = canvas_draw_damage(&draw, square_canvas, &HashMap::new(), 0)
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
    let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0)
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
    let rect = canvas_draw_damage(&draw, square_canvas, &transitions, 0)
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
    let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0)
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
    let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0)
        .region()
        .expect("BUG: an arc transition is bounded");
    assert_eq!(
        (rect.x, rect.w),
        (137.0, 126.0),
        "the widest stroke in the span sets the reach"
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
            canvas_draw_damage(&draw, canvas(), &transitions, 0),
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
        canvas_draw_damage(&square(-100.0, -100.0, 10.0), canvas(), &HashMap::new(), 0),
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
    let rect = canvas_draw_damage(&draw, canvas(), &transitions, 0)
        .region()
        .expect("BUG: a hand is bounded");
    let share = rect.w * rect.h / (1280.0 * 480.0);
    assert!(
        (0.20..0.30).contains(&share),
        "a hand should damage roughly a quarter of the screen, got {share} from {rect:?}"
    );
}
