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

use super::brightness_tests::{FULL, W, render};
use crate::renderer::Renderer;
use bmc_wasm_protocol::colors::{BLACK, WHITE};
use bmc_wasm_protocol::{Color, SVG_FLAG_HAS_FILL, SVG_OP_CLOSE, SVG_OP_LINE_TO, SVG_OP_MOVE_TO};

/// The problem icon's arcs, as the svg compiler stores them.
const ARC_ALPHA: u8 = 102;

/// Mid-height, inside the left square and inside the right one of a full-frame draw.
const OPAQUE_PROBE: (u32, u32) = (16, 32);
const TRANSLUCENT_PROBE: (u32, u32) = (48, 32);

/// Two unit squares side by side in a 2×1 viewbox:
/// an opaque white one on the left, a translucent white one on the right.
fn half_translucent_icon() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&2.0_f32.to_le_bytes());
    buf.extend_from_slice(&1.0_f32.to_le_bytes());
    buf.extend_from_slice(&2_u16.to_le_bytes());
    for (left, color) in [
        (0.0_f32, WHITE),
        (1.0, Color::from_rgba(255, 255, 255, ARC_ALPHA)),
    ] {
        buf.push(SVG_FLAG_HAS_FILL);
        buf.extend_from_slice(&color.to_u32().to_le_bytes());
        buf.extend_from_slice(&5_u16.to_le_bytes());
        for (op, x, y) in [
            (SVG_OP_MOVE_TO, left, 0.0_f32),
            (SVG_OP_LINE_TO, left + 1.0, 0.0),
            (SVG_OP_LINE_TO, left + 1.0, 1.0),
            (SVG_OP_LINE_TO, left, 1.0),
        ] {
            buf.push(op);
            buf.extend_from_slice(&x.to_le_bytes());
            buf.extend_from_slice(&y.to_le_bytes());
        }
        buf.push(SVG_OP_CLOSE);
    }
    buf
}

/// A held button tints its icon black over its white circle.
/// The translucent path must stay translucent, or the icon changes shape under the finger.
#[test]
fn a_tint_keeps_each_paths_own_alpha() {
    let (x, y, w, h) = FULL;
    let px = render(|r| {
        let icon = r
            .register_svg("half-translucent", &half_translucent_icon())
            .expect("BUG: the two-square icon must parse");
        r.fill_rect(x, y, w, h, WHITE);
        r.draw_svg(x, y, w, h, BLACK, icon, false, &[]);
    });
    let at = |(col, row): (u32, u32)| px[(row * W + col) as usize];
    assert_eq!(
        at(OPAQUE_PROBE)[..3],
        [0, 0, 0],
        "the opaque path takes the tint whole"
    );
    let see_through = 255 - u32::from(ARC_ALPHA);
    let translucent = at(TRANSLUCENT_PROBE);
    assert!(
        translucent[..3]
            .iter()
            .all(|&c| u32::from(c).abs_diff(see_through) <= 2),
        "the translucent path lets {see_through} of the white through, got {translucent:?}"
    );
}
