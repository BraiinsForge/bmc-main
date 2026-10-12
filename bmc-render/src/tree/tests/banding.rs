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

/// Size of the trailing static rect, unlike anything else the tree paints
/// so the draw log identifies it.
const STATIC_RECT: f32 = 37.0;

fn sized_canvas(props: PropsData, draws: Vec<super::DrawCommand>) -> TreeNode {
    TreeNode::Canvas {
        props: PropsData {
            width: 100.0,
            height: 50.0,
            ..props
        },
        touch_key: None,
        draws,
    }
}

fn static_draws() -> Vec<super::DrawCommand> {
    vec![super::DrawCommand::Rect {
        x: 0.0,
        y: 0.0,
        w: STATIC_RECT,
        h: STATIC_RECT,
        fill: Fill::Solid(Color::from_rgb(1, 2, 3)),
    }]
}

fn animated_draws() -> Vec<super::DrawCommand> {
    let TreeNode::Canvas { draws, .. } = moving_dot(10.0) else {
        unreachable!("BUG: `moving_dot` builds a canvas")
    };
    draws
}

/// A static canvas pinned over the animated one below it.
fn static_over_dynamic() -> TreeNode {
    let over = PropsData {
        inset_top: 0.0,
        inset_left: 0.0,
        ..PropsData::default()
    };
    TreeNode::Column(
        PropsData::default(),
        vec![
            sized_canvas(PropsData::default(), animated_draws()),
            sized_canvas(over, static_draws()),
        ],
    )
}

/// The same two canvases as flex siblings, which never overlap.
fn static_beside_dynamic() -> TreeNode {
    TreeNode::Column(
        PropsData::default(),
        vec![
            sized_canvas(PropsData::default(), animated_draws()),
            sized_canvas(PropsData::default(), static_draws()),
        ],
    )
}

fn painted_static_rect(renderer: &KeyRecordingRenderer) -> bool {
    renderer
        .draws
        .iter()
        .any(|r| (r.w - STATIC_RECT).abs() < f32::EPSILON)
}

/// The layer composites in one go before the dynamic half, so static
/// content painted over pixels the dynamic half owns repaints with that
/// half: in the layer it would land underneath what it should cover.
#[test]
fn a_static_node_over_a_dynamic_one_paints_with_the_dynamic_half() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    render_pass(
        &mut state,
        &mut renderer,
        &static_over_dynamic(),
        super::EmitMode::StaticOnly,
    );
    assert!(
        !painted_static_rect(&renderer),
        "a static draw over a dynamic node must stay out of the cached layer"
    );

    renderer.draws.clear();
    render_pass(
        &mut state,
        &mut renderer,
        &static_over_dynamic(),
        super::EmitMode::DynamicOnly,
    );
    assert!(
        painted_static_rect(&renderer),
        "the pass that skipped it in the layer must paint it over the dynamic half"
    );
}

/// Paint order is only observable where the two overlap.
/// Banding a node `Above` for following an animation it never touches
/// would cost the commonest layout in the fleet its layer,
/// and damage the whole of it every frame.
#[test]
fn a_static_node_beside_a_dynamic_one_stays_in_the_layer() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    render_pass(
        &mut state,
        &mut renderer,
        &static_beside_dynamic(),
        super::EmitMode::StaticOnly,
    );
    assert!(
        painted_static_rect(&renderer),
        "a static draw that overlaps nothing dynamic belongs in the layer"
    );

    renderer.draws.clear();
    let result = render_once(&mut state, &mut renderer, &static_beside_dynamic());
    assert!(
        !result
            .dynamic_rects
            .iter()
            .any(|r| r.y >= 50.0 && r.h > 0.0),
        "nothing in the static half owes damage, got {:?}",
        result.dynamic_rects
    );
}

/// A bordered or nine-patch container over the animation owes its box
/// just as a filled one does, though only the fill shows in the fill log.
#[test]
fn a_bordered_or_nine_patch_container_over_a_dynamic_one_damages_its_box() {
    let panel_box = crate::interaction::Rect::new(0.0, 0.0, 60.0, 30.0);
    let decorations = [
        (
            "border",
            PropsData {
                border_width: 2.0,
                border_color: PANEL,
                ..PropsData::default()
            },
        ),
        (
            "nine-patch",
            PropsData {
                bg_np_id: BitmapId::from_wire(1),
                ..PropsData::default()
            },
        ),
    ];
    for (decoration, props) in decorations {
        let panel = TreeNode::Column(
            PropsData {
                width: panel_box.w,
                height: panel_box.h,
                inset_top: panel_box.y,
                inset_left: panel_box.x,
                ..props
            },
            Vec::new(),
        );
        let tree = TreeNode::Column(
            PropsData::default(),
            vec![sized_canvas(PropsData::default(), animated_draws()), panel],
        );

        let mut renderer = KeyRecordingRenderer::default();
        let mut state = SlotState::default();
        let result = render_once(&mut state, &mut renderer, &tree);

        assert!(
            within(&result.dynamic_rects, panel_box),
            "a {decoration} panel {panel_box:?} must be damaged, got {:?}",
            result.dynamic_rects
        );
    }
}

/// Colour of the container laid over the animation,
/// unlike anything else the tree paints, so the fill log identifies it.
const PANEL: Color = Color::from_rgb(7, 77, 177);

/// A container's own background is the one box an `Above` node owes whole,
/// not draw by draw.
#[test]
fn a_container_over_a_dynamic_one_paints_and_damages_its_background_above() {
    let panel = TreeNode::Column(
        PropsData {
            width: 60.0,
            height: 30.0,
            inset_top: 0.0,
            inset_left: 0.0,
            background: PANEL,
            ..PropsData::default()
        },
        Vec::new(),
    );
    let tree = TreeNode::Column(
        PropsData::default(),
        vec![sized_canvas(PropsData::default(), animated_draws()), panel],
    );
    let panel_fill = |renderer: &KeyRecordingRenderer| {
        renderer
            .fills
            .iter()
            .find(|(_, color, _)| *color == PANEL)
            .map(|(rect, _, _)| *rect)
    };

    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();
    render_pass(
        &mut state,
        &mut renderer,
        &tree,
        super::EmitMode::StaticOnly,
    );
    assert!(
        panel_fill(&renderer).is_none(),
        "a background over the animation must stay out of the layer"
    );

    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();
    let result = render_once(&mut state, &mut renderer, &tree);
    let fill = panel_fill(&renderer).expect("BUG: the single walk paints the panel");
    assert!(
        within(&result.dynamic_rects, fill),
        "the panel {fill:?} must be damaged, got {:?}",
        result.dynamic_rects
    );
}

/// Emitting it with the dynamic half is only half the contract:
/// a cached frame scissors to the damage set, so an unreported `Above`
/// node is dropped, and the layer blitted underneath holds no copy of it.
#[test]
fn a_static_node_over_a_dynamic_one_reports_damage() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    let result = render_once(&mut state, &mut renderer, &static_over_dynamic());

    renderer.draws.clear();
    render_dynamic_pass(
        &mut state,
        &mut renderer,
        &static_over_dynamic(),
        &result.dynamic_rects,
    );

    assert!(
        painted_static_rect(&renderer),
        "this proves nothing if the pass paints nothing"
    );
    for draw in &renderer.draws {
        assert!(
            within(&result.dynamic_rects, *draw),
            "draw {draw:?} falls outside the damage {:?}, so the scissor drops it",
            result.dynamic_rects
        );
    }
}

/// The dial under the hands: static, but before the animation it sits under.
fn dynamic_over_static() -> TreeNode {
    let over = PropsData {
        inset_top: 0.0,
        inset_left: 0.0,
        ..PropsData::default()
    };
    TreeNode::Column(
        PropsData::default(),
        vec![
            sized_canvas(PropsData::default(), static_draws()),
            sized_canvas(over, animated_draws()),
        ],
    )
}

/// A capture frame walks the tree twice.
/// Banding the second walk against what the first one painted
/// would put the dial `Above` the hands it precedes.
#[test]
fn a_capture_frame_bands_its_dynamic_pass_like_a_single_walk() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();
    let single_walk = render_once(&mut state, &mut renderer, &dynamic_over_static());

    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();
    let captured = capture_once(&mut state, &mut renderer, &dynamic_over_static());

    let dial_draws = renderer
        .draws
        .iter()
        .filter(|r| (r.w - STATIC_RECT).abs() < f32::EPSILON)
        .count();
    assert_eq!(
        dial_draws, 1,
        "the dial belongs in the layer alone, not repainted over it"
    );
    for rect in &captured.dynamic_rects {
        assert!(
            within(&single_walk.dynamic_rects, *rect),
            "the dial owes {rect:?}, which a single walk of {:?} does not",
            single_walk.dynamic_rects
        );
    }
}

/// The scroll viewport clips only what is inside the container.
/// Kept past it, the clip would empty the animated footer's rect,
/// and the static overlay on top of it would land in the layer underneath.
#[test]
fn a_static_node_over_a_dynamic_one_after_a_scroll_container_paints_above() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();
    let over_footer = PropsData {
        inset_top: 50.0,
        inset_left: 0.0,
        ..PropsData::default()
    };
    let tree = TreeNode::Column(
        PropsData::default(),
        vec![
            TreeNode::Scroll {
                scroll_key: "rows".to_owned(),
                props: PropsData {
                    height: 50.0,
                    ..PropsData::default()
                },
                children: vec![sized_canvas(PropsData::default(), Vec::new())],
            },
            sized_canvas(PropsData::default(), animated_draws()),
            sized_canvas(over_footer, static_draws()),
        ],
    );

    render_pass(
        &mut state,
        &mut renderer,
        &tree,
        super::EmitMode::StaticOnly,
    );

    assert!(
        !painted_static_rect(&renderer),
        "the overlay covers an animation below the viewport, so it must stay out of the layer"
    );
}

/// Paint order chains through static nodes too:
/// the second overlay touches only the first, which already paints above the animation,
/// so the second has to paint above it as well.
#[test]
fn a_static_node_over_one_already_above_paints_above_too() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();
    let pinned = |top: f32| PropsData {
        inset_top: top,
        inset_left: 0.0,
        ..PropsData::default()
    };
    let first_over = vec![super::DrawCommand::Rect {
        x: 0.0,
        y: 0.0,
        w: 100.0,
        h: 50.0,
        fill: Fill::Solid(Color::from_rgb(4, 5, 6)),
    }];
    let tree = TreeNode::Column(
        PropsData::default(),
        vec![
            sized_canvas(PropsData::default(), animated_draws()),
            sized_canvas(pinned(25.0), first_over),
            sized_canvas(pinned(60.0), static_draws()),
        ],
    );

    render_pass(
        &mut state,
        &mut renderer,
        &tree,
        super::EmitMode::StaticOnly,
    );

    assert!(
        !painted_static_rect(&renderer),
        "a node over one that paints above the animation must not sit beneath it in the layer"
    );
}

/// Everything static paints over the animation, so the layer captures nothing.
#[test]
fn a_capture_with_every_static_node_above_reports_an_empty_layer() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    let result = capture_once(&mut state, &mut renderer, &static_over_dynamic());

    assert!(result.static_layer_empty);
}

#[test]
fn a_capture_holding_the_dial_reports_a_layer_in_use() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    let result = capture_once(&mut state, &mut renderer, &dynamic_over_static());

    assert!(!result.static_layer_empty);
}

/// A row scrolled out below the viewport paints nothing,
/// so the footer laid out where it would land keeps its layer.
#[test]
fn a_static_node_beside_a_scrolled_out_dynamic_one_stays_in_the_layer() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();
    let viewport = PropsData {
        height: 50.0,
        ..PropsData::default()
    };
    let tree = TreeNode::Column(
        PropsData::default(),
        vec![
            TreeNode::Scroll {
                scroll_key: "rows".to_owned(),
                props: viewport,
                children: vec![TreeNode::Column(
                    PropsData::default(),
                    vec![
                        sized_canvas(PropsData::default(), Vec::new()),
                        sized_canvas(PropsData::default(), animated_draws()),
                    ],
                )],
            },
            sized_canvas(PropsData::default(), static_draws()),
        ],
    );

    render_pass(
        &mut state,
        &mut renderer,
        &tree,
        super::EmitMode::StaticOnly,
    );

    assert!(
        painted_static_rect(&renderer),
        "the footer overlaps only a row the viewport hides, so it belongs in the layer"
    );
}
