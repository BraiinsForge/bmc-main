// Copyright (C) 2026  Braiins Systems s.r.o.

use core::f32::consts::TAU;

use crate::prelude::*;

story_meta! { title: "Animation" }

// ── Helpers ─────────────────────────────────────────────────────────

/// Animated square showing a single property + easing combination.
fn demo_square(
    property: AnimProperty,
    from: f32,
    to: f32,
    duration_ms: u32,
    easing: Easing,
    loop_mode: LoopMode,
    color: Color,
) -> Draw {
    Draw::rect(0.0, 0.0, 32.0, 32.0, color).animate(
        property,
        from,
        to,
        duration_ms,
        easing,
        loop_mode,
    )
}

/// Label + animated square in a row.
fn labeled_demo(label: &str, draw: Draw) -> Node {
    row(
        props!(gap: 16, height: 48),
        [
            text(label, style!(size: 12, color: GRAY_50, width: 120)),
            canvas(props!(width: 80, height: 48), [draw]),
        ],
    )
}

// ── Stories ──────────────────────────────────────────────────────────

#[story(default)]
#[expect(clippy::too_many_lines)]
fn easing_curves(c: &mut StoryCtx) {
    c.ui.header(
        "Easing Functions",
        "All easing variants applied to TranslateX",
    );
    c.ui.div(
        (500, AutoH),
        col(
            props!(gap: 4, padding: 16),
            [
                labeled_demo(
                    "Linear",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::Linear,
                        LoopMode::PingPong,
                        BLUE_50,
                    ),
                ),
                labeled_demo(
                    "EaseIn",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::EaseIn,
                        LoopMode::PingPong,
                        BLUE_50,
                    ),
                ),
                labeled_demo(
                    "EaseOut",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::EaseOut,
                        LoopMode::PingPong,
                        BLUE_50,
                    ),
                ),
                labeled_demo(
                    "EaseInOut",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::EaseInOut,
                        LoopMode::PingPong,
                        BLUE_50,
                    ),
                ),
                labeled_demo(
                    "EaseInCubic",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::EaseInCubic,
                        LoopMode::PingPong,
                        BLUE_50,
                    ),
                ),
                labeled_demo(
                    "EaseOutCubic",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::EaseOutCubic,
                        LoopMode::PingPong,
                        BLUE_50,
                    ),
                ),
                labeled_demo(
                    "EaseInOutCubic",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::EaseInOutCubic,
                        LoopMode::PingPong,
                        BLUE_50,
                    ),
                ),
                labeled_demo(
                    "EaseOutBack",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::EaseOutBack,
                        LoopMode::PingPong,
                        VIOLET_50,
                    ),
                ),
                labeled_demo(
                    "EaseInOutBack",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::EaseInOutBack,
                        LoopMode::PingPong,
                        VIOLET_50,
                    ),
                ),
                labeled_demo(
                    "EaseOutBounce",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::EaseOutBounce,
                        LoopMode::PingPong,
                        ORANGE_50,
                    ),
                ),
                labeled_demo(
                    "EaseOutElastic",
                    demo_square(
                        AnimProperty::TranslateX,
                        0.0,
                        40.0,
                        2_000,
                        Easing::EaseOutElastic,
                        LoopMode::PingPong,
                        RED_50,
                    ),
                ),
            ],
        ),
    );
}

#[story]
#[expect(clippy::too_many_lines)]
fn properties(c: &mut StoryCtx) {
    c.ui.header("Animated Properties", "Each AnimProperty variant");

    c.ui.header("Rotate", "");
    c.ui.div(
        (300, 80),
        canvas(
            props!(width: 300, height: 80),
            [Draw::centered(
                Draw::rect(-16.0, -16.0, 32.0, 32.0, GREEN_50).animate(
                    AnimProperty::Rotate,
                    0.0,
                    TAU,
                    3_000,
                    Easing::Linear,
                    LoopMode::Forever,
                ),
            )],
        ),
    );

    c.ui.header("Scale", "");
    c.ui.div(
        (300, 80),
        canvas(
            props!(width: 300, height: 80),
            [Draw::centered(
                Draw::rect(-16.0, -16.0, 32.0, 32.0, YELLOW_40).animate(
                    AnimProperty::Scale,
                    0.5,
                    1.5,
                    1_500,
                    Easing::EaseInOut,
                    LoopMode::PingPong,
                ),
            )],
        ),
    );

    c.ui.header("Alpha", "");
    c.ui.div(
        (300, 80),
        canvas(
            props!(width: 300, height: 80),
            [Draw::centered(
                Draw::rect(-16.0, -16.0, 32.0, 32.0, RED_50).animate(
                    AnimProperty::Alpha,
                    0.0,
                    1.0,
                    2_000,
                    Easing::EaseInOut,
                    LoopMode::PingPong,
                ),
            )],
        ),
    );

    c.ui.header("TranslateX", "");
    c.ui.div(
        (300, 60),
        canvas(
            props!(width: 300, height: 60),
            [Draw::rect(0.0, 14.0, 32.0, 32.0, BLUE_50).animate(
                AnimProperty::TranslateX,
                0.0,
                250.0,
                2_000,
                Easing::EaseInOut,
                LoopMode::PingPong,
            )],
        ),
    );

    c.ui.header("TranslateY", "");
    c.ui.div(
        (300, 100),
        canvas(
            props!(width: 300, height: 100),
            [Draw::centered(
                Draw::rect(-16.0, -40.0, 32.0, 32.0, VIOLET_50).animate(
                    AnimProperty::TranslateY,
                    0.0,
                    60.0,
                    2_000,
                    Easing::EaseInOut,
                    LoopMode::PingPong,
                ),
            )],
        ),
    );

    c.ui.header("OrbitAngle", "");
    c.ui.div(
        (300, 120),
        canvas(
            props!(width: 300, height: 120),
            [Draw::centered(
                Draw::orbit(40.0, 0.0, Draw::circle(0.0, 0.0, 10.0, ORANGE_50)).animate(
                    AnimProperty::OrbitAngle,
                    0.0,
                    TAU,
                    3_000,
                    Easing::Linear,
                    LoopMode::Forever,
                ),
            )],
        ),
    );

    c.ui.header("Color", "");
    c.ui.div(
        (300, 80),
        canvas(
            props!(width: 300, height: 80),
            [Draw::centered(
                Draw::rect(-16.0, -16.0, 32.0, 32.0, RED_50).animate_color(
                    RED_50,
                    BLUE_50,
                    3_000,
                    Easing::EaseInOut,
                    LoopMode::PingPong,
                ),
            )],
        ),
    );
}

#[story]
fn loop_modes(c: &mut StoryCtx) {
    c.ui.header("Loop Modes", "Once, Forever, PingPong");

    c.ui.header("Once — plays once then stops", "");
    c.ui.div(
        (400, 60),
        canvas(
            props!(width: 400, height: 60),
            [Draw::rect(0.0, 14.0, 32.0, 32.0, GREEN_50).animate(
                AnimProperty::TranslateX,
                0.0,
                340.0,
                2_000,
                Easing::EaseOut,
                LoopMode::Once,
            )],
        ),
    );

    c.ui.header("Forever — repeats from start", "");
    c.ui.div(
        (400, 60),
        canvas(
            props!(width: 400, height: 60),
            [Draw::rect(0.0, 14.0, 32.0, 32.0, BLUE_50).animate(
                AnimProperty::TranslateX,
                0.0,
                340.0,
                2_000,
                Easing::EaseInOut,
                LoopMode::Forever,
            )],
        ),
    );

    c.ui.header("PingPong — reverses direction", "");
    c.ui.div(
        (400, 60),
        canvas(
            props!(width: 400, height: 60),
            [Draw::rect(0.0, 14.0, 32.0, 32.0, VIOLET_50).animate(
                AnimProperty::TranslateX,
                0.0,
                340.0,
                2_000,
                Easing::EaseInOut,
                LoopMode::PingPong,
            )],
        ),
    );
}

#[story]
fn combined(c: &mut StoryCtx) {
    c.ui.header("Combined Animations", "Multiple animations on one element");

    c.ui.header("Rotate + Scale + Color", "");
    c.ui.div(
        (300, 120),
        canvas(
            props!(width: 300, height: 120),
            [Draw::centered(
                Draw::rect(-16.0, -16.0, 32.0, 32.0, RED_50)
                    .animate(
                        AnimProperty::Rotate,
                        0.0,
                        TAU,
                        4_000,
                        Easing::Linear,
                        LoopMode::Forever,
                    )
                    .animate(
                        AnimProperty::Scale,
                        0.5,
                        1.0,
                        1_000,
                        Easing::EaseInOut,
                        LoopMode::PingPong,
                    )
                    .animate_color(
                        RED_50,
                        GREEN_50,
                        3_000,
                        Easing::EaseInOut,
                        LoopMode::PingPong,
                    ),
            )],
        ),
    );

    c.ui.header("Orbit + Scale", "");
    c.ui.div(
        (300, 160),
        canvas(
            props!(width: 300, height: 160),
            [
                // Center dot
                Draw::centered(Draw::circle(0.0, 0.0, 6.0, GRAY_60)),
                // Orbiting element
                Draw::centered(
                    Draw::orbit(50.0, 0.0, Draw::circle(0.0, 0.0, 12.0, ORANGE_50))
                        .animate(
                            AnimProperty::OrbitAngle,
                            0.0,
                            TAU,
                            3_000,
                            Easing::Linear,
                            LoopMode::Forever,
                        )
                        .animate(
                            AnimProperty::Scale,
                            0.6,
                            1.0,
                            1_500,
                            Easing::EaseInOut,
                            LoopMode::PingPong,
                        ),
                ),
            ],
        ),
    );
}

#[story]
fn delay(c: &mut StoryCtx) {
    c.ui.header("Staggered Delay", "Same animation with increasing delay_ms");
    c.ui.div(
        (400, AutoH),
        col(
            props!(gap: 4, padding: 16),
            [
                canvas(
                    props!(width: 400, height: 40),
                    [Draw::rect(0.0, 4.0, 32.0, 32.0, BLUE_50).animate_delayed(
                        AnimProperty::TranslateX,
                        0.0,
                        340.0,
                        1_000,
                        0,
                        Easing::EaseOut,
                        LoopMode::PingPong,
                    )],
                ),
                canvas(
                    props!(width: 400, height: 40),
                    [Draw::rect(0.0, 4.0, 32.0, 32.0, BLUE_40).animate_delayed(
                        AnimProperty::TranslateX,
                        0.0,
                        340.0,
                        1_000,
                        200,
                        Easing::EaseOut,
                        LoopMode::PingPong,
                    )],
                ),
                canvas(
                    props!(width: 400, height: 40),
                    [Draw::rect(0.0, 4.0, 32.0, 32.0, BLUE_30).animate_delayed(
                        AnimProperty::TranslateX,
                        0.0,
                        340.0,
                        1_000,
                        400,
                        Easing::EaseOut,
                        LoopMode::PingPong,
                    )],
                ),
                canvas(
                    props!(width: 400, height: 40),
                    [Draw::rect(0.0, 4.0, 32.0, 32.0, BLUE_20).animate_delayed(
                        AnimProperty::TranslateX,
                        0.0,
                        340.0,
                        1_000,
                        600,
                        Easing::EaseOut,
                        LoopMode::PingPong,
                    )],
                ),
            ],
        ),
    );
}
