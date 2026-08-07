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

use std::f32::consts::TAU;

use bmc_gallery::prelude::*;

scene_meta! { title: "Components / Canvas / Arcs" }

const GREEN: Color = Color::from_rgb(0x34, 0xC0, 0x6A);
const TEAL: Color = Color::from_rgb(0x1F, 0xB6, 0xC1);
const YELLOW: Color = Color::from_rgb(0xF5, 0xD9, 0x0A);

#[scene(default)]
fn rings(ctx: &mut SceneCtx, ui: &mut Ui) {
    let (cx, cy) = (160.0, 160.0);
    let a0 = ctx.slider("Start angle", 2.35, 0.0, TAU, 0.01);
    let portion = ctx.slider("Circle portion", 0.75, 0.05, 1.0, 0.01);
    let a1 = a0 + TAU * portion;

    ctx.node_stage(ui, (320_u32, 320_u32), || {
        canvas(
            props!(width: 320, height: 320),
            [
                Draw::arc(
                    cx,
                    cy,
                    130.0,
                    a0,
                    a1,
                    6.0,
                    ArcFill::gradient(GREEN, TEAL),
                    ArcSegments::Continuous,
                    ArcCap::Round,
                )
                .transition("outer-ring", 500, Easing::EaseOutCubic),
                Draw::arc(
                    cx,
                    cy,
                    112.0,
                    a0,
                    a1,
                    8.0,
                    ArcFill::gradient(GREEN, YELLOW),
                    ArcSegments::short_ends(a0, a1, 24, 0.04, 0.5),
                    ArcCap::Round,
                )
                .transition("inner-segments", 500, Easing::EaseOutCubic),
            ],
        )
    });
}

#[scene]
fn uniform_gauge(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Uniform segments");
    ui.label("12 equal segments, solid teal");

    let (cx, cy) = (160.0, 160.0);
    let a0 = 2.35;
    let a1 = 7.07;

    ctx.node_stage(ui, (320_u32, 320_u32), || {
        canvas(
            props!(width: 320, height: 320),
            [Draw::arc(
                cx,
                cy,
                120.0,
                a0,
                a1,
                10.0,
                TEAL,
                ArcSegments::uniform(a0, a1, 12, 0.06),
                ArcCap::Round,
            )],
        )
    });
}
