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

const LIT: Color = Color::from_rgb(200, 0, 0);
const SHADED: Color = Color::from_rgb(0, 0, 200);

/// A 10×10 box painted `color`, positioned by `props`.
fn swatch(color: Color, props: PropsData) -> Node {
    Node::Column(
        PropsData {
            background: color,
            width: 10.0,
            height: 10.0,
            ..props
        },
        vec![],
    )
}

fn brightness_of(renderer: &KeyRecordingRenderer, color: Color) -> f32 {
    renderer
        .fills
        .iter()
        .find(|(_, c, _)| *c == color)
        .map(|(_, _, brightness)| *brightness)
        .expect("BUG: the swatch must paint")
}

#[test]
fn a_dimmed_subtree_paints_at_its_brightness_and_its_siblings_do_not() {
    let tree = Node::Column(
        PropsData::default(),
        vec![
            super::dimmed(0.5, swatch(SHADED, PropsData::default())),
            swatch(LIT, PropsData::default()),
        ],
    );
    let mut renderer = KeyRecordingRenderer::default();
    render_once(&mut SlotState::default(), &mut renderer, &tree);
    assert_close(brightness_of(&renderer, SHADED), 0.5);
    assert_close(brightness_of(&renderer, LIT), 1.0);
}

#[test]
fn nested_dims_multiply_and_unwind() {
    let tree = super::dimmed(
        0.5,
        Node::Column(
            PropsData::default(),
            vec![
                swatch(LIT, PropsData::default()),
                super::dimmed(0.5, swatch(SHADED, PropsData::default())),
            ],
        ),
    );
    let mut renderer = KeyRecordingRenderer::default();
    render_once(&mut SlotState::default(), &mut renderer, &tree);
    assert_close(brightness_of(&renderer, LIT), 0.5);
    assert_close(brightness_of(&renderer, SHADED), 0.25);
    assert_close(renderer.brightness(), 1.0);
}

/// Wrapping must not add a box: an absolute child's insets resolve
/// against the same parent either way.
#[test]
fn a_dimmed_node_lays_out_as_its_child() {
    let placed = || {
        swatch(
            LIT,
            PropsData {
                inset_left: 7.0,
                inset_top: 9.0,
                ..PropsData::default()
            },
        )
    };
    let rect_of = |child: Node| {
        let tree = Node::Column(
            PropsData::default(),
            vec![swatch(SHADED, PropsData::default()), child],
        );
        let mut renderer = KeyRecordingRenderer::default();
        render_once(&mut SlotState::default(), &mut renderer, &tree);
        renderer
            .fills
            .iter()
            .find(|(_, c, _)| *c == LIT)
            .map(|(rect, _, _)| (rect.x, rect.y, rect.w, rect.h))
            .expect("BUG: the placed swatch must paint")
    };
    assert_eq!(rect_of(super::dimmed(0.5, placed())), rect_of(placed()));
}
