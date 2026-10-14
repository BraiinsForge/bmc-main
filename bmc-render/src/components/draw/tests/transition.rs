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

fn transition_arc(end_angle: f32, duration_ms: u32) -> DrawCommand {
    DrawCommand::Modified {
        animations: Vec::new(),
        transition: Some(crate::tree::HostTransitionDef {
            id_hash: 42,
            duration_ms,
            easing: Easing::Linear,
        }),
        color_space: ColorSpace::default(),
        inner: Box::new(DrawCommand::Arc {
            cx: 20.0,
            cy: 30.0,
            radius: 40.0,
            start_angle: 0.0,
            end_angle,
            width: 6.0,
            fill: ArcFill::Solid(Color::from_rgb(1, 2, 3)),
            segments: ArcSegments::Continuous,
            cap: ArcCap::Round,
        }),
    }
}

fn transition_arc_frame(
    renderer: &mut RecordingRenderer,
    anim_ctx: &mut AnimationContext<'_>,
    delta_ms: u32,
    end_angle: f32,
    duration_ms: u32,
) {
    renderer.events.clear();
    anim_ctx.has_active = false;
    anim_ctx.delta_ms = delta_ms;
    anim_ctx.frame_counter += 1;
    render_draw_inner_for_test(
        renderer,
        &transition_arc(end_angle, duration_ms),
        0.0,
        0.0,
        100.0,
        100.0,
        DrawOffset::default(),
        0.0,
        1.0,
        1.0,
        0.0,
        None,
        anim_ctx,
    );
}

fn recorded_arc_end_angle(renderer: &RecordingRenderer) -> f32 {
    let [RenderEvent::Arc { end_angle, .. }] = &renderer.events[..] else {
        panic!("BUG: expected one arc draw event");
    };
    *end_angle
}

fn transition_sphere(center_lon: f32, duration_ms: u32) -> DrawCommand {
    transition_sphere_with_easing(center_lon, duration_ms, Easing::Linear)
}

fn transition_sphere_with_easing(center_lon: f32, duration_ms: u32, easing: Easing) -> DrawCommand {
    DrawCommand::Modified {
        animations: Vec::new(),
        transition: Some(crate::tree::HostTransitionDef {
            id_hash: 43,
            duration_ms,
            easing,
        }),
        color_space: ColorSpace::default(),
        inner: Box::new(DrawCommand::Sphere {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
            bitmap_id: BitmapId::from_wire(1),
            atmosphere: true,
            center_lat: 0.0,
            center_lon,
            zoom: 1.8,
            light_lat: 0.0,
            light_lon: 0.0,
        }),
    }
}

fn transition_sphere_frame(
    renderer: &mut RecordingRenderer,
    anim_ctx: &mut AnimationContext<'_>,
    delta_ms: u32,
    center_lon: f32,
    duration_ms: u32,
) -> f32 {
    renderer.events.clear();
    anim_ctx.has_active = false;
    anim_ctx.delta_ms = delta_ms;
    anim_ctx.frame_counter += 1;
    render_draw_inner_for_test(
        renderer,
        &transition_sphere(center_lon, duration_ms),
        0.0,
        0.0,
        100.0,
        100.0,
        DrawOffset::default(),
        0.0,
        1.0,
        1.0,
        0.0,
        None,
        anim_ctx,
    );
    let [RenderEvent::Sphere { center_lon }] = &renderer.events[..] else {
        panic!("BUG: expected one sphere draw event");
    };
    *center_lon
}

#[test]
fn shortest_degree_delta_handles_accumulated_turns() {
    for (from, to, expected) in [
        (539.98, -179.95, 0.07),
        (-539.98, 179.95, -0.07),
        (540.0, -180.0, 0.0),
        (0.0, 180.0, 180.0),
        (0.0, -180.0, -180.0),
    ] {
        let actual = shortest_angle_delta_deg(from, to);
        assert!(
            (actual - expected).abs() < 0.001,
            "{from}° to {to}° must move {expected}°, got {actual}°"
        );
    }
}

#[test]
fn shortest_radian_delta_handles_accumulated_turns() {
    let pi = std::f32::consts::PI;
    for (from, to, expected) in [
        (3.0 * pi, -pi + 0.05, 0.05),
        (-3.0 * pi, pi - 0.05, -0.05),
        (0.0, pi, pi),
        (0.0, -pi, -pi),
    ] {
        let actual = shortest_angle_delta(from, to);
        assert!(
            (actual - expected).abs() < 0.000_01,
            "{from} rad to {to} rad must move {expected} rad, got {actual} rad"
        );
    }
}

#[test]
fn interpolation_bounds_wrapped_angles_in_the_target_frame() {
    let turns = 20.0 * std::f32::consts::TAU;
    let from = PrevDrawValues {
        angle: turns + 0.1,
        rotation: turns + 0.2,
        arc_start_angle: turns + 0.3,
        center_lon: 7_200.4,
        light_lon: 7_200.5,
        ..PrevDrawValues::default()
    };
    let target = PrevDrawValues {
        angle: 4.0,
        rotation: 4.1,
        arc_start_angle: 4.2,
        center_lon: 200.0,
        light_lon: -200.0,
        ..PrevDrawValues::default()
    };
    let values = interpolate_draw_values(&from, &target, 0.0, ColorSpace::default());

    assert!((values.angle - target.angle).abs() <= std::f32::consts::PI);
    assert!((values.rotation - target.rotation).abs() <= std::f32::consts::PI);
    assert!((values.arc_start_angle - target.arc_start_angle).abs() <= std::f32::consts::PI);
    assert!((values.center_lon - target.center_lon).abs() <= 180.0);
    assert!((values.light_lon - target.light_lon).abs() <= 180.0);

    let settled = interpolate_draw_values(&from, &target, 1.0, ColorSpace::default());
    assert_eq!(settled.angle.to_bits(), target.angle.to_bits());
    assert_eq!(settled.rotation.to_bits(), target.rotation.to_bits());
    assert_eq!(
        settled.arc_start_angle.to_bits(),
        target.arc_start_angle.to_bits()
    );
    assert_eq!(settled.center_lon.to_bits(), target.center_lon.to_bits());
    assert_eq!(settled.light_lon.to_bits(), target.light_lon.to_bits());
}

#[test]
fn repeated_antimeridian_crossings_never_reverse_the_globe() {
    const TARGET_STEP_DEGREES: f32 = 0.065;
    const CACHED_FRAME_MS: u32 = 100;
    const CACHED_FRAMES_PER_TARGET: usize = 10;

    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);
    let mut previous = transition_sphere_frame(&mut renderer, &mut anim_ctx, 0, 170.0, 1_000);

    for step in 1..=6_000 {
        let target = normalize_angle_deg(170.0 + step as f32 * TARGET_STEP_DEGREES);
        for _ in 0..CACHED_FRAMES_PER_TARGET {
            let rendered = transition_sphere_frame(
                &mut renderer,
                &mut anim_ctx,
                CACHED_FRAME_MS,
                target,
                1_000,
            );
            let movement = normalize_angle_deg(rendered - previous).abs();
            assert!(
                movement <= TARGET_STEP_DEGREES,
                "target {step} moved globe {movement}° from {previous}° to {rendered}°"
            );
            previous = rendered;
        }
    }
}

#[test]
fn zero_duration_retarget_snaps_and_the_next_transition_starts_there() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

    transition_sphere_frame(&mut renderer, &mut anim_ctx, 0, 10.0, 1_000);
    let snapped = transition_sphere_frame(&mut renderer, &mut anim_ctx, 30_000, 50.0, 0);

    assert_eq!(snapped.to_bits(), 50.0_f32.to_bits());
    assert!(
        !anim_ctx.has_active,
        "a snap must not request cached frames"
    );

    let transition_start = transition_sphere_frame(&mut renderer, &mut anim_ctx, 300, 51.0, 1_000);
    assert_eq!(transition_start.to_bits(), 50.0_f32.to_bits());
    assert!(
        anim_ctx.has_active,
        "the ordinary update after a snap must animate"
    );

    let halfway = transition_sphere_frame(&mut renderer, &mut anim_ctx, 500, 51.0, 1_000);
    assert_eq!(halfway.to_bits(), 50.5_f32.to_bits());
}

#[test]
fn retarget_anchors_against_the_active_transition_duration() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

    transition_sphere_frame(&mut renderer, &mut anim_ctx, 0, 0.0, 1_000);
    transition_sphere_frame(&mut renderer, &mut anim_ctx, 0, 11.0, 1_100);
    let before_retarget = transition_sphere_frame(&mut renderer, &mut anim_ctx, 1_000, 11.0, 1_100);
    let retargeted = transition_sphere_frame(&mut renderer, &mut anim_ctx, 0, 20.0, 1_000);

    assert_eq!(before_retarget.to_bits(), 10.0_f32.to_bits());
    assert_eq!(retargeted.to_bits(), before_retarget.to_bits());

    let halfway = transition_sphere_frame(&mut renderer, &mut anim_ctx, 500, 20.0, 1_000);
    assert_eq!(halfway.to_bits(), 15.0_f32.to_bits());
}

#[test]
fn retarget_anchors_against_the_active_transition_easing() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

    let mut render = |delta_ms, center_lon, easing| {
        renderer.events.clear();
        anim_ctx.has_active = false;
        anim_ctx.delta_ms = delta_ms;
        anim_ctx.frame_counter += 1;
        render_draw_inner_for_test(
            &mut renderer,
            &transition_sphere_with_easing(center_lon, 1_000, easing),
            0.0,
            0.0,
            100.0,
            100.0,
            DrawOffset::default(),
            0.0,
            1.0,
            1.0,
            0.0,
            None,
            &mut anim_ctx,
        );
        let [RenderEvent::Sphere { center_lon }] = &renderer.events[..] else {
            panic!("BUG: expected one sphere draw event");
        };
        *center_lon
    };

    render(0, 0.0, Easing::EaseOut);
    render(0, 10.0, Easing::EaseOut);
    let before_retarget = render(500, 10.0, Easing::EaseOut);
    let retargeted = render(0, 20.0, Easing::Linear);

    assert_eq!(before_retarget.to_bits(), 7.5_f32.to_bits());
    assert_eq!(retargeted.to_bits(), before_retarget.to_bits());
}

/// A drag following a finger retargets every frame, and the clock has to
/// advance on those frames too: re-anchoring `from` at `t = 0` each time
/// pins the draw to its first value for the whole interaction.
#[test]
fn a_transition_retargeted_every_frame_still_advances() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

    // Settles the state at 1.0, the way a first frame does.
    transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 1.0, 1_000);

    let mut rendered = Vec::new();
    for step in 1..=6_u8 {
        let target = 1.0 + f32::from(step);
        transition_arc_frame(&mut renderer, &mut anim_ctx, 100, target, 1_000);
        rendered.push(recorded_arc_end_angle(&renderer));
    }

    assert!(
        rendered.windows(2).all(|w| w[1] > w[0]),
        "each frame must land past the one before it, got {rendered:?}"
    );
    assert!(
        rendered.last().is_some_and(|last| *last > 1.5),
        "six frames of chasing a rising target must leave the start well \
         behind, got {rendered:?}"
    );
}

/// The advance belongs *before* the retarget, not after it. A frame slower
/// than the whole duration must still start the new transition at its old
/// position. Charged afterwards, the clock lands past the duration on the
/// retarget frame itself, and the new transition is skipped entirely.
#[test]
fn a_frame_slower_than_the_duration_still_starts_the_new_transition() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

    transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 1.0, 1_000);
    // 1500 ms against the arc's 1000 ms duration.
    transition_arc_frame(&mut renderer, &mut anim_ctx, 1500, 3.0, 1_000);

    assert_eq!(
        recorded_arc_end_angle(&renderer).to_bits(),
        1.0_f32.to_bits(),
        "the new transition starts where the old one ended, not at its target"
    );
}

#[test]
fn arc_transition_interpolates_sweep() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

    transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 1.0, 1_000);
    transition_arc_frame(&mut renderer, &mut anim_ctx, 16, 3.0, 1_000);
    transition_arc_frame(&mut renderer, &mut anim_ctx, 500, 3.0, 1_000);

    let [
        RenderEvent::Arc {
            cx,
            cy,
            radius,
            start_angle: 0.0,
            end_angle,
            width,
            fill,
            segments,
            cap,
        },
    ] = &renderer.events[..]
    else {
        panic!("BUG: expected one arc draw event");
    };
    assert_eq!(cx.to_bits(), 20.0_f32.to_bits());
    assert_eq!(cy.to_bits(), 30.0_f32.to_bits());
    assert_eq!(radius.to_bits(), 40.0_f32.to_bits());
    assert_eq!(end_angle.to_bits(), 2.0_f32.to_bits());
    assert_eq!(width.to_bits(), 6.0_f32.to_bits());
    assert_eq!(*fill, ArcFill::Solid(Color::from_rgb(1, 2, 3)));
    assert_eq!(*segments, ArcSegments::Continuous);
    assert_eq!(*cap, ArcCap::Round);
}

fn gauge_arc(start_angle: f32, end_angle: f32, easing: Easing) -> DrawCommand {
    sized_arc(30.0, 6.0, (start_angle, end_angle), easing, Vec::new())
}

fn sized_arc(
    radius: f32,
    width: f32,
    (start_angle, end_angle): (f32, f32),
    easing: Easing,
    animations: Vec<crate::tree::HostAnimationDef>,
) -> DrawCommand {
    DrawCommand::Modified {
        animations,
        transition: Some(crate::tree::HostTransitionDef {
            id_hash: 42,
            duration_ms: 500,
            easing,
        }),
        color_space: ColorSpace::default(),
        inner: Box::new(DrawCommand::Arc {
            cx: 50.0,
            cy: 50.0,
            radius,
            start_angle,
            end_angle,
            width,
            fill: ArcFill::Solid(Color::from_rgb(1, 2, 3)),
            segments: ArcSegments::Continuous,
            cap: ArcCap::Round,
        }),
    }
}

/// Arc transitions as `(before, after)` pairs:
/// overshooting easings, a sweep overshooting below zero,
/// a start angle travelling round, or wrapping past 12 o'clock,
/// a radius growing, from zero too, with a thick stroke,
/// and a radius or scale pulling the arc towards the ring's centre.
fn arc_transitions() -> [(DrawCommand, DrawCommand); 10] {
    let shrinking = crate::tree::HostAnimationDef {
        property: AnimProperty::Scale,
        from: 1.0,
        to: 0.4,
        duration_ms: 500,
        delay_ms: 0,
        easing: Easing::EaseOutElastic,
        loop_mode: LoopMode::Once,
    };
    [
        (
            gauge_arc(0.0, 0.5, Easing::EaseOutCubic),
            gauge_arc(2.0, 2.5, Easing::EaseOutCubic),
        ),
        (
            sized_arc(15.0, 20.0, (0.0, 1.0), Easing::Linear, Vec::new()),
            sized_arc(40.0, 20.0, (0.0, 1.0), Easing::Linear, Vec::new()),
        ),
        (
            sized_arc(0.0, 10.0, (0.0, 1.0), Easing::Linear, Vec::new()),
            sized_arc(30.0, 10.0, (0.0, 1.0), Easing::Linear, Vec::new()),
        ),
        (
            sized_arc(40.0, 6.0, (0.0, 1.0), Easing::EaseOutElastic, Vec::new()),
            sized_arc(15.0, 6.0, (0.0, 1.0), Easing::EaseOutElastic, Vec::new()),
        ),
        (
            gauge_arc(0.2, 0.8, Easing::EaseOutBack),
            sized_arc(30.0, 6.0, (0.2, 1.2), Easing::EaseOutBack, vec![shrinking]),
        ),
        (
            gauge_arc(0.0, 0.4, Easing::EaseOutBack),
            gauge_arc(0.0, 0.9, Easing::EaseOutBack),
        ),
        (
            gauge_arc(0.0, 2.0, Easing::Linear),
            gauge_arc(0.0, 1.0, Easing::Linear),
        ),
        (
            gauge_arc(6.0, 6.5, Easing::EaseOutElastic),
            gauge_arc(0.5, 1.0, Easing::EaseOutElastic),
        ),
        (
            gauge_arc(0.0, 0.6, Easing::EaseOutBack),
            gauge_arc(0.0, 0.0, Easing::EaseOutBack),
        ),
        (
            gauge_arc(0.0, 0.6, Easing::EaseOutElastic),
            gauge_arc(0.0, 0.05, Easing::EaseOutElastic),
        ),
    ]
}

/// Every arc a transition paints until the next retarget
/// lies inside the damage taken right after that retarget.
#[test]
fn every_arc_a_transition_paints_lands_inside_its_damage() {
    let canvas = crate::interaction::Rect::new(0.0, 0.0, 100.0, 100.0);
    for (before, after) in arc_transitions() {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);
        let mut frame = |draw: &DrawCommand, anim_ctx: &mut AnimationContext<'_>, delta_ms| {
            anim_ctx.delta_ms = delta_ms;
            anim_ctx.frame_counter += 1;
            // Animation state is keyed by draw position, which the walk restarts each pass.
            anim_ctx.draw_counter = 0;
            render_draw_inner_for_test(
                &mut renderer,
                draw,
                canvas.x,
                canvas.y,
                canvas.w,
                canvas.h,
                DrawOffset::default(),
                0.0,
                1.0,
                1.0,
                0.0,
                None,
                anim_ctx,
            );
        };
        frame(&before, &mut anim_ctx, 0);
        frame(&after, &mut anim_ctx, 16);
        let crate::damage::DrawDamage::Region(damage) = crate::damage::canvas_draw_damage(
            &after,
            canvas,
            anim_ctx.transition_states,
            0,
            &|_| None,
        ) else {
            panic!("BUG: an arc transition is bounded");
        };
        for _ in 0..40 {
            frame(&after, &mut anim_ctx, 16);
        }

        for event in &renderer.events {
            for (x, y) in painted_arc_outline(event) {
                assert!(
                    x >= damage.x
                        && x <= damage.x + damage.w
                        && y >= damage.y
                        && y <= damage.y + damage.h,
                    "({x}, {y}) of {event:?} falls outside {damage:?}"
                );
            }
        }
    }
}

/// Points around the edge of what femtovg paints for one recorded arc:
/// the stroke's inner and outer edges, and a round cap's disc at each end.
///
/// The renderer splits a sweep into chunks, and femtovg winds a negative one
/// forward by full turns, so a negative sweep paints the ring like a full turn does.
fn painted_arc_outline(event: &RenderEvent) -> Vec<(f32, f32)> {
    use std::f32::consts::TAU;
    let RenderEvent::Arc {
        cx,
        cy,
        radius,
        start_angle: start,
        end_angle: end,
        width,
        cap,
        ..
    } = *event
    else {
        return Vec::new();
    };
    let sweep = end - start;
    let whole_ring = !(0.0..TAU).contains(&sweep);
    let (first, span) = if whole_ring {
        (0.0, TAU)
    } else {
        (start, sweep)
    };
    let at = |reach: f32, angle: f32| (cx + reach * angle.sin(), cy - reach * angle.cos());
    let mut points = Vec::new();
    for step in 0..=64_u8 {
        let angle = first + span * f32::from(step) / 64.0;
        points.push(at(radius - width / 2.0, angle));
        points.push(at(radius + width / 2.0, angle));
    }
    if !whole_ring && matches!(cap, ArcCap::Round) {
        for end_angle in [start, end] {
            let (ex, ey) = at(radius, end_angle);
            for step in 0..16_u8 {
                let around = TAU * f32::from(step) / 16.0;
                points.push((
                    ex + width / 2.0 * around.cos(),
                    ey + width / 2.0 * around.sin(),
                ));
            }
        }
    }
    points
}

#[test]
fn a_gauge_gaining_a_tick_damages_a_wedge_not_the_ring() {
    let canvas = crate::interaction::Rect::new(0.0, 0.0, 100.0, 100.0);
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);
    for (draw, delta_ms) in [
        (gauge_arc(0.0, 0.4, Easing::EaseOutCubic), 0),
        (gauge_arc(0.0, 0.6, Easing::EaseOutCubic), 16),
    ] {
        anim_ctx.delta_ms = delta_ms;
        anim_ctx.frame_counter += 1;
        render_draw_inner_for_test(
            &mut renderer,
            &draw,
            canvas.x,
            canvas.y,
            canvas.w,
            canvas.h,
            DrawOffset::default(),
            0.0,
            1.0,
            1.0,
            0.0,
            None,
            &mut anim_ctx,
        );
    }
    let crate::damage::DrawDamage::Region(damage) = crate::damage::canvas_draw_damage(
        &gauge_arc(0.0, 0.6, Easing::EaseOutCubic),
        canvas,
        anim_ctx.transition_states,
        0,
        &|_| None,
    ) else {
        panic!("BUG: an arc transition is bounded");
    };
    // The 66x66 ring box costs 4356 px².
    assert!(
        damage.area() < 1_000.0,
        "the top-right slice of the ring, got {damage:?}"
    );
}

#[test]
fn transition_start_frame_does_not_consume_the_gap_delta() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

    transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 1.0, 1_000);

    // A widget rendering at 1 Hz: the whole 1000 ms gap elapsed before the
    // new target appeared, so the 1000 ms transition starts here rather
    // than ending here.
    transition_arc_frame(&mut renderer, &mut anim_ctx, 1_000, 3.0, 1_000);
    assert_eq!(
        recorded_arc_end_angle(&renderer).to_bits(),
        1.0_f32.to_bits()
    );
    assert!(
        anim_ctx.has_active,
        "an unfinished transition must keep asking for frames"
    );
}

#[test]
fn zero_duration_transition_stays_at_target_when_animation_resumes() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

    transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 1.0, 1_000);
    transition_arc_frame(&mut renderer, &mut anim_ctx, 2_001, 3.0, 0);
    assert_eq!(
        recorded_arc_end_angle(&renderer).to_bits(),
        3.0_f32.to_bits()
    );
    assert!(
        !anim_ctx.has_active,
        "a snapped update needs no animation frames"
    );

    transition_arc_frame(&mut renderer, &mut anim_ctx, 16, 3.0, 1_000);
    assert_eq!(
        recorded_arc_end_angle(&renderer).to_bits(),
        3.0_f32.to_bits(),
        "an immediate refresh must not move back toward the pre-gap position"
    );

    assert!(
        !anim_ctx.has_active,
        "a settled snap must not schedule redundant animation frames"
    );

    transition_arc_frame(&mut renderer, &mut anim_ctx, 16, 5.0, 1_000);
    assert_eq!(
        recorded_arc_end_angle(&renderer).to_bits(),
        3.0_f32.to_bits(),
        "the next animation must start at the snapped position"
    );
    transition_arc_frame(&mut renderer, &mut anim_ctx, 500, 5.0, 1_000);
    assert_eq!(
        recorded_arc_end_angle(&renderer).to_bits(),
        4.0_f32.to_bits()
    );
    assert!(anim_ctx.has_active, "normal updates must animate again");
}

#[test]
fn first_frame_snap_does_not_replay_frames_once_a_duration_returns() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

    // A slot's first frame carries a zero delta
    // even when the widget saw a long gap and asked for a snap.
    transition_arc_frame(&mut renderer, &mut anim_ctx, 0, 3.0, 0);
    assert!(!anim_ctx.has_active);

    transition_arc_frame(&mut renderer, &mut anim_ctx, 16, 3.0, 500);
    assert_eq!(
        recorded_arc_end_angle(&renderer).to_bits(),
        3.0_f32.to_bits()
    );
    assert!(
        !anim_ctx.has_active,
        "a snap with no elapsed time must still count as finished"
    );
}
