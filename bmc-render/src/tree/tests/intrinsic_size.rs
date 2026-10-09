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

use super::*;
use crate::renderer::test_support::ShapingRecorder;

fn span(text: &str) -> SpanData {
    SpanData {
        text: text.to_owned(),
        weight: None,
        color: None,
        size: None,
        italic: false,
        underline: false,
        strikethrough: false,
    }
}

fn word_texts(words: &[Vec<SpanData>]) -> Vec<Vec<&str>> {
    words
        .iter()
        .map(|w| w.iter().map(|s| s.text.as_str()).collect())
        .collect()
}

#[test]
fn min_content_words_join_across_span_boundaries() {
    // "hashrate" is one word split across two styled spans: probing its
    // fragments separately would under-report the min-content width.
    let words = split_min_content_words(&[span("current hash"), span("rate now")]);
    assert_eq!(
        word_texts(&words),
        [vec!["current"], vec!["hash", "rate"], vec!["now"],]
    );
}

#[test]
fn min_content_words_treat_nbsp_as_glue() {
    // A line cannot break at U+00A0, so "10\u{A0}TH/s" must probe as one
    // word; regular spaces still split.
    let words = split_min_content_words(&[span("about 10\u{A0}TH/s total")]);
    assert_eq!(
        word_texts(&words),
        [vec!["about"], vec!["10\u{A0}TH/s"], vec!["total"]]
    );
}

#[test]
fn min_content_words_drop_leading_and_trailing_whitespace() {
    let words = split_min_content_words(&[span("  spaced  out  ")]);
    assert_eq!(word_texts(&words), [vec!["spaced"], vec!["out"]]);
}

#[test]
fn a_button_sizes_itself_around_its_shaped_label() {
    const LABEL: &str = "Disconnect";
    let size = ButtonSize::Normal;

    let mut recorder = ShapingRecorder::default();
    let label_w = recorder.measure_text(LABEL, size.font_size());

    let mut taffy: TaffyTree<NodeContext> = TaffyTree::new();
    let button = taffy
        .new_leaf_with_context(
            Style::default(),
            NodeContext {
                button: Some(ButtonContext {
                    id: "btn".to_owned(),
                    label: LABEL.to_owned(),
                    style: ButtonStyle::Primary,
                    size,
                    icon_id: None,
                    disabled: false,
                    skin: None,
                }),
                ..NodeContext::default()
            },
        )
        .expect("BUG: taffy must accept a leaf node");
    compute_taffy_layout(&mut taffy, button, &mut recorder)
        .expect("BUG: laying out a single leaf must succeed");

    let laid_out = taffy
        .layout(button)
        .expect("BUG: the laid-out leaf must have a layout");
    assert!(
        (laid_out.size.width - (label_w + size.h_padding() * 2.0)).abs() < 0.5,
        "a label-only button must be as wide as its shaped label plus padding, got {}",
        laid_out.size.width
    );
}
