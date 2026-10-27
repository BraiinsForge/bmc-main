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

//! The static-half key the decoder derives from encoded trees,
//! and the one a tree built on the host gets.

use bmc_render::partition::tree_static_key;
use bmc_render::tree::deserialize_tree_keyed;
use bmc_ui::{
    AnimProperty, Color, Draw, Easing, LoopMode, Node, ProgressMode, PropsData, RelTimeClamp,
    RelTimeFormat, SystemTime, TextStyle, canvas, col, dimmed, encode, modal, progress_bar,
    relative_time_live, spacer, text,
};

fn key(node: &Node) -> u64 {
    deserialize_tree_keyed(&encode(node))
        .expect("BUG: the host decodes every tree the codec encodes")
        .1
}

fn square(x: f32) -> Draw {
    Draw::rect(x, 10.0, 20.0, 20.0, Color::from_rgb(1, 2, 3))
}

fn spinning(draw: Draw) -> Draw {
    draw.animate(
        AnimProperty::Rotate,
        0.0,
        360.0,
        1000,
        Easing::Linear,
        LoopMode::Forever,
    )
}

fn label(content: &str) -> Node {
    text(content, TextStyle::default())
}

fn meter(fraction: f32) -> Node {
    progress_bar(
        None,
        6.0,
        ProgressMode::Meter(fraction),
        false,
        Color::from_rgb(1, 2, 3),
        Color::from_rgb(4, 5, 6),
        Color::default(),
        None,
    )
}

fn clock(anchor: i64) -> Node {
    relative_time_live(
        SystemTime { unix_secs: anchor },
        RelTimeFormat::try_from(0).expect("BUG: 0 is a valid format"),
        RelTimeClamp::default(),
        TextStyle::default(),
    )
}

#[test]
fn a_changed_draw_in_the_layer_moves_the_key() {
    assert_ne!(
        key(&canvas(PropsData::default(), [square(1.0)])),
        key(&canvas(PropsData::default(), [square(2.0)])),
    );
}

#[test]
fn a_changed_animated_draw_holds_the_key() {
    assert_eq!(
        key(&canvas(
            PropsData::default(),
            [square(0.0), spinning(square(1.0))]
        )),
        key(&canvas(
            PropsData::default(),
            [square(0.0), spinning(square(2.0))]
        )),
    );
}

#[test]
fn a_changed_draw_above_the_dynamic_half_holds_the_key() {
    // Repainted with the dynamic half, so the layer never held it.
    assert_eq!(
        key(&canvas(
            PropsData::default(),
            [spinning(square(0.0)), square(1.0)]
        )),
        key(&canvas(
            PropsData::default(),
            [spinning(square(0.0)), square(2.0)]
        )),
    );
}

#[test]
fn another_dynamic_draw_holds_the_key() {
    assert_eq!(
        key(&canvas(
            PropsData::default(),
            [square(0.0), spinning(square(1.0))]
        )),
        key(&canvas(
            PropsData::default(),
            [square(0.0), spinning(square(1.0)), spinning(square(2.0))]
        )),
        "the draw count covers the dynamic draws too"
    );
}

#[test]
fn a_static_draw_moved_to_another_canvas_moves_the_key() {
    let empty = || canvas(PropsData::default(), []);
    let one = || canvas(PropsData::default(), [square(0.0)]);
    assert_ne!(
        key(&col(PropsData::default(), [one(), empty()])),
        key(&col(PropsData::default(), [empty(), one()])),
    );
}

#[test]
fn changed_text_moves_the_key() {
    assert_ne!(key(&label("12.5 PH/s")), key(&label("12.6 PH/s")));
}

#[test]
fn a_host_driven_node_holds_the_key_of_its_static_siblings() {
    assert_eq!(
        key(&col(
            PropsData::default(),
            [label("Last share"), clock(100)]
        )),
        key(&col(
            PropsData::default(),
            [label("Last share"), clock(200)]
        )),
    );
}

#[test]
fn a_modal_body_is_dynamic_with_the_modal() {
    let with_body = |content: &str| {
        col(
            PropsData::default(),
            [
                label("Pool"),
                modal("details", true, "Details", vec![label(content)], None),
            ],
        )
    };
    assert_eq!(key(&with_body("first")), key(&with_body("second")));
}

#[test]
fn a_static_node_beside_a_modal_moves_the_key() {
    let beside = |content: &str| {
        col(
            PropsData::default(),
            [
                label(content),
                modal("details", true, "Details", vec![clock(100)], None),
            ],
        )
    };
    assert_ne!(key(&beside("first")), key(&beside("second")));
}

#[test]
fn a_progress_value_holds_the_key() {
    assert_eq!(
        key(&col(PropsData::default(), [label("Volume"), meter(0.25)])),
        key(&col(PropsData::default(), [label("Volume"), meter(0.75)])),
    );
}

#[test]
fn a_dynamic_node_changing_kind_moves_the_key() {
    let between_labels = |dynamic: Node| {
        col(
            PropsData::default(),
            [label("above"), dynamic, label("below")],
        )
    };
    assert_ne!(
        key(&between_labels(meter(0.5))),
        key(&between_labels(modal(
            "details",
            false,
            "Details",
            Vec::new(),
            None
        ))),
        "a progress bar takes height where a modal takes none, so the labels move"
    );
}

#[test]
fn a_canvas_gaining_or_losing_its_dynamic_draw_moves_the_key() {
    // The overlay paints above the animation only while there is one,
    // so the layer holds it in one tree and lacks it in the other.
    let over = PropsData {
        inset_top: 0.0,
        inset_left: 0.0,
        ..PropsData::default()
    };
    let overlaid = |under: Vec<Draw>| {
        col(
            PropsData::default(),
            [
                canvas(PropsData::default(), under),
                canvas(over, [square(5.0)]),
            ],
        )
    };
    assert_ne!(
        key(&overlaid(vec![spinning(square(0.0))])),
        key(&overlaid(Vec::new())),
    );
}

/// Static and dynamic content side by side, through every kind of dynamic split:
/// host-driven nodes, a modal's body, and a canvas banded around its animation.
fn mixed(title: &str, progress: f32, anchor: i64, spin_x: f32) -> Node {
    col(
        PropsData::default(),
        [
            label(title),
            meter(progress),
            dimmed(0.5, clock(anchor)),
            canvas(
                PropsData::default(),
                [square(0.0), spinning(square(spin_x)), square(3.0)],
            ),
            modal("details", true, "Details", vec![label(title)], None),
        ],
    )
}

fn host_key(node: &Node) -> u64 {
    tree_static_key(node).expect("BUG: the host decodes every tree the codec encodes")
}

#[test]
fn a_host_built_tree_keys_as_its_wire_bytes() {
    let tree = mixed("Upgrade", 0.25, 100, 1.0);
    assert_eq!(host_key(&tree), key(&tree));
}

#[test]
fn a_host_built_tree_holds_its_key_when_only_dynamic_content_moves() {
    assert_eq!(
        host_key(&mixed("Upgrade", 0.25, 100, 1.0)),
        host_key(&mixed("Upgrade", 0.75, 200, 2.0)),
    );
}

#[test]
fn a_host_built_tree_moves_its_key_on_a_static_edit() {
    assert_ne!(
        host_key(&mixed("Upgrade", 0.25, 100, 1.0)),
        host_key(&mixed("Upgrading", 0.25, 100, 1.0)),
    );
}

#[test]
fn a_host_built_tree_the_decoder_refuses_has_no_key() {
    assert_eq!(tree_static_key(&spacer(-1.0)), None);
}

#[test]
fn a_host_built_tree_past_the_wire_format_has_no_key() {
    let past_a_length_prefix = "x".repeat(usize::from(u16::MAX) + 1);
    assert_eq!(tree_static_key(&label(&past_a_length_prefix)), None);
}
