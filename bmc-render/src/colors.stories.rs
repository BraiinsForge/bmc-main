// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Colors" }

#[story(default)]
fn palette(_ctx: &mut StoryCtx) -> Node {
    col(
        props!(gap: 4, padding: 8),
        PALETTE
            .iter()
            .map(|swatch| {
                col(
                    props!(gap: 4),
                    [
                        text(swatch.name, style!(size: 12, color: GRAY_30)),
                        row(
                            props!(gap: 2),
                            swatch
                                .colors
                                .iter()
                                .enumerate()
                                .map(|(i, &c)| {
                                    let step = (i + 1) * 10;
                                    let label_color = if step <= 50 { BLACK } else { WHITE };
                                    col(
                                        props!(width: 32, height: 32, background: c, padding: 2),
                                        [text(
                                            format!("{step}"),
                                            style!(size: 9, color: label_color),
                                        )],
                                    )
                                })
                                .collect::<Vec<_>>(),
                        ),
                    ],
                )
            })
            .collect::<Vec<_>>(),
    )
}
