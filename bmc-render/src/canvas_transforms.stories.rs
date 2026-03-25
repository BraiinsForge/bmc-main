// Copyright (C) 2026  Braiins Systems s.r.o.

use core::f32::consts::{FRAC_PI_4, FRAC_PI_6, TAU};

use crate::prelude::*;

story_meta! { title: "Canvas/Transforms" }

#[story(default)]
fn rotated(c: &mut StoryCtx) {
    c.ui.header(
        "Rotated",
        "Draw::rotated wraps any draw command with a rotation",
    );

    c.ui.div(
        (400, 100),
        canvas(
            props!(width: 400, height: 100),
            [
                // Unrotated reference
                Draw::rect(20.0, 30.0, 40.0, 40.0, GRAY_70),
                // 15° rotation
                Draw::rotated(
                    FRAC_PI_6 / 2.0,
                    Draw::rect(100.0, 30.0, 40.0, 40.0, BLUE_50),
                ),
                // 30° rotation
                Draw::rotated(FRAC_PI_6, Draw::rect(180.0, 30.0, 40.0, 40.0, GREEN_50)),
                // 45° rotation
                Draw::rotated(FRAC_PI_4, Draw::rect(260.0, 30.0, 40.0, 40.0, VIOLET_50)),
                // Labels
                Draw::text(20.0, 78.0, "0°", style!(size: 10, color: GRAY_50)),
                Draw::text(100.0, 78.0, "15°", style!(size: 10, color: GRAY_50)),
                Draw::text(180.0, 78.0, "30°", style!(size: 10, color: GRAY_50)),
                Draw::text(260.0, 78.0, "45°", style!(size: 10, color: GRAY_50)),
            ],
        ),
    );
}

#[story]
fn centered(c: &mut StoryCtx) {
    c.ui.header(
        "Centered",
        "Draw::centered places a draw at the canvas center",
    );

    c.ui.div(
        (200, 100),
        canvas(
            props!(width: 200, height: 100),
            [
                // Crosshair to show center
                Draw::rect(99.0, 0.0, 1.0, 100.0, GRAY_80),
                Draw::rect(0.0, 49.0, 200.0, 1.0, GRAY_80),
                // Centered elements
                Draw::centered(Draw::circle(0.0, 0.0, 20.0, BLUE_50)),
                Draw::centered(Draw::rect(-30.0, -6.0, 60.0, 12.0, GREEN_50)),
            ],
        ),
    );
}

#[story]
fn orbit(c: &mut StoryCtx) {
    c.ui.header(
        "Orbit",
        "Draw::orbit positions a draw at radius+angle from canvas center",
    );

    c.ui.div(
        (200, 200),
        canvas(
            props!(width: 200, height: 200),
            [
                // Center dot
                Draw::centered(Draw::circle(0.0, 0.0, 4.0, GRAY_60)),
                // Orbit ring (approximate with circles at regular angles)
                Draw::centered(Draw::orbit(60.0, 0.0, Draw::circle(0.0, 0.0, 8.0, RED_50))),
                Draw::centered(Draw::orbit(
                    60.0,
                    TAU / 6.0,
                    Draw::circle(0.0, 0.0, 8.0, ORANGE_50),
                )),
                Draw::centered(Draw::orbit(
                    60.0,
                    TAU / 3.0,
                    Draw::circle(0.0, 0.0, 8.0, YELLOW_30),
                )),
                Draw::centered(Draw::orbit(
                    60.0,
                    TAU / 2.0,
                    Draw::circle(0.0, 0.0, 8.0, GREEN_50),
                )),
                Draw::centered(Draw::orbit(
                    60.0,
                    TAU * 2.0 / 3.0,
                    Draw::circle(0.0, 0.0, 8.0, BLUE_50),
                )),
                Draw::centered(Draw::orbit(
                    60.0,
                    TAU * 5.0 / 6.0,
                    Draw::circle(0.0, 0.0, 8.0, VIOLET_50),
                )),
            ],
        ),
    );
}
