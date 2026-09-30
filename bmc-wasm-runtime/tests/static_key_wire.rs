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

//! The static-half key the decoder derives from SDK-serialized trees.

use bmc_render::tree::deserialize_tree_keyed;
use bmc_wasm_protocol::{
    AnimProperty, Color, Easing, LoopMode, PropsData, RelTimeClamp, RelTimeFormat, TextStyle,
};
use bmc_wasm_sdk::host::SystemTime;
use bmc_wasm_sdk::modal::modal;
use bmc_wasm_sdk::progress_bar::{ProgressMode, progress_bar};
use bmc_wasm_sdk::relative_time::relative_time_live;
use bmc_wasm_sdk::text::{StyleResult, text};
use bmc_wasm_sdk::tree::serialize_node_to_bytes;
use bmc_wasm_sdk::{Draw, Node, canvas, col};

fn key(node: &Node) -> u64 {
    deserialize_tree_keyed(&serialize_node_to_bytes(node))
        .expect("BUG: the SDK must serialize a tree the host decodes")
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
    text(
        content,
        StyleResult(TextStyle::default(), PropsData::default(), None),
    )
}

fn meter(fraction: f32) -> Node {
    progress_bar(
        "",
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
