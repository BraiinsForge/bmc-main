// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Layout" }

#[story(default)]
fn examples(_: &mut StoryCtx) -> Node {
    col(
        props!(gap: 24, padding: 16),
        [
            // ── Row with spacer ──
            text("Row with spacer", style!(size: 14, color: GRAY_30)),
            row(
                props!(gap: 8, width: 300, height: 60, cross_align: CrossAlign::Center, background: GRAY_90, padding: 8),
                [
                    text(
                        "Left",
                        style!(size: 16, color: WHITE, background: GRAY_80, padding: 8),
                    ),
                    spacer(1),
                    text(
                        "Right",
                        style!(size: 16, color: WHITE, background: GRAY_80, padding: 8),
                    ),
                ],
            ),
            // ── Nested panels ──
            text("Nested columns", style!(size: 14, color: GRAY_30)),
            col(
                props!(gap: 12, width: 400),
                [
                    text(
                        "Header",
                        style!(size: 20, weight: FontWeight::BOLD, color: WHITE),
                    ),
                    row(
                        props!(gap: 8),
                        [
                            col(
                                props!(gap: 4, flex: 1, background: GRAY_90, padding: 8),
                                [
                                    text("Panel A", style!(size: 14, color: GRAY_30)),
                                    text("Content here", style!(size: 16, color: WHITE)),
                                ],
                            ),
                            col(
                                props!(gap: 4, flex: 1, background: GRAY_90, padding: 8),
                                [
                                    text("Panel B", style!(size: 14, color: GRAY_30)),
                                    text("More content", style!(size: 16, color: WHITE)),
                                ],
                            ),
                        ],
                    ),
                ],
            ),
            // ── Cross-axis alignment ──
            text("Cross-axis alignment", style!(size: 14, color: GRAY_30)),
            row(
                props!(gap: 8, width: 400, height: 80, background: GRAY_90, padding: 8, cross_align: CrossAlign::Start),
                [text(
                    "Start",
                    style!(size: 14, color: WHITE, background: GRAY_80, padding: 4),
                )],
            ),
            row(
                props!(gap: 8, width: 400, height: 80, background: GRAY_90, padding: 8, cross_align: CrossAlign::Center),
                [text(
                    "Center",
                    style!(size: 14, color: WHITE, background: GRAY_80, padding: 4),
                )],
            ),
            row(
                props!(gap: 8, width: 400, height: 80, background: GRAY_90, padding: 8, cross_align: CrossAlign::End),
                [text(
                    "End",
                    style!(size: 14, color: WHITE, background: GRAY_80, padding: 4),
                )],
            ),
            // ── Centered container ──
            text(
                "center() — children laid out in the centre of the box",
                style!(size: 14, color: GRAY_30),
            ),
            center(
                props!(width: 400, height: 100, background: GRAY_90),
                [text(
                    "Centered",
                    style!(size: 18, color: WHITE, background: GRAY_80, padding: 8),
                )],
            ),
        ],
    )
}
