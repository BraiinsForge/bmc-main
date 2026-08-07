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

scene_meta! { title: "Components / Canvas / Fills" }

#[scene(default)]
fn paints_on_shapes(ctx: &mut SceneCtx, ui: &mut Ui) {
    let top = TEAL_50;
    let bottom = TEAL_50.with_alpha(0.0);
    ctx.node_stage(ui, (320_u32, 220_u32), || {
        canvas(
            props!(width: 320, height: 220),
            [
                Draw::rect(10.0, 10.0, 90.0, 60.0, VIOLET_60),
                Draw::rect(115.0, 10.0, 90.0, 60.0, Fill::linear(0.0, top, bottom)),
                Draw::rect(220.0, 10.0, 90.0, 60.0, Fill::radial(top, bottom)),
                Draw::circle(55.0, 130.0, 35.0, RED_50),
                Draw::circle(160.0, 130.0, 35.0, Fill::linear(90.0, top, bottom)),
                Draw::circle(265.0, 130.0, 35.0, Fill::radial(top, bottom)),
            ],
        )
    });
}

const CHART_W: f32 = 1000.0;
const CHART_H: f32 = 240.0;
const CHART_POINTS: usize = 200;

/// Deterministic volatile price walk, mapped to the chart box and trending up
/// (`rising`) or down. A seeded xorshift keeps the rendered scene stable.
fn price_series(seed: u32, rising: bool) -> Vec<(f32, f32)> {
    let (start_y, end_y) = if rising {
        (CHART_H * 0.85, CHART_H * 0.18)
    } else {
        (CHART_H * 0.25, CHART_H * 0.82)
    };
    let mut state = seed | 1;
    let mut rand = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        px(state >> 8) / px(1 << 24)
    };
    let mut wander = 0.0_f32;
    let mut points = Vec::with_capacity(CHART_POINTS);
    for i in 0..CHART_POINTS {
        let t = px(idx(i)) / px(idx(CHART_POINTS - 1));
        let trend = start_y + (end_y - start_y) * t;
        wander = wander * 0.9 + (rand() - 0.5) * CHART_H * 0.12;
        let noise = (rand() - 0.5) * CHART_H * 0.06;
        let y = (trend + wander + noise).clamp(CHART_H * 0.05, CHART_H * 0.95);
        points.push((t * CHART_W, y));
    }
    points
}

fn area_card(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    color: Color,
    top_alpha: f32,
    rising: bool,
    seed: u32,
) {
    let trend = price_series(seed, rising);
    let mut area = trend.clone();
    area.push((CHART_W, CHART_H));
    area.push((0.0, CHART_H));
    ctx.node_stage(ui, (1000_u32, 240_u32), || {
        canvas(
            props!(width: 1000, height: 240),
            [
                fill!(area, linear: (color.with_alpha(top_alpha), color.with_alpha(0.0)), smooth),
                path!(trend, stroke: 2.0, color: color, smooth),
            ],
        )
    });
}

#[scene]
fn area_chart_up(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("BTC-USD up");
    ui.label("Rising trend, #34C06A area 16% to 0%");
    area_card(
        ctx,
        ui,
        Color::from_rgb(0x34, 0xC0, 0x6A),
        0.16,
        true,
        0x1234_5678,
    );
}

#[scene]
fn area_chart_down(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("BTC-USD down");
    ui.label("Falling trend, #F95355 area 30% to 0%");
    area_card(
        ctx,
        ui,
        Color::from_rgb(0xF9, 0x53, 0x55),
        0.30,
        false,
        0x0BAD_F00D,
    );
}
