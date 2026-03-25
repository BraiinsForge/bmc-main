// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Modal" }

#[story(default)]
fn examples(ctx: &mut StoryCtx) {
    let key_basic = ctx.action("basic");
    let key_footer = ctx.action("footer");
    let key_danger = ctx.action("danger");

    let variant = ctx.radio("Variant", &["Basic", "With footer", "Danger"], 0);
    let v: usize = variant.into();
    let long_body = ctx.toggle("Long body (scrolling)", false);

    // Realistic prose content (3 paragraphs, repeated) used when the
    // "Long body (scrolling)" toggle is on. Crafted so the scroll
    // fallback is visibly exercised rather than showing identical lines.
    let body = if long_body.get() {
        let para_a = "Long-form modal content lays out at its natural height. When the body's \
                      intrinsic height exceeds the modal viewport, the body must be wrapped in a \
                      scroll container — the modal frame itself stays anchored.";
        let para_b = "Wrapping the body in scroll() pins the modal chrome (title bar, footer) \
                      to the modal edges and lets only the body region scroll. This is the \
                      recommended pattern for confirmation dialogs with terms-of-service text, \
                      release notes, log inspectors, and similar dense content.";
        let para_c = "Without the scroll container, the modal would simply size to fit its \
                      intrinsic height, potentially exceeding the screen. The scroll wrapper \
                      bounds the modal height to a fixed viewport while keeping all content \
                      reachable via vertical pan.";

        let mut prose: Vec<Node> = Vec::new();
        for round in 0..3 {
            for (i, para) in [para_a, para_b, para_c].into_iter().enumerate() {
                prose.push(text(
                    &fmt!("§{}.{}  {}", round + 1, i + 1, para),
                    style!(size: 14, color: GRAY_10, line_height: 1.4),
                ));
            }
        }
        prose
    } else {
        match v {
            0 => vec![text(
                "This is a simple modal with no footer actions.",
                style!(size: 14, color: GRAY_10),
            )],
            1 => vec![text(
                "Are you sure you want to proceed?",
                style!(size: 14, color: GRAY_10),
            )],
            // Danger
            _ => vec![text(
                "This action cannot be undone. All associated data will be permanently removed.",
                style!(size: 14, color: GRAY_10),
            )],
        }
    };

    let (key, title, footer) = match v {
        0 => (key_basic, "Basic modal", None),
        1 => (
            key_footer,
            "Confirm action",
            Some(ModalFooter {
                primary: ModalAction { label: "Confirm" },
                secondary: Some(ModalAction { label: "Cancel" }),
                danger: false,
            }),
        ),
        // Danger
        _ => (
            key_danger,
            "Delete item",
            Some(ModalFooter {
                primary: ModalAction { label: "Delete" },
                secondary: Some(ModalAction { label: "Cancel" }),
                danger: true,
            }),
        ),
    };

    let n = modal(
        key,
        true,
        title,
        body,
        Some(ModalProps {
            max_width: 480,
            footer,
            ..Default::default()
        }),
    );

    let content = col(
        props!(padding: 16),
        [
            text(
                "Background content behind modal",
                style!(size: 14, padding: 16),
            ),
            n,
        ],
    );

    ctx.ui.div(Full, content.clone());
    ctx.ui.div(Large, content.clone());
    ctx.ui.div(Medium, content.clone());
    ctx.ui.div(Small, content.clone());
}
