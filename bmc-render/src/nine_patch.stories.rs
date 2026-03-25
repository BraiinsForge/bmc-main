// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

const PANEL: NinePatchAsset = include_nine_patch!("bmc-render/assets/panel.9.png");
const BUBBLE: NinePatchAsset = include_nine_patch!("bmc-render/assets/bubble.9.png");
const BUTTON: NinePatchAsset = include_nine_patch!("bmc-render/assets/button_normal.9.png");

story_meta! { title: "Canvas/Nine-Patch" }

#[story(default)]
fn demo(c: &mut StoryCtx) {
    let pw = 300.0;
    let ph = 200.0;

    c.ui.header(
        "Panel",
        "rounded panel — corners stay fixed, edges and center stretch",
    );

    c.ui.div(
        (300, 200),
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
        ),
    );

    c.ui.header("Speech Bubble", "asymmetric nine-patch with a tail");

    c.ui.div(
        (300, 200),
        canvas(
            props!(width: pw, height: ph),
            [Draw::nine_patch(10, 10, pw - 20.0, ph - 20.0, &BUBBLE)],
        ),
    );

    c.ui.header("Button", "pixel-art button with thick corners");

    c.ui.div(
        (300, 60),
        canvas(
            props!(width: 300, height: 60),
            [Draw::nine_patch(10, 10, 280, 40, &BUTTON)],
        ),
    );
}
