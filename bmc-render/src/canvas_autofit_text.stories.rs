// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;
use std::cmp::min;

story_meta! { title: "Canvas/AutofitText" }

const EXAMPLE_TEXT_CZ: &str = "Kapr modravý plave řekou pod starým mostem, zatímco vítr nese vůni jarního deště přes rozkvetlé louky. Tichá voda zrcadlí oblohu a větve vrb se sklánějí až k hladině. Mnohé příběhy začínají právě zde, mezi kameny a kořeny, kde se světlo láme do tisíce drobných jisker. Žluťoučký kůň úpěl ďábelské ódy, neboť měšťané spěchali domů ještě před soumrakem. Každý krok zněl po dlažbě jako tlumený buben a stíny se prodlužovaly do úzkých uliček.";
const EXAMPLE_TEXT_EN: &str = "Lorem ipsum dolor sit amet, consectetuer adipiscing elit. Aenean commodo ligula eget dolor. Aenean massa. Cum sociis natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus. Donec quam felis, ultricies nec, pellentesque eu, pretium quis, sem. Nulla consequat massa quis enim. Donec pede justo, fringilla vel, aliquet nec, vulputate eget, arcu.";

/// First `chars` characters of `string`, or all of it when `chars` exceeds the
/// character count. Slices on a `char_indices` boundary so multibyte text is
/// never split mid-`char`.
fn get_string_prefix(string: &str, chars: usize) -> &str {
    match string.char_indices().nth(chars) {
        #[expect(
            clippy::string_slice,
            reason = "'end' comes from char_indices, so it is char-aligned"
        )]
        Some((end, _)) => &string[..end],
        None => string,
    }
}

/// Shared "Text length" slider over `[1, min(EN, CZ) char count]`, returning the
/// chosen length in characters.
fn text_length_slider(c: &mut StoryCtx, default: f32) -> usize {
    let max_len = min(
        EXAMPLE_TEXT_EN.chars().count(),
        EXAMPLE_TEXT_CZ.chars().count(),
    );
    #[expect(
        clippy::cast_precision_loss,
        reason = "text lengths are small, well within f32's exact-integer range"
    )]
    let max = max_len as f32;
    let value = c.slider("Text length", default, 1.0, max).get();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the slider yields a non-negative length; truncating toward zero is intended"
    )]
    let len = value as usize;
    len
}

// This story uses also Czech text to show correct handling of multibyte characters.
#[story(default)]
fn basic(c: &mut StoryCtx) {
    c.ui.header(
        "Basic Example",
        "Draw dummy text of an arbitrary length using Draw::autofit_text, font size 30, TextAlign::Center",
    );

    let text_length = text_length_slider(c, 57.0);

    c.ui.div(
        (400, 120),
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
        ),
    );

    c.ui.div(
        (400, 120),
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
        ),
    );
}

#[story]
fn horizontal_alignment(c: &mut StoryCtx) {
    c.ui.header(
        "Horizontal Alignment",
        "Left, Center and Right alignment modes",
    );

    let text_length = text_length_slider(c, 105.0);

    let alignments = [TextAlign::Left, TextAlign::Center, TextAlign::Right];

    for align in alignments {
        c.ui.div(
            (400, 120),
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
            ),
        );
    }
}

#[story]
fn vertical_alignment(c: &mut StoryCtx) {
    c.ui.header(
        "Vertical Alignment",
        "Top, Center and Bottom alignment modes",
    );

    let text_length = text_length_slider(c, 57.0);

    let alignments = [
        VerticalAlign::Top,
        VerticalAlign::Center,
        VerticalAlign::Bottom,
    ];

    for valign in alignments {
        c.ui.div(
            (400, 120),
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
            ),
        );
    }
}

#[story]
fn text_positioning(c: &mut StoryCtx) {
    const DIV_WIDTH: f32 = 400.0;
    const DIV_HEIGHT: f32 = 400.0;

    c.ui.header("Text Positioning", "Showcase of text positioning");

    let x = c.slider("X", 0.0, 0.0, DIV_WIDTH).get();
    let y = c.slider("Y", 0.0, 0.0, DIV_HEIGHT).get();
    let box_width = c.slider("Box width", 60.0, 60.0, 100.0).get();
    let box_height = c.slider("Box height", 60.0, 60.0, 100.0).get();

    c.ui.div(
        (DIV_WIDTH, DIV_HEIGHT),
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
        ),
    );
}
