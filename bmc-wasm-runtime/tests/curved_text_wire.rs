// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_render::tree::{DrawCommand, TreeNode};
use bmc_wasm_protocol::{ArcAnchor, ArcTextFacing, Color, FontWeight, PropsData, TextStyle};
use bmc_wasm_sdk::{Draw, canvas, tree::serialize_node_to_bytes};

#[test]
fn sdk_curved_text_decodes_on_the_host_for_each_anchor_and_facing() {
    let style = TextStyle {
        size: 20,
        color: Color::from_rgb(0xAA, 0xBB, 0xCC),
        weight: FontWeight::BOLD,
        ..TextStyle::default()
    };
    let draws = [
        Draw::curved_text(
            195.0,
            195.0,
            120.0,
            0.0,
            ArcAnchor::Start,
            ArcTextFacing::Outward,
            "START",
            style,
        ),
        Draw::curved_text(
            195.0,
            195.0,
            120.0,
            1.0,
            ArcAnchor::Center,
            ArcTextFacing::Inward,
            "CENTER",
            style,
        ),
        Draw::curved_text(
            195.0,
            195.0,
            120.0,
            2.0,
            ArcAnchor::End,
            ArcTextFacing::Outward,
            "END",
            style,
        ),
    ];

    let bytes = serialize_node_to_bytes(&canvas(PropsData::default(), draws));
    let TreeNode::Canvas { draws, .. } =
        bmc_render::deserialize_tree(&bytes).expect("BUG: SDK must serialize a valid canvas tree")
    else {
        panic!("expected canvas root");
    };

    assert_eq!(draws.len(), 3);
    let DrawCommand::CurvedText {
        anchor,
        facing,
        text,
        style: decoded_style,
        ..
    } = &draws[0]
    else {
        panic!("expected curved text 0");
    };
    assert_eq!(
        (*anchor, *facing, text.as_str(), decoded_style.weight),
        (
            ArcAnchor::Start,
            ArcTextFacing::Outward,
            "START",
            FontWeight::BOLD,
        ),
    );
    let DrawCommand::CurvedText {
        anchor,
        facing,
        text,
        ..
    } = &draws[1]
    else {
        panic!("expected curved text 1");
    };
    assert_eq!(
        (*anchor, *facing, text.as_str()),
        (ArcAnchor::Center, ArcTextFacing::Inward, "CENTER"),
    );
    let DrawCommand::CurvedText {
        anchor,
        facing,
        text,
        ..
    } = &draws[2]
    else {
        panic!("expected curved text 2");
    };
    assert_eq!(
        (*anchor, *facing, text.as_str()),
        (ArcAnchor::End, ArcTextFacing::Outward, "END"),
    );
}
