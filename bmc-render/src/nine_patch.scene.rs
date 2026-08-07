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

const PANEL: NinePatchAsset = include_nine_patch!("bmc-render/assets/panel.9.png");
const BUBBLE: NinePatchAsset = include_nine_patch!("bmc-render/assets/bubble.9.png");
const BUTTON: NinePatchAsset = include_nine_patch!("bmc-render/assets/button_normal.9.png");

scene_meta! { title: "Components / Canvas / Nine-Patch" }

#[scene(default)]
fn demo(ctx: &mut SceneCtx, ui: &mut Ui) {
    let pw = 300.0;
    let ph = 200.0;

    ui.heading("Panel");
    ui.label("rounded panel — corners stay fixed, edges and center stretch");

    ctx.node_stage(ui, (300_u32, 200_u32), || {
        canvas(
            props!(width: pw, height: ph),
            [
                Draw::nine_patch(10, 10, pw / 2.0 - 15.0, ph - 20.0, &PANEL),
                Draw::nine_patch(pw / 2.0 + 5.0, 10, pw / 2.0 - 15.0, ph / 2.0 - 15.0, &PANEL),
                Draw::nine_patch(
                    pw / 2.0 + 5.0,
                    ph / 2.0 + 5.0,
                    pw / 2.0 - 15.0,
                    ph / 2.0 - 15.0,
                    &PANEL,
                ),
            ],
        )
    });

    ui.heading("Speech Bubble");
    ui.label("asymmetric nine-patch with a tail");

    ctx.node_stage(ui, (300_u32, 200_u32), || {
        canvas(
            props!(width: pw, height: ph),
            [Draw::nine_patch(10, 10, pw - 20.0, ph - 20.0, &BUBBLE)],
        )
    });

    ui.heading("Button");
    ui.label("pixel-art button with thick corners");

    ctx.node_stage(ui, (300_u32, 60_u32), || {
        canvas(
            props!(width: 300, height: 60),
            [Draw::nine_patch(10, 10, 280, 40, &BUTTON)],
        )
    });
}
