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

scene_meta! { title: "Components / Animation / Transitions" }

#[scene(default)]
fn position(ctx: &mut SceneCtx, ui: &mut Ui) {
    let x = ctx.slider("X position", 0.0, 0.0, 250.0, 1.0);

    ui.heading("Position Transition");
    ui.label("Drag the slider — the rect smoothly follows");

    ctx.node_stage(ui, (300_u32, 60_u32), || {
        canvas(
            props!(width: 300, height: 60),
            [Draw::rect(x, 14.0, 32.0, 32.0, BLUE_50).transition(
                "rect",
                500,
                Easing::EaseOutCubic,
            )],
        )
    });
}

#[scene]
fn color(ctx: &mut SceneCtx, ui: &mut Ui) {
    let toggle = ctx.toggle("Alternate color", false);
    let color = if toggle { GREEN_50 } else { RED_50 };

    ui.heading("Color Transition");
    ui.label("Toggle the color — smooth interpolation in Oklab");

    ctx.node_stage(ui, (300_u32, 120_u32), || {
        canvas(
            props!(width: 300, height: 120),
            [
                Draw::rect(126.0, 36.0, 48.0, 48.0, color).transition(
                    "rect",
                    800,
                    Easing::EaseInOut,
                ),
            ],
        )
    });
}

#[scene]
fn size(ctx: &mut SceneCtx, ui: &mut Ui) {
    let s = ctx.slider("Size", 32.0, 16.0, 80.0, 1.0);

    ui.heading("Size Transition");
    ui.label("Drag the slider — smooth resize");

    ctx.node_stage(ui, (300_u32, 140_u32), || {
        canvas(
            props!(width: 300, height: 140),
            [
                Draw::rect(150.0 - s / 2.0, 70.0 - s / 2.0, s, s, VIOLET_50).transition(
                    "rect",
                    300,
                    Easing::EaseOut,
                ),
            ],
        )
    });
}
