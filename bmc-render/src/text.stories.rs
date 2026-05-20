// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Text" }

#[story(default)]
fn examples(_ctx: &mut StoryCtx) -> Node {
    col(
        props!(gap: 24, padding: 16),
        [
            // ── Type scale ──
            text("Type scale", style!(size: 14, color: GRAY_30)),
            col(
                props!(gap: 8),
                [
                    text(
                        "Heading 1",
                        style!(size: 32, weight: FontWeight::BOLD, color: WHITE),
                    ),
                    text(
                        "Heading 2",
                        style!(size: 24, weight: FontWeight::BOLD, color: WHITE),
                    ),
                    text(
                        "Heading 3",
                        style!(size: 20, weight: FontWeight::SEMIBOLD, color: WHITE),
                    ),
                    text("Body text (16px)", style!(size: 16, color: GRAY_10)),
                    text("Caption text (13px)", style!(size: 13, color: GRAY_30)),
                    text("Small text (11px)", style!(size: 11, color: GRAY_50)),
                ],
            ),
            // ── Rich text ──
            text("Rich text", style!(size: 14, color: GRAY_30)),
            col(
                props!(gap: 12),
                [
                    paragraph(
                        style!(size: 16, color: GRAY_10, line_height: 1.4),
                        [
                            span("This is ", ()),
                            span("bold", style!(weight: FontWeight::BOLD)),
                            span(" and this is ", ()),
                            span("italic", style!(italic: true)),
                            span(".", ()),
                        ],
                    ),
                    paragraph(
                        style!(size: 16, color: GRAY_10, line_height: 1.4),
                        [
                            span("Underline", style!(underline: true)),
                            span(" / ", ()),
                            span("Strikethrough", style!(strikethrough: true)),
                            span(" / ", ()),
                            span("Colored", style!(color: VIOLET_40)),
                        ],
                    ),
                ],
            ),
        ],
    )
}
