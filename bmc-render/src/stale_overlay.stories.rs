// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "StaleOverlay" }

#[story(default)]
fn stale_overlay(ctx: &mut StoryCtx) {
    let age = ctx.slider("Last refresh age (s)", 120.0, 0.0, 200_000.0);
    let round = ctx.toggle("Round face", false);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "slider seconds are small whole numbers"
    )]
    let secs = age.get() as i64;
    // Storybook clock ticks up from 0, so the anchor sits `secs` in the past.
    let anchor = SystemTime { unix_secs: -secs };

    let (shape, frame, tile_h) = if round.get() {
        (ViewportShape::Round, FrameSize::Round(480), 480.0_f32)
    } else {
        (
            ViewportShape::Rectangular,
            FrameSize::from((480, 240)),
            240.0_f32,
        )
    };
    let tile = col(
        props!(width: 480, height: tile_h, padding: 24, cross_align: CrossAlign::Center),
        [text("Weather · 21°C", style!(size: 22, color: WHITE))],
    );
    ctx.ui.div(frame, with_stale_overlay(tile, anchor, shape));
}
