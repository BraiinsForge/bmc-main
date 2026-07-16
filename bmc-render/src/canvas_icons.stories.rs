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

use crate::prelude::*;

const STAR: Svg = include_svg!("bmc-wasm-runtime/sdk/assets/icons/star.svg");
const MULTI_PATH: Svg = include_svg!("bmc-render/assets/stories/multi-path.svg");

story_meta! { title: "Canvas/Icons" }

#[story(default)]
fn builtin(c: &mut StoryCtx) {
    c.ui.header(
        "Built-in Icons",
        "Draw::svg_builtin with host-bundled icon IDs",
    );

    c.ui.div(
        (400, 80),
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
        ),
    );
}

#[story]
fn custom(c: &mut StoryCtx) {
    let star_id = ensure_registered(&STAR);

    let size_knob = c.slider("Size", 32.0, 32.0, 64.0);
    let size = size_knob.get();

    c.ui.header(
        "Custom SVG Icon",
        "Draw::svg with include_svg! + ensure_registered",
    );
    c.ui.div(
        (300, 60),
        canvas(
            props!(width: 300, height: 60),
            [
                Draw::svg_builtin(10.0, 10.0, size, size, star_id, ORANGE_50),
                Draw::svg_builtin(50.0, 10.0, size, size, star_id, YELLOW_30),
                Draw::svg_builtin(90.0, 10.0, size, size, star_id, WHITE),
                Draw::svg_builtin(150.0, 10.0, size, size, star_id, ORANGE_50).with_anti_alias(),
            ],
        ),
    );

    c.ui.prose("Last icon uses anti alias.");
}

#[story]
fn fill_by_id(c: &mut StoryCtx) {
    c.ui.header(
        "Fill by SVG path id",
        "Override individual `<path id=\"…\">` colours per draw",
    );

    let size = 80.0;
    c.ui.div(
        (520, 100),
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
        ),
    );
}
