// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Touchable" }

#[story(default)]
fn examples(ctx: &mut StoryCtx) -> Node {
    let click_key = ctx.action("Canvas clicked");

    col(
        props!(gap: 24, padding: 16),
        [
            text(
                "Touchable canvas (click the shapes)",
                style!(size: 14, color: GRAY_30),
            ),
            touchable(
                &click_key,
                props!(width: 200, height: 120, background: GRAY_90),
                [
                    Draw::Rect {
                        x: 16.0,
                        y: 16.0,
                        w: 80.0,
                        h: 88.0,
                        color: VIOLET_60,
                    },
                    Draw::Circle {
                        cx: 150.0,
                        cy: 60.0,
                        r: 40.0,
                        color: TEAL_50,
                    },
                ],
            ),
        ],
    )
}
