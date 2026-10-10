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

/// A centred draw an outer translate moves, so the frame exercises both
/// halves of the renderer's offset at once.
fn translated_centred_dot() -> Node {
    Node::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws: vec![super::Draw::Modified {
            animations: vec![super::AnimationDef {
                property: AnimProperty::TranslateY,
                from: 0.0,
                to: 30.0,
                duration_ms: 500,
                delay_ms: 0,
                easing: Easing::Linear,
                loop_mode: LoopMode::Forever,
            }],
            transition: None,
            color_space: ColorSpace::default(),
            inner: Box::new(super::Draw::Centered {
                inner: Box::new(super::Draw::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 12.0,
                    h: 12.0,
                    fill: Fill::Solid(Color::from_rgb(9, 9, 9)),
                }),
            }),
        }],
    }
}

/// The contract a damage-scissored frame rests on: everything the dynamic
/// pass paints falls inside the reported damage, or the scissor drops a
/// pixel that then never lands.
///
/// The span is two walks wide because the device rotates export buffers:
/// this frame paints into the buffer the frame *before* last drew, so the
/// region that one moved is still stale in it.
fn assert_dynamic_draws_land_inside_span(older: &Node, newer: &Node) {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    let older = render_once(&mut state, &mut renderer, older);
    let newest = render_once(&mut state, &mut renderer, newer);

    let span: Vec<_> = newest
        .dynamic_rects
        .iter()
        .chain(older.dynamic_rects.iter())
        .copied()
        .collect();
    assert!(
        !span.is_empty(),
        "an animating draw must report damage, or a cached frame repaints nothing"
    );

    renderer.draws.clear();
    render_dynamic_pass(&mut state, &mut renderer, newer, &span);

    assert!(
        !renderer.draws.is_empty(),
        "the dynamic pass must paint the animating draw, or this proves nothing"
    );
    for draw in &renderer.draws {
        assert!(
            within(&span, *draw),
            "draw {draw:?} falls outside the damage span {span:?}, so the scissor drops it"
        );
    }
}

/// An 8 px square at `(x, y)` sliding 2 px right and back.
fn sliding_square(x: f32, y: f32) -> super::Draw {
    super::Draw::Modified {
        animations: vec![super::AnimationDef {
            property: AnimProperty::TranslateX,
            from: 0.0,
            to: 2.0,
            duration_ms: 500,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(super::Draw::Rect {
            x,
            y,
            w: 8.0,
            h: 8.0,
            fill: Fill::Solid(Color::from_rgb(9, 9, 9)),
        }),
    }
}

#[test]
fn draws_far_apart_in_one_canvas_report_a_rect_each() {
    let tree = Node::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws: vec![sliding_square(2.0, 2.0), sliding_square(88.0, 88.0)],
    };
    let mut renderer = KeyRecordingRenderer::default();
    let result = render_once(&mut SlotState::default(), &mut renderer, &tree);

    let sizes: Vec<_> = result
        .dynamic_rects
        .iter()
        .map(|rect| (rect.w, rect.h))
        .collect();
    assert!(
        sizes.len() == 2 && sizes.iter().all(|&(w, h)| w <= 14.0 && h <= 12.0),
        "each square owes only its own slide, not the 94 px box spanning both: {sizes:?}"
    );
}

#[test]
fn every_dynamic_draw_lands_inside_the_two_walk_damage_span() {
    assert_dynamic_draws_land_inside_span(&moving_dot(10.0), &moving_dot(60.0));
}

/// An outer translate composes onto the centring in both walks; a bound
/// that replaced it instead would scissor the draw away.
#[test]
fn a_translated_centred_draw_lands_inside_the_two_walk_damage_span() {
    assert_dynamic_draws_land_inside_span(&translated_centred_dot(), &translated_centred_dot());
}

/// When the layer blit fails, the frame falls back to a full walk,
/// which paints only where a draw paints.
/// Without a base under it, last frame's dynamic pixels show through.
#[test]
fn a_failed_layer_blit_lays_the_frame_base_before_the_walk() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();
    let damage = [crate::interaction::Rect::new(0.0, 0.0, 30.0, 30.0)];
    let mut ctx = ProcessContext {
        interaction: &mut state.interaction,
        modal_states: &mut state.modal_states,
        scroll_states: &mut state.scroll_states,
        animation_states: &mut state.animation_states,
        transition_states: &mut state.transition_states,
        taffy: &mut state.taffy,
        frame_counter: 1,
        delta_ms: 16,
        now_unix_secs: 0,
        emit: super::EmitMode::All,
        // Nothing was captured under this key, so the layer blit fails.
        static_layer: LayerUse::Reuse,
        static_layer_key: "failed-blit-test",
        layout_outlines: false,
        stroke_outlines: false,
        damage_rects: &damage,
    };
    let mut timings = FrameTimings::default();
    let (result, _) = layout_and_render(
        &moving_dot(60.0),
        100.0,
        100.0,
        &mut renderer,
        &mut timings,
        &mut ctx,
    )
    .expect("BUG: layout_and_render must succeed for the moving dot");

    let first = renderer
        .draws
        .first()
        .expect("BUG: the fallback walk must paint something");
    assert!(
        first.x <= 0.0 && first.y <= 0.0 && first.w >= 100.0 && first.h >= 100.0,
        "the first draw must cover the surface, got {:?}",
        renderer.draws
    );
    let (_, base, _) = renderer
        .fills
        .first()
        .expect("BUG: the base is a fill_rect, so it must be recorded");
    assert_eq!(
        *base,
        Color::from_rgb(0, 0, 0),
        "a base that is not opaque lets last frame's pixels show through",
    );
    assert!(
        matches!(result.static_layer, super::StaticLayer::Missed),
        "a failed blit must report the miss, or the host keeps reusing the layer \
         and never repaints the other export buffers",
    );
}

/// Walk the moving dot handed `damage`, with its layer captured beforehand when `captured`.
fn walk_with_layer_and_damage(
    static_layer: LayerUse,
    captured: bool,
    damage: &[crate::interaction::Rect],
) -> super::TreeResult {
    const KEY: &str = "damage-only-test";
    let mut renderer = KeyRecordingRenderer::default();
    if captured {
        renderer.captured.push(KEY.to_owned());
    }
    let mut state = SlotState::default();
    let mut ctx = ProcessContext {
        interaction: &mut state.interaction,
        modal_states: &mut state.modal_states,
        scroll_states: &mut state.scroll_states,
        animation_states: &mut state.animation_states,
        transition_states: &mut state.transition_states,
        taffy: &mut state.taffy,
        frame_counter: 1,
        delta_ms: 16,
        now_unix_secs: 0,
        emit: super::EmitMode::All,
        static_layer,
        static_layer_key: KEY,
        layout_outlines: false,
        stroke_outlines: false,
        damage_rects: damage,
    };
    let mut timings = FrameTimings::default();
    let (result, _) = layout_and_render(
        &moving_dot(60.0),
        100.0,
        100.0,
        &mut renderer,
        &mut timings,
        &mut ctx,
    )
    .expect("BUG: layout_and_render must succeed for the moving dot");
    result
}

fn walk_with_layer(static_layer: LayerUse, captured: bool) -> super::TreeResult {
    walk_with_layer_and_damage(
        static_layer,
        captured,
        &[crate::interaction::Rect::new(0.0, 0.0, 30.0, 30.0)],
    )
}

#[test]
fn a_reused_layer_reports_a_blit_with_or_without_damage() {
    for layer in [
        walk_with_layer_and_damage(LayerUse::Reuse, true, &[]).static_layer,
        walk_with_layer(LayerUse::Reuse, true).static_layer,
    ] {
        assert!(
            matches!(layer, super::StaticLayer::Blitted),
            "got {layer:?}"
        );
    }
}

/// Handed damage, a frame can still paint everything: the damage alone
/// would tell the host the rest of the target was kept when it was not.
#[test]
fn a_recaptured_or_missed_layer_reports_no_blit_despite_the_damage() {
    assert!(
        matches!(
            walk_with_layer(LayerUse::Capture, false).static_layer,
            super::StaticLayer::Captured { .. }
        ),
        "capturing paints the whole frame"
    );
    assert!(
        matches!(
            walk_with_layer(LayerUse::Reuse, false).static_layer,
            super::StaticLayer::Missed
        ),
        "a failed blit lays the base over the whole frame"
    );
}
