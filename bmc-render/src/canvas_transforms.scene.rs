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

use core::f32::consts::{FRAC_PI_4, FRAC_PI_6, TAU};

use bmc_gallery::prelude::*;

scene_meta! { title: "Components / Canvas / Transforms" }

#[scene(default)]
fn rotated(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Rotated");
    ui.label("Draw::rotated wraps any draw command with a rotation");

    ctx.node_stage(ui, (400_u32, 100_u32), || {
        canvas(
            props!(width: 400, height: 100),
            [
                // Unrotated reference
                Draw::rect(20.0, 30.0, 40.0, 40.0, GRAY_70),
                // 15° rotation
                Draw::rotated(
                    FRAC_PI_6 / 2.0,
                    Draw::rect(100.0, 30.0, 40.0, 40.0, BLUE_50),
                ),
                // 30° rotation
                Draw::rotated(FRAC_PI_6, Draw::rect(180.0, 30.0, 40.0, 40.0, GREEN_50)),
                // 45° rotation
                Draw::rotated(FRAC_PI_4, Draw::rect(260.0, 30.0, 40.0, 40.0, VIOLET_50)),
                // Labels
                Draw::text(20.0, 78.0, "0°", style!(size: 10, color: GRAY_50)),
                Draw::text(100.0, 78.0, "15°", style!(size: 10, color: GRAY_50)),
                Draw::text(180.0, 78.0, "30°", style!(size: 10, color: GRAY_50)),
                Draw::text(260.0, 78.0, "45°", style!(size: 10, color: GRAY_50)),
            ],
        )
    });
}

#[scene]
fn centered(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Centered");
    ui.label("Draw::centered places a draw at the canvas center");

    ctx.node_stage(ui, (200_u32, 100_u32), || {
        canvas(
            props!(width: 200, height: 100),
            [
                // Crosshair to show center
                Draw::rect(99.0, 0.0, 1.0, 100.0, GRAY_80),
                Draw::rect(0.0, 49.0, 200.0, 1.0, GRAY_80),
                // Centered elements
                Draw::centered(Draw::circle(0.0, 0.0, 20.0, BLUE_50)),
                Draw::centered(Draw::rect(-30.0, -6.0, 60.0, 12.0, GREEN_50)),
            ],
        )
    });
}

#[scene]
fn orbit(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Orbit");
    ui.label("Draw::orbit positions a draw at radius+angle from canvas center");

    ctx.node_stage(ui, (200_u32, 200_u32), || {
        canvas(
            props!(width: 200, height: 200),
            [
                // Center dot
                Draw::centered(Draw::circle(0.0, 0.0, 4.0, GRAY_60)),
                // Orbit ring (approximate with circles at regular angles)
                Draw::centered(Draw::orbit(60.0, 0.0, Draw::circle(0.0, 0.0, 8.0, RED_50))),
                Draw::centered(Draw::orbit(
                    60.0,
                    TAU / 6.0,
                    Draw::circle(0.0, 0.0, 8.0, ORANGE_50),
                )),
                Draw::centered(Draw::orbit(
                    60.0,
                    TAU / 3.0,
                    Draw::circle(0.0, 0.0, 8.0, YELLOW_30),
                )),
                Draw::centered(Draw::orbit(
                    60.0,
                    TAU / 2.0,
                    Draw::circle(0.0, 0.0, 8.0, GREEN_50),
                )),
                Draw::centered(Draw::orbit(
                    60.0,
                    TAU * 2.0 / 3.0,
                    Draw::circle(0.0, 0.0, 8.0, BLUE_50),
                )),
                Draw::centered(Draw::orbit(
                    60.0,
                    TAU * 5.0 / 6.0,
                    Draw::circle(0.0, 0.0, 8.0, VIOLET_50),
                )),
            ],
        )
    });
}
