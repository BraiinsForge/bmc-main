// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

const STAR: Icon = include_icon!("bmc-wasm-runtime/sdk/assets/icons/star.svg");

story_meta! { title: "Canvas/Icons" }

#[story(default)]
fn builtin(c: &mut StoryCtx) {
    c.ui.header(
        "Built-in Icons",
        "Draw::icon_builtin with host-bundled icon IDs",
    );

    c.ui.div(
        (400, 80),
        canvas(
            props!(width: 400, height: 80),
            [
                Draw::icon_builtin(10.0, 10.0, 24.0, 24.0, ICON_CLOSE, WHITE),
                Draw::icon_builtin(50.0, 10.0, 24.0, 24.0, ICON_PLUS, GREEN_50),
                Draw::icon_builtin(90.0, 10.0, 24.0, 24.0, ICON_MINUS, RED_50),
                Draw::icon_builtin(130.0, 10.0, 24.0, 24.0, ICON_WARNING, YELLOW_30),
                Draw::icon_builtin(170.0, 10.0, 24.0, 24.0, ICON_ERROR, RED_50),
                Draw::icon_builtin(210.0, 10.0, 24.0, 24.0, ICON_SUCCESS, GREEN_50),
                Draw::icon_builtin(250.0, 10.0, 24.0, 24.0, ICON_INFO, BLUE_50),
                Draw::icon_builtin(290.0, 10.0, 24.0, 24.0, ICON_METER, ORANGE_50),
                // Same row, smaller
                Draw::icon_builtin(10.0, 48.0, 16.0, 16.0, ICON_CLOSE, GRAY_50),
                Draw::icon_builtin(34.0, 48.0, 16.0, 16.0, ICON_PLUS, GRAY_50),
                Draw::icon_builtin(58.0, 48.0, 16.0, 16.0, ICON_MINUS, GRAY_50),
                Draw::icon_builtin(82.0, 48.0, 16.0, 16.0, ICON_WARNING, GRAY_50),
                Draw::icon_builtin(106.0, 48.0, 16.0, 16.0, ICON_ERROR, GRAY_50),
                Draw::icon_builtin(130.0, 48.0, 16.0, 16.0, ICON_SUCCESS, GRAY_50),
                Draw::icon_builtin(154.0, 48.0, 16.0, 16.0, ICON_INFO, GRAY_50),
                Draw::icon_builtin(178.0, 48.0, 16.0, 16.0, ICON_METER, GRAY_50),
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
        "Draw::icon with include_icon! + ensure_registered",
    );
    c.ui.div(
        (300, 60),
        canvas(
            props!(width: 300, height: 60),
            [
                Draw::icon_builtin(10.0, 10.0, size, size, star_id, ORANGE_50),
                Draw::icon_builtin(50.0, 10.0, size, size, star_id, YELLOW_30),
                Draw::icon_builtin(90.0, 10.0, size, size, star_id, WHITE),
                Draw::icon_builtin(150.0, 10.0, size, size, star_id, ORANGE_50).with_anti_alias(),
            ],
        ),
    );

    c.ui.prose("Last icon uses anti alias.");
}
