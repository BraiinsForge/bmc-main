// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Animation/Transitions" }

#[story(default)]
fn position(c: &mut StoryCtx) {
    let x = c.slider("X position", 0.0, 0.0, 250.0);

    c.ui.header(
        "Position Transition",
        "Drag the slider — the rect smoothly follows",
    );

    c.ui.div(
        (300, 60),
        canvas(
            props!(width: 300, height: 60),
            [
                Draw::rect(x.get(), 14.0, 32.0, 32.0, BLUE_50)
                    .transition(500, Easing::EaseOutCubic),
            ],
        ),
    );
}

#[story]
fn color(c: &mut StoryCtx) {
    let toggle = c.toggle("Alternate color", false);
    let color = if toggle.get() { GREEN_50 } else { RED_50 };

    c.ui.header(
        "Color Transition",
        "Toggle the color — smooth interpolation in Oklab",
    );

    c.ui.div(
        (300, 120),
        canvas(
            props!(width: 300, height: 120),
            [Draw::rect(126.0, 36.0, 48.0, 48.0, color).transition(800, Easing::EaseInOut)],
        ),
    );
}

#[story]
fn size(c: &mut StoryCtx) {
    let size = c.slider("Size", 32.0, 16.0, 80.0);
    let s = size.get();

    c.ui.header("Size Transition", "Drag the slider — smooth resize");

    c.ui.div(
        (300, 140),
        canvas(
            props!(width: 300, height: 140),
            [Draw::rect(150.0 - s / 2.0, 70.0 - s / 2.0, s, s, VIOLET_50)
                .transition(300, Easing::EaseOut)],
        ),
    );
}
