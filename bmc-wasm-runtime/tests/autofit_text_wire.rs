// Copyright (C) 2026  Braiins Systems s.r.o.

//! SDK→host round-trip for the AutofitText canvas draw command.

use bmc_render::tree::{DrawCommand, TreeNode};
use bmc_wasm_protocol::{AutoFit, PropsData, TextStyle};
use bmc_wasm_sdk::{Draw, canvas, tree::serialize_node_to_bytes};

#[test]
fn sdk_autofit_text_decodes_on_the_host() {
    let style = TextStyle {
        size: 48,
        ..TextStyle::default()
    };
    let draws = [Draw::autofit_text_ranged(
        4.0,
        8.0,
        200.0,
        80.0,
        "Many names here",
        style,
        AutoFit::Shrink,
        16,
        0,
    )];

    let bytes = serialize_node_to_bytes(&canvas(PropsData::default(), draws));
    let TreeNode::Canvas { draws, .. } =
        bmc_render::deserialize_tree(&bytes).expect("BUG: SDK must serialize a valid canvas tree")
    else {
        panic!("expected canvas root");
    };

    assert_eq!(draws.len(), 1);
    let DrawCommand::AutofitText {
        mode,
        min_size,
        max_size,
        text,
        style: decoded_style,
        ..
    } = &draws[0]
    else {
        panic!("expected AutofitText draw");
    };

    assert_eq!(*mode, AutoFit::Shrink);
    assert_eq!(*min_size, 16_u16);
    assert_eq!(*max_size, 0_u16);
    assert_eq!(text.as_str(), "Many names here");
    assert_eq!(decoded_style.size, 48);
}
