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

fn capture_frame(state: &mut SlotState, renderer: &mut KeyRecordingRenderer, key: &str) {
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
        static_layer: LayerUse::Capture,
        static_layer_key: key,
        layout_outlines: false,
        stroke_outlines: false,
        damage_rects: &[],
    };
    let tree = Node::Column(PropsData::default(), vec![Node::Spacer { flex: 1.0 }]);
    let mut timings = FrameTimings::default();
    layout_and_render(&tree, 100.0, 100.0, renderer, &mut timings, &mut ctx)
        .expect("BUG: layout_and_render must succeed for a trivial tree");
}

/// A scroll container holding static content, so the frame both
/// splits and scrolls — 200 px of children in a 100 px viewport.
fn scrolling_tree() -> Node {
    let child = |y: f32| Node::Canvas {
        props: PropsData {
            width: 100.0,
            height: 100.0,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![super::Draw::Rect {
            x: 0.0,
            y,
            w: 100.0,
            h: 100.0,
            fill: Fill::Solid(Color::from_rgb(9, 9, 9)),
        }],
    };
    Node::Scroll {
        scroll_key: "list".to_owned(),
        props: PropsData::default(),
        children: vec![child(0.0), child(100.0)],
    }
}

/// A capture frame walks the tree twice. The scroll offset advances inside
/// that walk, so without a gate the second pass adds the same drag again:
/// content runs at twice the finger and the captured half lags a delta
/// behind the dynamic one for the whole gesture.
#[test]
fn a_capture_frame_applies_a_scroll_drag_once() {
    use crate::interaction::TouchEvent;

    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    // First frame registers the scroll region the drag hit-tests against.
    capture_once(&mut state, &mut renderer, &scrolling_tree());

    state
        .interaction
        .push_event(TouchEvent::Down { x: 50.0, y: 50.0 });
    state
        .interaction
        .push_event(TouchEvent::Move { x: 50.0, y: 30.0 });
    // The host drains the queue here, hit-testing against the regions
    // that the frame above registered.
    state.interaction.begin_frame();
    capture_once(&mut state, &mut renderer, &scrolling_tree());

    let offset = state
        .scroll_states
        .get("list")
        .expect("BUG: the scroll container must record state")
        .scroll_offset;
    assert!(
        (offset - 20.0).abs() < f32::EPSILON,
        "a 20 px drag must move the content 20 px, got {offset}"
    );
}

/// A skeleton's bar is a distinctive height, so it can be counted among
/// the other draws a frame makes.
const SKELETON_BAR_H: f32 = 7.0;

fn skeleton_beside_animation() -> Node {
    Node::Column(
        PropsData::default(),
        vec![
            moving_dot(0.0),
            Node::Skeleton(SkeletonData {
                kind: bmc_ui::SkeletonKind::Placeholder,
                chars: 0.0,
                font_size: 0.0,
                width: 40.0,
                height: SKELETON_BAR_H,
                color: Color::from_rgb(9, 9, 9),
            }),
        ],
    )
}

/// Every paint site is gated on the emission mode. An ungated one paints
/// into the layer on the capture pass, then again over the blit on the
/// dynamic pass — and being static, nothing repaints underneath, so each
/// frame blends another copy onto the last. Translucent fills darken,
/// and edges thicken over a few seconds of cached frames.
#[test]
fn a_skeleton_paints_once_on_a_capture_frame() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    capture_once(&mut state, &mut renderer, &skeleton_beside_animation());

    let bars = renderer
        .draws
        .iter()
        .filter(|r| (r.h - SKELETON_BAR_H).abs() < f32::EPSILON)
        .count();
    assert_eq!(
        bars, 1,
        "the capture pass paints the bar into the layer; the dynamic pass \
         must not paint it again, got {:?}",
        renderer.draws
    );
}

/// A static mesh followed by an animating one, in one canvas.
fn static_and_dynamic_mesh() -> Node {
    let mesh = |wire: u16| super::Draw::Mesh {
        x: 0.0,
        y: 0.0,
        w: 40.0,
        h: 40.0,
        mesh_id: MeshId::from_wire(wire),
        args: crate::gpu::mesh::ZERO_MESH_ARGS,
    };
    Node::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws: vec![mesh(1), animating(mesh(2))],
    }
}

/// A capture frame walks the static half first,
/// so unless the dynamic pass restarts slot numbering,
/// its meshes land on other atlas slots than on a reuse frame and re-render.
#[test]
fn a_dynamic_mesh_keeps_its_atlas_slot_across_capture_and_reuse_frames() {
    let dynamic_mesh = MeshId::from_wire(2).expect("BUG: fixture mesh ID must be non-zero");
    let tree = static_and_dynamic_mesh();
    let mut state = SlotState::default();
    let mut renderer = KeyRecordingRenderer::default();
    let dynamic_slot = |renderer: &KeyRecordingRenderer| {
        renderer
            .meshes
            .iter()
            .find(|(id, _)| *id == dynamic_mesh)
            .map(|(_, slot)| *slot)
    };

    render_with(&mut state, &mut renderer, &tree, LayerUse::Capture);
    let on_capture = dynamic_slot(&renderer);
    renderer.meshes.clear();
    renderer.calls.clear();
    render_with(&mut state, &mut renderer, &tree, LayerUse::Reuse);
    let on_reuse = dynamic_slot(&renderer);

    assert!(
        renderer.calls.iter().any(|(call, _)| *call == "blit"),
        "the second frame must reuse the layer, or this proves nothing",
    );
    assert!(
        on_reuse.is_some(),
        "the dynamic mesh must be drawn on a reuse frame"
    );
    assert_eq!(
        on_capture, on_reuse,
        "the dynamic mesh must take the same atlas slot on both frame types"
    );
}

/// Two slots sharing one renderer must address distinct layers, or each
/// blits whatever the other just captured.
#[test]
fn each_slot_captures_under_its_own_key() {
    let mut renderer = KeyRecordingRenderer::default();
    let (mut a, mut b) = (SlotState::default(), SlotState::default());

    for (slot, key) in [(0, "widget-a"), (1, "widget-b"), (0, "widget-a")] {
        renderer.calls.clear();
        let state = if slot == 0 { &mut a } else { &mut b };
        capture_frame(state, &mut renderer, key);

        assert!(
            !renderer.calls.is_empty(),
            "slot '{key}' made no static-layer calls, so this asserts nothing"
        );
        for (op, seen) in &renderer.calls {
            assert_eq!(
                seen, key,
                "'{op}' addressed layer '{seen}' while rendering slot '{key}'"
            );
        }
    }
}

/// A scroll container outlines itself like any other box,
/// and the boxes inside it carry its viewport as their clip.
#[test]
fn a_scroll_container_is_outlined_and_clips_what_it_holds() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    let result = capture_collecting(&mut state, &mut renderer, &scrolling_tree(), true, false);

    let viewport = (0.0, 0.0, 100.0, 100.0);
    let bounds = |r: Rect| (r.x, r.y, r.w, r.h);
    let container = result
        .layout_outlines
        .iter()
        .filter(|o| bounds(o.rect) == viewport && o.clip.is_none())
        .count();
    let held = result
        .layout_outlines
        .iter()
        .filter_map(|o| o.clip.map(bounds))
        .collect::<Vec<_>>();
    assert_eq!(
        container, 1,
        "the container outlines itself, outside its own viewport"
    );
    assert_eq!(held, [viewport, viewport]);
}

/// A capture frame strokes its outlines after the layer, never into it:
/// the layer outlives the frame, and reuse frames would blit the strokes back
/// where the boxes no longer are.
#[test]
fn a_capture_frame_strokes_its_outlines_over_the_layer() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    capture_collecting(&mut state, &mut renderer, &scrolling_tree(), false, true);

    let calls = renderer
        .calls
        .iter()
        .map(|(call, _)| *call)
        .filter(|call| matches!(*call, "begin" | "end" | "stroke"))
        .collect::<Vec<_>>();
    let end = calls
        .iter()
        .position(|call| *call == "end")
        .expect("BUG: a capture frame ends the layer it began");
    assert_eq!(
        calls[..=end],
        ["begin", "end"],
        "nothing strokes into the layer"
    );
    assert!(
        calls[end..].contains(&"stroke"),
        "the outlines still stroke over it, got {calls:?}"
    );
}

/// A scroll inside a scroll clips what it holds to both viewports at once:
/// the frame's scissors nest, so the inner one never paints outside the outer.
#[test]
fn a_nested_scroll_clips_what_it_holds_to_both_viewports() {
    let canvas = |height: f32| Node::Canvas {
        props: PropsData {
            width: 100.0,
            height,
            ..PropsData::default()
        },
        touch_key: None,
        draws: Vec::new(),
    };
    // The inner viewport spans rows 60..140, past the outer one's 0..100.
    let tree = Node::Scroll {
        scroll_key: "outer".to_owned(),
        props: PropsData::default(),
        children: vec![
            canvas(60.0),
            Node::Scroll {
                scroll_key: "inner".to_owned(),
                props: PropsData {
                    width: 100.0,
                    height: 80.0,
                    ..PropsData::default()
                },
                children: vec![canvas(120.0)],
            },
        ],
    };
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    let result = capture_collecting(&mut state, &mut renderer, &tree, true, false);

    let bounds = |r: Rect| (r.x, r.y, r.w, r.h);
    let held = result
        .layout_outlines
        .iter()
        .filter(|o| o.rect.h > 100.0)
        .map(|o| o.clip.map(bounds))
        .collect::<Vec<_>>();
    assert_eq!(held, [Some((0.0, 60.0, 100.0, 40.0))]);
}

/// A capture that loses its layer has already walked the static half,
/// so the full paint after it has to start the walk over:
/// carrying on the slot numbering puts every mesh on another atlas slot than a plain frame does.
#[test]
fn a_capture_that_loses_its_layer_paints_as_a_plain_frame() {
    let tree = static_and_dynamic_mesh();
    let mut plain = KeyRecordingRenderer::default();
    render_with(
        &mut SlotState::default(),
        &mut plain,
        &tree,
        LayerUse::Ignore,
    );
    let mut renderer = KeyRecordingRenderer {
        lose_captures: true,
        ..KeyRecordingRenderer::default()
    };

    let result = render_with(
        &mut SlotState::default(),
        &mut renderer,
        &tree,
        LayerUse::Capture,
    );

    assert!(
        matches!(result.static_layer, super::StaticLayer::Missed),
        "a lost layer must report the miss, got {:?}",
        result.static_layer
    );
    assert!(
        renderer.meshes.ends_with(&plain.meshes),
        "the full paint must walk as a plain frame does: {:?} after the capture, {:?} plain",
        renderer.meshes,
        plain.meshes
    );
}
