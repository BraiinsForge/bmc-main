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

scene_meta! { title: "Components / Canvas / QR" }

fn qr_tile(label: &str, draw: Draw, size: f32) -> Node {
    col(
        props!(gap: 8, cross_align: CrossAlign::Center),
        [
            canvas(props!(width: size, height: size), [draw]),
            text(label, style!(size: 12, color: GRAY_50)),
        ],
    )
}

#[scene(default)]
fn styles(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("QR code");
    ui.label("Draw::qr — the host encodes the text and rasterises it");
    let content = "https://deck.local/setup";
    let size = 200.0;
    ctx.node_stage(ui, (720_u32, 300_u32), || {
        row(
            props!(gap: 32, padding: 24, cross_align: CrossAlign::Center),
            [
                qr_tile(
                    "black on white",
                    Draw::qr(0.0, 0.0, size, content, QrStyle::default()),
                    size,
                ),
                qr_tile(
                    "tinted, wide quiet zone",
                    Draw::qr(
                        0.0,
                        0.0,
                        size,
                        content,
                        QrStyle {
                            dark: BLUE_60,
                            light: WHITE,
                            quiet_zone: 4,
                        },
                    ),
                    size,
                ),
                qr_tile(
                    "light on transparent",
                    Draw::qr(
                        0.0,
                        0.0,
                        size,
                        content,
                        QrStyle {
                            dark: WHITE,
                            light: TRANSPARENT,
                            quiet_zone: 2,
                        },
                    ),
                    size,
                ),
            ],
        )
    });
}
