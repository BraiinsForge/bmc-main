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

use bmc_gallery::prelude::*;

scene_meta! { title: "Components / Layout / Scroll" }

#[scene(default)]
fn examples(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Vertical scroll (200px viewport)");
    // Takes the wheel, so the demo scrolls its own list rather than the canvas.
    ctx.node_stage_input(ui, (300_u32, 200_u32), || {
        scroll(
            "v_scroll",
            props!(background: GRAY_90),
            (0..20).map(|i| {
                text(
                    format!("Item {i}"),
                    style!(size: 14, color: GRAY_10, padding: 8),
                )
            }),
        )
    });

    ui.heading("Scroll with mixed content");
    ctx.node_stage_input(ui, (400_u32, 200_u32), || {
        scroll(
            "mixed_scroll",
            props!(background: GRAY_90),
            [
                text(
                    "Header",
                    style!(size: 18, weight: FontWeight::BOLD, color: WHITE, padding: 8),
                ),
                col(
                    props!(gap: 4, padding: 8),
                    (0..15).map(|i| {
                        row(
                            props!(gap: 8, cross_align: CrossAlign::Center),
                            [
                                canvas(
                                    props!(width: 8, height: 8),
                                    [Draw::circle(
                                        4.0,
                                        4.0,
                                        4.0,
                                        if i % 2 == 0 { GREEN_50 } else { BLUE_50 },
                                    )],
                                ),
                                text(format!("List item {i}"), style!(size: 14, color: GRAY_10)),
                            ],
                        )
                    }),
                ),
            ],
        )
    });
}
