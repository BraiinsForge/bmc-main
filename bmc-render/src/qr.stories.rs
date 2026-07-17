// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "QR" }

fn qr_tile(label: &str, draw: Draw, size: f32) -> Node {
    col(
        props!(gap: 8, cross_align: CrossAlign::Center),
        [
            canvas(props!(width: size, height: size), [draw]),
            text(label, style!(size: 12, color: GRAY_50)),
        ],
    )
}

#[story(default)]
fn styles(c: &mut StoryCtx) {
    c.ui.header(
        "QR code",
        "Draw::qr — the host encodes the text and rasterises it",
    );
    let content = "https://deck.local/setup";
    let size = 200.0;
    c.ui.div(
        (720, 300),
        row(
            props!(gap: 32, padding: 24, cross_align: CrossAlign::Center),
            [
                qr_tile(
                    "black on white",
                    Draw::qr(0.0, 0.0, size, content, QrStyle::default()),
                    size,
                ),
                qr_tile(
                    "tinted, wide quiet zone",
                    Draw::qr(
                        0.0,
                        0.0,
                        size,
                        content,
                        QrStyle {
                            dark: BLUE_60,
                            light: WHITE,
                            quiet_zone: 4,
                        },
                    ),
                    size,
                ),
                qr_tile(
                    "light on transparent",
                    Draw::qr(
                        0.0,
                        0.0,
                        size,
                        content,
                        QrStyle {
                            dark: WHITE,
                            light: TRANSPARENT,
                            quiet_zone: 2,
                        },
                    ),
                    size,
                ),
            ],
        ),
    );
}
