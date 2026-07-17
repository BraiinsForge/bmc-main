// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Link" }

#[story(default)]
fn examples(c: &mut StoryCtx) {
    c.ui.header(
        "Link",
        "Tappable text whose hit target hugs the label — click a link to log it",
    );
    let on_fleet = c.action("fleet");
    let on_model = c.action("model");
    c.ui.div(
        (560, AutoH),
        // A breadcrumb-style path: because each link hugs its label, the "/"
        // separators sit an even gap apart whatever the label lengths.
        row(
            props!(gap: 12, padding: 24, cross_align: CrossAlign::Center),
            [
                link(&on_fleet, "My Fleet", style!(size: 24, color: BLUE_50)),
                text("/", style!(size: 24, color: GRAY_50)),
                link(
                    &on_model,
                    "Braiins Forge Miner x4",
                    style!(size: 24, color: BLUE_50),
                ),
                text("/", style!(size: 24, color: GRAY_50)),
                text("bos-01", style!(size: 24, color: GRAY_10)),
            ],
        ),
    );
}
