// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use core::f32::consts::TAU;

use bmc_gallery::prelude::*;

scene_meta! { title: "Components / Animation" }

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

// ── Scenes ───────────────────────────────────────────────────────────

#[scene(default)]
#[expect(
    clippy::too_many_lines,
    reason = "one stage per easing variant, which is the catalogue itself"
)]
fn easing_curves(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Easing Functions");
    ui.label("All easing variants applied to TranslateX");
    ctx.node_stage(ui, (500_usize, AutoH), || {
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
        )
    });
}

#[scene]
#[expect(
    clippy::too_many_lines,
    reason = "one stage per animated property, which is the catalogue itself"
)]
fn properties(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Animated Properties");
    ui.label("Each AnimProperty variant");

    ui.heading("Rotate");
    ctx.node_stage(ui, (300_u32, 80_u32), || {
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
        )
    });

    ui.heading("Scale");
    ctx.node_stage(ui, (300_u32, 80_u32), || {
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
        )
    });

    ui.heading("Alpha");
    ctx.node_stage(ui, (300_u32, 80_u32), || {
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
        )
    });

    ui.heading("TranslateX");
    ctx.node_stage(ui, (300_u32, 60_u32), || {
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
        )
    });

    ui.heading("TranslateY");
    ctx.node_stage(ui, (300_u32, 100_u32), || {
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
        )
    });

    ui.heading("OrbitAngle");
    ctx.node_stage(ui, (300_u32, 120_u32), || {
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
        )
    });

    ui.heading("Color");
    ctx.node_stage(ui, (300_u32, 80_u32), || {
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
        )
    });
}

#[scene]
fn loop_modes(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Loop Modes");
    ui.label("Once, Forever, PingPong");

    ui.heading("Once — plays once then stops");
    ctx.node_stage(ui, (400_u32, 60_u32), || {
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
        )
    });

    ui.heading("Forever — repeats from start");
    ctx.node_stage(ui, (400_u32, 60_u32), || {
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
        )
    });

    ui.heading("PingPong — reverses direction");
    ctx.node_stage(ui, (400_u32, 60_u32), || {
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
        )
    });
}

#[scene]
fn combined(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Combined Animations");
    ui.label("Multiple animations on one element");

    ui.heading("Rotate + Scale + Color");
    ctx.node_stage(ui, (300_u32, 120_u32), || {
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
        )
    });

    ui.heading("Orbit + Scale");
    ctx.node_stage(ui, (300_u32, 160_u32), || {
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
        )
    });
}

#[scene]
fn delay(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Staggered Delay");
    ui.label("Same animation with increasing delay_ms");
    ctx.node_stage(ui, (400_usize, AutoH), || {
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
        )
    });
}
