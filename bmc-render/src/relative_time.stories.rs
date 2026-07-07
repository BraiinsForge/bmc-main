// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "RelativeTimeLive" }

#[story(default)]
fn relative_time(ctx: &mut StoryCtx) {
    let age = ctx.slider("Age (s)", 90.0, 0.0, 200_000.0);
    let future = ctx.toggle("Countdown (in …)", false);

    let length = match ctx.radio("Length", &["Short", "Long"], 0).get() {
        1 => RelTimeLength::Long,
        _ => RelTimeLength::Short,
    };

    let segments = match ctx.radio("Segments", &["Single", "Double"], 0).get() {
        1 => RelTimeSegments::Double,
        _ => RelTimeSegments::Single,
    };

    #[expect(
        clippy::cast_possible_truncation,
        reason = "slider seconds are small whole numbers"
    )]
    let secs = age.get() as i64;

    // Storybook clock is 0, so the anchor is placed `secs` on either side of it.
    let anchor = SystemTime {
        unix_secs: if future.get() { secs } else { -secs },
    };
    let format = RelTimeFormat { length, segments };

    ctx.ui.div(
        (240, 120),
        center(
            props!(flex: 1.0),
            [relative_time_live(
                anchor,
                format,
                TextStyle {
                    size: 24,
                    color: ORANGE_40,
                    ..Default::default()
                },
            )],
        ),
    );
}
