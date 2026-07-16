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
