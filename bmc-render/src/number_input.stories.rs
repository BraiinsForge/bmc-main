// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "NumberInput" }

#[story(default)]
fn examples(ctx: &mut StoryCtx) -> Node {
    let value = ctx.slider("Value", 25.0, 0.0, 100.0);
    let key = ctx.action("Change");
    ctx.bind(&key, "_plus", value.nudge(1.0));
    ctx.bind(&key, "_minus", value.nudge(-1.0));

    col(
        props!(padding: 16, gap: 24),
        [
            number_input!(&key, &value, label: "Temperature", suffix: "°C", min: 0, max: 100),
            number_input!(&key, &value, label: "Disabled", suffix: "min", min: 1, max: 60, disabled: true),
            number_input!(&key, &value, label: "Normal", suffix: "V", min: 0, max: 100),
            number_input!(&key, &value, label: "Warning", suffix: "W", min: 0, max: 100, warning: "Value is high"),
            number_input!(&key, &value, label: "Error", suffix: "%", min: 0, max: 100, error: "Exceeds maximum"),
        ],
    )
}
