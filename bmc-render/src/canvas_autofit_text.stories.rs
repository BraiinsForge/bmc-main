// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;
use std::cmp::min;

story_meta! { title: "Canvas/AutofitText" }

const EXAMPLE_TEXT_CZ: &str = "Kapr modravý plave řekou pod starým mostem, zatímco vítr nese vůni jarního deště přes rozkvetlé louky. Tichá voda zrcadlí oblohu a větve vrb se sklánějí až k hladině. Mnohé příběhy začínají právě zde, mezi kameny a kořeny, kde se světlo láme do tisíce drobných jisker. Žluťoučký kůň úpěl ďábelské ódy, neboť měšťané spěchali domů ještě před soumrakem. Každý krok zněl po dlažbě jako tlumený buben a stíny se prodlužovaly do úzkých uliček.";
const EXAMPLE_TEXT_EN: &str = "Lorem ipsum dolor sit amet, consectetuer adipiscing elit. Aenean commodo ligula eget dolor. Aenean massa. Cum sociis natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus. Donec quam felis, ultricies nec, pellentesque eu, pretium quis, sem. Nulla consequat massa quis enim. Donec pede justo, fringilla vel, aliquet nec, vulputate eget, arcu.";

fn get_string_prefix(string: &str, chars: usize) -> &str {
    if string.len() == 0 {
        return "";
    }

    let end = string
        .char_indices()
        .map(|(i, _)| i)
        .nth(min(chars, string.len() - 1))
        .unwrap();

    return &string[0..end];
}

// This story uses also Czech text to show correct handling of multibyte characters.
#[story(default)]
fn basic(c: &mut StoryCtx) {
    c.ui.header(
        "Basic Example",
        "Draw dummy text of an arbitrary length using Draw::autofit_text, font size 30, TextAlign::Center",
    );

    let min_text_len: usize = min(
        EXAMPLE_TEXT_EN.chars().count(),
        EXAMPLE_TEXT_CZ.chars().count(),
    );

    let text_length = c
        .slider(
            "Text length",
            57.0,
            min(min_text_len, 1) as f32,
            min_text_len as f32,
        )
        .get();

    c.ui.div(
        (400, 120),
        canvas(
            props!(width: 400, height: 120),
            [Draw::autofit_text(
                0.0,
                0.0,
                400.0,
                120.0,
                get_string_prefix(EXAMPLE_TEXT_EN, text_length as usize),
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
                get_string_prefix(EXAMPLE_TEXT_CZ, text_length as usize),
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
                    get_string_prefix(EXAMPLE_TEXT_EN, 105),
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
                    get_string_prefix(EXAMPLE_TEXT_EN, 57),
                    style!(
                        size: 30,
                        align: TextAlign::Center
                    ),
                )],
            ),
        );
    }
}
