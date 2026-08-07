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

const STAR: Svg = include_svg!("bmc-wasm-runtime/sdk/assets/icons/star.svg");
const MULTI_PATH: Svg = include_svg!("bmc-render/assets/stories/multi-path.svg");

scene_meta! { title: "Components / Canvas / Icons" }

#[scene(default)]
fn builtin(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Built-in Icons");
    ui.label("Draw::svg_builtin with host-bundled icon IDs");

    ctx.node_stage(ui, (400_u32, 80_u32), || {
        canvas(
            props!(width: 400, height: 80),
            [
                Draw::svg_builtin(10.0, 10.0, 24.0, 24.0, ICON_CLOSE, WHITE),
                Draw::svg_builtin(50.0, 10.0, 24.0, 24.0, ICON_PLUS, GREEN_50),
                Draw::svg_builtin(90.0, 10.0, 24.0, 24.0, ICON_MINUS, RED_50),
                Draw::svg_builtin(130.0, 10.0, 24.0, 24.0, ICON_WARNING, YELLOW_30),
                Draw::svg_builtin(170.0, 10.0, 24.0, 24.0, ICON_ERROR, RED_50),
                Draw::svg_builtin(210.0, 10.0, 24.0, 24.0, ICON_SUCCESS, GREEN_50),
                Draw::svg_builtin(250.0, 10.0, 24.0, 24.0, ICON_INFO, BLUE_50),
                Draw::svg_builtin(290.0, 10.0, 24.0, 24.0, ICON_METER, ORANGE_50),
                // Same row, smaller
                Draw::svg_builtin(10.0, 48.0, 16.0, 16.0, ICON_CLOSE, GRAY_50),
                Draw::svg_builtin(34.0, 48.0, 16.0, 16.0, ICON_PLUS, GRAY_50),
                Draw::svg_builtin(58.0, 48.0, 16.0, 16.0, ICON_MINUS, GRAY_50),
                Draw::svg_builtin(82.0, 48.0, 16.0, 16.0, ICON_WARNING, GRAY_50),
                Draw::svg_builtin(106.0, 48.0, 16.0, 16.0, ICON_ERROR, GRAY_50),
                Draw::svg_builtin(130.0, 48.0, 16.0, 16.0, ICON_SUCCESS, GRAY_50),
                Draw::svg_builtin(154.0, 48.0, 16.0, 16.0, ICON_INFO, GRAY_50),
                Draw::svg_builtin(178.0, 48.0, 16.0, 16.0, ICON_METER, GRAY_50),
            ],
        )
    });
}

#[scene]
fn custom(ctx: &mut SceneCtx, ui: &mut Ui) {
    let size = ctx.slider("Size", 32.0, 32.0, 64.0, 1.0);

    ui.heading("Custom SVG Icon");
    ui.label("Draw::svg with include_svg! + ensure_registered");
    ctx.node_stage(ui, (300_u32, 60_u32), || {
        // Registered in here, where the registrars are live: a scene rendered
        // before any stage has drawn has nothing to register against.
        let star_id = ensure_registered(&STAR);
        canvas(
            props!(width: 300, height: 60),
            [
                Draw::svg_builtin(10.0, 10.0, size, size, star_id, ORANGE_50),
                Draw::svg_builtin(50.0, 10.0, size, size, star_id, YELLOW_30),
                Draw::svg_builtin(90.0, 10.0, size, size, star_id, WHITE),
                Draw::svg_builtin(150.0, 10.0, size, size, star_id, ORANGE_50).with_anti_alias(),
            ],
        )
    });

    ui.label("Last icon uses anti alias.");
}

#[scene]
fn fill_by_id(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Fill by SVG path id");
    ui.label("Override individual `<path id=\"…\">` colours per draw");

    let size = 80.0;
    ctx.node_stage(ui, (520_u32, 100_u32), || {
        canvas(
            props!(width: 520, height: 100),
            [
                // No overrides — SVG's stored greys flow through.
                Draw::svg(10.0, 10.0, size, size, &MULTI_PATH, TRANSPARENT),
                // Whole-icon tint via the existing `color` arg.
                Draw::svg(130.0, 10.0, size, size, &MULTI_PATH, BLUE_50),
                // Single path recoloured; the others keep their SVG colours.
                Draw::svg(250.0, 10.0, size, size, &MULTI_PATH, TRANSPARENT)
                    .fill("inner-dot", RED_50),
                // Layered overrides chain naturally.
                Draw::svg(370.0, 10.0, size, size, &MULTI_PATH, TRANSPARENT)
                    .fill("outer-ring", GREEN_50)
                    .fill("middle-ring", YELLOW_30)
                    .fill("inner-dot", ORANGE_50),
            ],
        )
    });
}
