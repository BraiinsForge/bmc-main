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

#[test]
fn emit_mode_partitions_nodes_without_overlap_or_gap() {
    for dynamic in [false, true] {
        assert!(
            EmitMode::All.emits(dynamic),
            "the default mode must emit everything"
        );
        assert_ne!(
            EmitMode::StaticOnly.emits(dynamic),
            EmitMode::DynamicOnly.emits(dynamic),
            "static and dynamic passes must not overlap or leave a gap"
        );
    }
    assert!(EmitMode::StaticOnly.emits(false));
    assert!(EmitMode::DynamicOnly.emits(true));
}

/// After a fractional scroll, a row's top can land a float step
/// above the bottom of the dynamic row before it.
#[test]
fn the_dynamic_half_ignores_an_overlap_no_pixel_shows() {
    let mut painted = DynamicHalf::default();
    let row = Rect::new(0.0, 10.3, 100.0, 33.0);
    painted.add(row);
    let bottom = row.y + row.h;
    let next_top = f32::from_bits(bottom.to_bits() - 1);
    assert!(
        !painted.covers(Rect::new(0.0, next_top, 100.0, 33.0)),
        "a row merely touching the dynamic one belongs in the layer"
    );
    assert!(
        painted.covers(Rect::new(0.0, bottom - 1.0, 100.0, 33.0)),
        "a whole pixel of overlap still bands the row above"
    );
}

/// Two strips meeting in a corner have the whole surface as their bounding box,
/// yet neither reaches the opposite corner.
#[test]
fn the_dynamic_half_covers_only_what_was_painted() {
    let mut painted = DynamicHalf::default();
    painted.add(Rect::new(0.0, 0.0, 100.0, 10.0));
    painted.add(Rect::new(0.0, 0.0, 10.0, 100.0));
    assert!(
        !painted.covers(Rect::new(80.0, 80.0, 10.0, 10.0)),
        "a node clear of both strips belongs in the layer"
    );
}

/// The tag must survive the trip into `NodeContext`, which is all the
/// render walk ever sees of it.
#[test]
fn build_taffy_node_tags_context_with_the_partition_classification() {
    fn build(node: &TreeNode) -> (TaffyTree<NodeContext>, taffy::NodeId) {
        let mut taffy = TaffyTree::new();
        let mut result = TreeResult::default();
        let mut modals = Vec::new();
        let id = build_taffy_node(&mut taffy, node, 0, &mut result, &mut modals)
            .expect("BUG: taffy build failed");
        (taffy, id)
    }

    let static_canvas = TreeNode::Canvas {
        props: PropsData::default(),
        touch_key: None,
        draws: vec![DrawCommand::Rect {
            x: 0.0,
            y: 0.0,
            w: 4.0,
            h: 4.0,
            fill: Fill::Solid(Color::from_rgb(1, 2, 3)),
        }],
    };
    let (taffy, id) = build(&static_canvas);
    assert!(
        !taffy
            .get_node_context(id)
            .expect("BUG: canvas must own a context")
            .dynamic,
        "a canvas of plain draws must be tagged static"
    );

    let dynamic_tree = TreeNode::Column(
        PropsData::default(),
        vec![TreeNode::RelTime {
            anchor: 0,
            format: RelTimeFormat {
                length: RelTimeLength::Short,
                segments: RelTimeSegments::Single,
            },
            clamp: RelTimeClamp::default(),
            style: TextStyle::default(),
        }],
    );
    let (taffy, id) = build(&dynamic_tree);
    assert!(
        taffy
            .get_node_context(id)
            .expect("BUG: every container must own a context to carry the tag")
            .dynamic,
        "a container holding a host-driven node must be tagged dynamic"
    );
}
