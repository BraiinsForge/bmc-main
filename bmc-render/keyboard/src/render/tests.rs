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

use bmc_render::interaction::InteractionState;
use bmc_render::renderer::Renderer;
use bmc_render::renderer::test_support::{DrawnRect, DrawnText, ShapingRecorder};

use crate::layout::ALL_LAYOUTS;
use crate::sound::SilentSink;
use crate::theme::KeyboardTheme;
use crate::{KeyboardCtx, KeyboardState, render_keyboard};

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 480.0;
const TYPED: &str = "hello world";
const CURSOR: usize = 5;
/// Positions agree with the measured widths that produced them to well
/// under a pixel; a shaper disagreement moves them by whole pixels.
const TOLERANCE: f32 = 0.05;

fn render_qwerty_us() -> ShapingRecorder {
    let mut recorder = ShapingRecorder::new(WIDTH, HEIGHT);
    let mut state = KeyboardState::new(TYPED, "Wi-Fi Password", "password");
    state.cursor = CURSOR;
    let mut interaction = InteractionState::new();
    let mut audio = SilentSink;
    let layout = ALL_LAYOUTS
        .first()
        .expect("BUG: at least one layout must be generated");
    render_keyboard(
        &mut KeyboardCtx {
            renderer: &mut recorder,
            interaction: &mut interaction,
            state: &mut state,
            audio: &mut audio,
            theme: &KeyboardTheme::CARBON_DARK,
            width: WIDTH,
            height: HEIGHT,
            delta_ms: 16,
        },
        layout,
    );
    recorder
}

fn only_draw_of(recorder: &ShapingRecorder, text: &str) -> DrawnText {
    let mut matches = recorder.texts.iter().filter(|t| t.text == text);
    let found = matches
        .next()
        .unwrap_or_else(|| panic!("{text:?} must be drawn"))
        .clone();
    assert!(matches.next().is_none(), "{text:?} must be drawn once");
    found
}

/// The key background the drawn `text` sits on.
fn key_bounds_under(recorder: &ShapingRecorder, text: &DrawnText) -> DrawnRect {
    *recorder
        .rounded_rects
        .iter()
        .find(|r| (r.x..r.x + r.w).contains(&text.x) && (r.y..r.y + r.h).contains(&text.y))
        .unwrap_or_else(|| panic!("{:?} must be drawn on a key background", text.text))
}

/// The gap between a key's right edge and its right-aligned popup hint.
fn hint_right_gap(recorder: &mut ShapingRecorder, label: &str, hint: &str) -> f32 {
    let label_draw = only_draw_of(recorder, label);
    let key = key_bounds_under(recorder, &label_draw);
    let hint_draw = only_draw_of(recorder, hint);
    assert_eq!(
        key_bounds_under(recorder, &hint_draw),
        key,
        "the hint must be drawn on the key it belongs to"
    );
    let hint_w = recorder.measure_text(hint, hint_draw.size);
    key.x + key.w - (hint_draw.x + hint_w)
}

#[test]
fn the_cursor_stands_at_the_shaped_width_of_the_text_before_it() {
    let mut recorder = render_qwerty_us();
    let field = only_draw_of(&recorder, TYPED);
    let before_cursor = TYPED
        .get(..CURSOR)
        .expect("BUG: the cursor must sit on a char boundary");

    let expected_x = field.x + recorder.measure_text(before_cursor, field.size);
    assert!(
        recorder
            .rects
            .iter()
            .any(|r| { (r.x - expected_x).abs() < TOLERANCE && (r.y - field.y).abs() < TOLERANCE }),
        "the cursor must stand at {expected_x} on the text baseline, drew {:?}",
        recorder.rects
    );
}

#[test]
fn a_key_label_is_centered_by_its_shaped_width() {
    let mut recorder = render_qwerty_us();
    let label = only_draw_of(&recorder, "a");
    let key = key_bounds_under(&recorder, &label);

    let label_w = recorder.measure_text(&label.text, label.size);
    let left = label.x - key.x;
    let right = key.x + key.w - (label.x + label_w);
    assert!(
        (left - right).abs() < TOLERANCE,
        "a centered label must leave equal margins, left {left} right {right}"
    );
}

#[test]
fn popup_hints_share_one_right_margin_whatever_they_shape_to() {
    let mut recorder = render_qwerty_us();
    // Two hints of different shaped widths: right alignment must absorb
    // the difference, a left-anchored or width-blind draw could not.
    let narrow = recorder.measure_text("ĵ", 20.0);
    let wide = recorder.measure_text("§", 20.0);
    assert!(
        (narrow - wide).abs() > 1.0,
        "the fixture needs hints of visibly different widths"
    );

    let j_gap = hint_right_gap(&mut recorder, "j", "ĵ");
    let s_gap = hint_right_gap(&mut recorder, "s", "§");
    assert!(
        j_gap > 0.0 && (j_gap - s_gap).abs() < TOLERANCE,
        "hints must be inset from their key's right edge alike, got {j_gap} and {s_gap}"
    );
}
