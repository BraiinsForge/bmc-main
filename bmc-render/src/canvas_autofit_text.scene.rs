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

use std::cmp::min;

use bmc_gallery::prelude::*;

scene_meta! { title: "Components / Canvas / AutofitText" }

const EXAMPLE_TEXT_CZ: &str = "Kapr modravý plave řekou pod starým mostem, zatímco vítr nese vůni jarního deště přes rozkvetlé louky. Tichá voda zrcadlí oblohu a větve vrb se sklánějí až k hladině. Mnohé příběhy začínají právě zde, mezi kameny a kořeny, kde se světlo láme do tisíce drobných jisker. Žluťoučký kůň úpěl ďábelské ódy, neboť měšťané spěchali domů ještě před soumrakem. Každý krok zněl po dlažbě jako tlumený buben a stíny se prodlužovaly do úzkých uliček.";
const EXAMPLE_TEXT_EN: &str = "Lorem ipsum dolor sit amet, consectetuer adipiscing elit. Aenean commodo ligula eget dolor. Aenean massa. Cum sociis natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus. Donec quam felis, ultricies nec, pellentesque eu, pretium quis, sem. Nulla consequat massa quis enim. Donec pede justo, fringilla vel, aliquet nec, vulputate eget, arcu.";

/// First `chars` characters of `string`, or all of it when `chars` exceeds the
/// character count. Slices on a `char_indices` boundary so multibyte text is
/// never split mid-`char`.
#[expect(
    clippy::string_slice,
    reason = "`end` is a `char_indices` offset, so it is a boundary by construction"
)]
fn get_string_prefix(string: &str, chars: usize) -> &str {
    match string.char_indices().nth(chars) {
        Some((end, _)) => &string[..end],
        None => string,
    }
}

/// Shared "Text length" slider over `[1, min(EN, CZ) char count]`, returning the
/// chosen length in characters.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a character count over a fixed sample paragraph, and the slider \
              is bounded to it"
)]
fn text_length_slider(ctx: &mut SceneCtx, default: f32) -> usize {
    let max_len = min(
        EXAMPLE_TEXT_EN.chars().count(),
        EXAMPLE_TEXT_CZ.chars().count(),
    );
    let max = max_len as f32;
    ctx.slider("Text length", default, 1.0, max, 1.0) as usize
}

/// One labeled autofit demo: `label` above a dark box that scales `content`
/// with the given mode and explicit bounds via `Draw::autofit_text_ranged`.
fn mode_demo(label: &str, content: &str, mode: AutoFit, min_size: u16, max_size: u16) -> Node {
    const W: f32 = 300.0;
    const H: f32 = 100.0;
    col(
        props!(gap: 8),
        [
            text(label, style!(size: 14, color: GRAY_50)),
            canvas(
                props!(width: W, height: H),
                [
                    Draw::rect(0.0, 0.0, W, H, Fill::Solid(GRAY_80)),
                    Draw::autofit_text_ranged(
                        0.0,
                        0.0,
                        W,
                        H,
                        content,
                        style!(size: 30, align: TextAlign::Center),
                        mode,
                        min_size,
                        max_size,
                    ),
                ],
            ),
        ],
    )
}

// This scene uses also Czech text to show correct handling of multibyte characters.
#[scene(default)]
fn basic(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Basic Example");
    ui.label(
        "Draw dummy text of an arbitrary length using Draw::autofit_text, font size 30, TextAlign::Center",
    );

    let text_length = text_length_slider(ctx, 57.0);

    ctx.node_stage(ui, (400_u32, 120_u32), || {
        canvas(
            props!(width: 400, height: 120),
            [Draw::autofit_text(
                0.0,
                0.0,
                400.0,
                120.0,
                get_string_prefix(EXAMPLE_TEXT_EN, text_length),
                style!(
                    size: 30,
                    align: TextAlign::Center
                ),
            )],
        )
    });

    ctx.node_stage(ui, (400_u32, 120_u32), || {
        canvas(
            props!(width: 400, height: 120),
            [Draw::autofit_text(
                0.0,
                0.0,
                400.0,
                120.0,
                get_string_prefix(EXAMPLE_TEXT_CZ, text_length),
                style!(
                    size: 30,
                    align: TextAlign::Center
                ),
            )],
        )
    });
}

#[scene]
fn horizontal_alignment(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Horizontal Alignment");
    ui.label("Left, Center and Right alignment modes");

    let text_length = text_length_slider(ctx, 105.0);

    let alignments = [TextAlign::Left, TextAlign::Center, TextAlign::Right];

    for align in alignments {
        ctx.node_stage(ui, (400_u32, 120_u32), || {
            canvas(
                props!(width: 400, height: 120),
                [Draw::autofit_text(
                    0.0,
                    0.0,
                    400.0,
                    120.0,
                    get_string_prefix(EXAMPLE_TEXT_EN, text_length),
                    style!(
                        size: 30,
                        align: align
                    ),
                )],
            )
        });
    }
}

#[scene]
fn vertical_alignment(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Vertical Alignment");
    ui.label("Top, Center and Bottom alignment modes");

    let text_length = text_length_slider(ctx, 57.0);

    let alignments = [
        VerticalAlign::Top,
        VerticalAlign::Center,
        VerticalAlign::Bottom,
    ];

    for valign in alignments {
        ctx.node_stage(ui, (400_u32, 120_u32), || {
            canvas(
                props!(width: 400, height: 120),
                [Draw::autofit_text(
                    0.0,
                    0.0,
                    400.0,
                    120.0,
                    get_string_prefix(EXAMPLE_TEXT_EN, text_length),
                    style!(
                        size: 30,
                        align: TextAlign::Center,
                        valign: valign,
                    ),
                )],
            )
        });
    }
}

#[scene]
fn text_positioning(ctx: &mut SceneCtx, ui: &mut Ui) {
    const DIV_WIDTH: f32 = 400.0;
    const DIV_HEIGHT: f32 = 400.0;

    ui.heading("Text Positioning");
    ui.label("Showcase of text positioning");

    let x = ctx.slider("X", 0.0, 0.0, DIV_WIDTH, 1.0);
    let y = ctx.slider("Y", 0.0, 0.0, DIV_HEIGHT, 1.0);
    let box_width = ctx.slider("Box width", 60.0, 60.0, 100.0, 1.0);
    let box_height = ctx.slider("Box height", 60.0, 60.0, 100.0, 1.0);

    ctx.node_stage(ui, (DIV_WIDTH, DIV_HEIGHT), || {
        canvas(
            props!(width: DIV_WIDTH, height: DIV_HEIGHT),
            [
                Draw::rect(x, y, box_width, box_height, Fill::Solid(GRAY_70)),
                Draw::autofit_text(
                    x,
                    y,
                    box_width,
                    box_height,
                    get_string_prefix(EXAMPLE_TEXT_EN, 20),
                    style!(
                        size: 30,
                        align: TextAlign::Center,
                        valign: VerticalAlign::Top,
                    ),
                ),
            ],
        )
    });
}

// Same content and box in every mode. Start size is 30. Drag the slider:
// short text lets Grow / ShrinkAndGrow enlarge to fill, while Shrink stays at
// 30; long text lets Shrink / ShrinkAndGrow scale down, while Grow cannot and
// overflows.
#[scene]
fn fit_modes(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Fit Modes");
    ui.label("Shrink, Grow and ShrinkAndGrow at start size 30 in a 300x100 box");

    let text_length = text_length_slider(ctx, 12.0);
    let content = get_string_prefix(EXAMPLE_TEXT_EN, text_length);

    ctx.node_stage(ui, (350_usize, AutoH), || {
        col(
            props!(gap: 16, padding: 16),
            [
                // Shrink ignores max_size and searches [min_size, size].
                mode_demo("Shrink [12..30]", content, AutoFit::Shrink, 12, 0),
                // Grow ignores min_size and searches [size, max_size].
                mode_demo("Grow [30..120]", content, AutoFit::Grow, 0, 120),
                mode_demo(
                    "ShrinkAndGrow [12..120]",
                    content,
                    AutoFit::ShrinkAndGrow,
                    12,
                    120,
                ),
            ],
        )
    });
}

// Explicit min/max clamp the fitted size: it never leaves [min, max] even when
// the box could fit a larger or smaller line. Uses ShrinkAndGrow so both
// bounds are active.
#[scene]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "font sizes off sliders bounded to 4..=160 points"
)]
fn explicit_bounds(ctx: &mut SceneCtx, ui: &mut Ui) {
    const W: f32 = 400.0;
    const H: f32 = 160.0;

    ui.heading("Explicit Bounds");
    ui.label("ShrinkAndGrow clamped to an explicit min/max font size");

    let text_length = text_length_slider(ctx, 40.0);
    let min_size = ctx.slider("Min size", 16.0, 4.0, 80.0, 1.0) as u16;
    let max_size = ctx.slider("Max size", 72.0, 4.0, 160.0, 1.0) as u16;

    ctx.node_stage(ui, (W, H), || {
        canvas(
            props!(width: W, height: H),
            [
                Draw::rect(0.0, 0.0, W, H, Fill::Solid(GRAY_80)),
                Draw::autofit_text_ranged(
                    0.0,
                    0.0,
                    W,
                    H,
                    get_string_prefix(EXAMPLE_TEXT_EN, text_length),
                    style!(size: 30, align: TextAlign::Center),
                    AutoFit::ShrinkAndGrow,
                    min_size,
                    max_size,
                ),
            ],
        )
    });
}
