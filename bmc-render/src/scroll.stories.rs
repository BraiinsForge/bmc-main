// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Scroll" }

#[story(default)]
fn examples(c: &mut StoryCtx) {
    c.ui.header("Vertical scroll (200px viewport)", "");
    c.ui.div(
        (300, 200),
        scroll(
            "v_scroll",
            props!(background: GRAY_90),
            (0..20).map(|i| {
                text(
                    format!("Item {i}"),
                    style!(size: 14, color: GRAY_10, padding: 8),
                )
            }),
        ),
    );

    c.ui.header("Scroll with mixed content", "");
    c.ui.div(
        (400, 200),
        scroll(
            "mixed_scroll",
            props!(background: GRAY_90),
            [
                text(
                    "Header",
                    style!(size: 18, weight: FontWeight::BOLD, color: WHITE, padding: 8),
                ),
                col(
                    props!(gap: 4, padding: 8),
                    (0..15).map(|i| {
                        row(
                            props!(gap: 8, cross_align: CrossAlign::Center),
                            [
                                canvas(
                                    props!(width: 8, height: 8),
                                    [Draw::circle(
                                        4.0,
                                        4.0,
                                        4.0,
                                        if i % 2 == 0 { GREEN_50 } else { BLUE_50 },
                                    )],
                                ),
                                text(format!("List item {i}"), style!(size: 14, color: GRAY_10)),
                            ],
                        )
                    }),
                ),
            ],
        ),
    );
}
