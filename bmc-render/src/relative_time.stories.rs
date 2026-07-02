// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "RelativeTimeLive" }

#[story(default)]
fn relative_time(ctx: &mut StoryCtx) -> Node {
    let age = ctx.slider("Age (s)", 90.0, 0.0, 200_000.0);
    let future = ctx.toggle("Countdown (in …)", false);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "slider seconds are small whole numbers"
    )]
    let secs = age.get() as i64;
    // Storybook clock is 0, so the anchor is placed `secs` on either side of it.
    let anchor = SystemTime {
        unix_secs: if future.get() { secs } else { -secs },
    };
    col(
        props!(padding: 24),
        [relative_time_live(
            anchor,
            RelTimeFormat::Short,
            TextStyle {
                size: 24,
                color: ORANGE_40,
                ..Default::default()
            },
        )],
    )
}
