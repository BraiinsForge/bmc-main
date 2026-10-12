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

/// An open modal whose body is a single static paragraph.
fn modal_tree_with_body_text(text: &str) -> TreeNode {
    let TreeNode::Column(props, mut children) = modal_tree(true) else {
        unreachable!("BUG: modal_tree builds a Column")
    };
    if let Some(TreeNode::Modal { body, .. }) = children.last_mut() {
        *body = vec![TreeNode::Paragraph {
            props: PropsData::default(),
            base_style: TextStyle::default(),
            spans: vec![SpanData {
                text: text.to_owned(),
                weight: None,
                color: None,
                size: None,
                italic: false,
                underline: false,
                strikethrough: false,
            }],
        }];
    }
    TreeNode::Column(props, children)
}

fn modal_tree(is_open: bool) -> TreeNode {
    TreeNode::Column(
        PropsData::default(),
        vec![
            TreeNode::Spacer { flex: 1.0 },
            TreeNode::Modal {
                modal_id: "confirm".to_owned(),
                is_open,
                padding: 8,
                backdrop_alpha: 160,
                title: String::new(),
                content_height: 40.0,
                bg_color: Color::default(),
                header_color: Color::default(),
                title_color: Color::default(),
                max_width: 0,
                body: Vec::new(),
                footer_primary_key: String::new(),
                footer_primary_label: String::new(),
                footer_secondary_key: String::new(),
                footer_secondary_label: String::new(),
                footer_danger: false,
            },
        ],
    )
}

/// A modal that scrolls, with enough children to overflow its body.
fn scrollable_modal_tree() -> TreeNode {
    let TreeNode::Column(props, mut children) = modal_tree(true) else {
        unreachable!("BUG: modal_tree builds a Column")
    };
    if let Some(TreeNode::Modal { body, .. }) = children.last_mut() {
        *body = (0..4)
            .map(|_| TreeNode::Canvas {
                props: PropsData {
                    width: 60.0,
                    height: 60.0,
                    ..PropsData::default()
                },
                touch_key: None,
                draws: Vec::new(),
            })
            .collect();
    }
    TreeNode::Column(props, children)
}

/// The modal overlays are walked after `render_tree` returns, so anything
/// that spends the frame's drag inside that walk strands them: a capture
/// frame is the *normal* case while a modal drags, because a non-zero
/// global delta forces the layer to recapture.
#[test]
fn a_modal_body_scrolls_on_a_capture_frame() {
    use crate::interaction::TouchEvent;

    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    // Let the modal's entrance animation settle, so its body is on screen
    // and the drag below can land in it.
    for _ in 0..40 {
        capture_once(&mut state, &mut renderer, &scrollable_modal_tree());
    }

    state
        .interaction
        .push_event(TouchEvent::Down { x: 50.0, y: 50.0 });
    state
        .interaction
        .push_event(TouchEvent::Move { x: 50.0, y: 30.0 });
    state.interaction.begin_frame();
    capture_once(&mut state, &mut renderer, &scrollable_modal_tree());

    let offset = state
        .scroll_states
        .get("confirm::body")
        .map_or(0.0, |s| s.scroll_offset);
    assert!(
        offset > 0.0,
        "the modal's body must take the drag the main tree's passes did not, got {offset}"
    );
}

/// A modal's body is not in the cached layer, so a layer-reusing frame must
/// still emit it in full: inheriting `DynamicOnly` from the blit drops every
/// static node inside the modal while the modal itself keeps painting.
#[test]
fn an_open_modal_emits_its_static_body_on_a_layer_reusing_frame() {
    let mut state = SlotState::default();
    let mut renderer = KeyRecordingRenderer::default();
    let tree = modal_tree_with_body_text("are you sure?");

    // First frame captures the layer, so the second can reuse it.
    render_with(&mut state, &mut renderer, &tree, LayerUse::Capture);
    renderer.paragraphs.clear();
    renderer.calls.clear();
    render_with(&mut state, &mut renderer, &tree, LayerUse::Reuse);

    assert!(
        renderer.calls.iter().any(|(call, _)| *call == "blit"),
        "the second frame must reuse the layer, or this proves nothing",
    );
    assert!(
        renderer.paragraphs.iter().any(|t| t == "are you sure?"),
        "the modal's body text must be painted on a layer-reusing frame; \
         got {:?}",
        renderer.paragraphs,
    );
}

/// The backdrop dims the whole surface from outside the walk, so a frame
/// scissored to the walk's rects darkens only those.
#[test]
fn an_open_modal_damages_the_whole_surface() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    let result = render_once(&mut state, &mut renderer, &modal_tree(true));

    assert!(
        result
            .dynamic_rects
            .iter()
            .any(|r| r.w >= 100.0 && r.h >= 100.0),
        "an open modal must damage the surface, got {:?}",
        result.dynamic_rects
    );
}

#[test]
fn a_closed_modal_damages_nothing() {
    let mut renderer = KeyRecordingRenderer::default();
    let mut state = SlotState::default();

    let result = render_once(&mut state, &mut renderer, &modal_tree(false));

    assert!(
        result.dynamic_rects.is_empty(),
        "a modal that never opened paints nothing, got {:?}",
        result.dynamic_rects
    );
}
