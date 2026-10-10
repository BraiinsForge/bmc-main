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

//! `Clip` and `Ellipsis` through the real GL path:
//! what gets cut, and what the cut must leave alone.

use std::collections::HashMap;

use super::multiline_text::{H, W, lit, render, span};
use crate::FrameTimings;
use crate::interaction::InteractionState;
use crate::renderer::prelude::*;
use crate::tree::{EmitMode, LayerUse, Node, ProcessContext, layout_and_render};
use bmc_ui::{Color, PropsData, TextAlign, TextOverflow, TextStyle};
use taffy::prelude::TaffyTree;

const X: f32 = 10.0;
const Y: f32 = 10.0;
const BOX_W: f32 = 80.0;
const OVERWIDE: &str = "Grayscale Bitcoin Mini Trust ETF";

fn style(text_overflow: TextOverflow, align: TextAlign) -> TextStyle {
    TextStyle {
        size: 28,
        color: Color::from_rgb(255, 255, 255),
        align,
        text_overflow,
        ..TextStyle::default()
    }
}

fn lit_from_column(px: &[[u8; 4]], x0: usize) -> usize {
    px.chunks(W as usize)
        .map(|row| row[x0..].iter().filter(|p| lit(**p)).count())
        .sum()
}

fn lit_from_row(px: &[[u8; 4]], y0: usize) -> usize {
    px[y0 * W as usize..].iter().filter(|p| lit(**p)).count()
}

/// The first row a second line would ink:
/// one line box down, past the first line's descenders.
fn second_line_row(style: &TextStyle) -> usize {
    (Y + style.size as f32 * (style.line_height + 0.2)) as usize
}

/// The scissor's edge is anti-aliased across one pixel.
fn past_box() -> usize {
    (X + BOX_W) as usize + 1
}

#[test]
fn a_clipped_line_leaves_no_ink_past_its_box_or_below_it() {
    let style = style(TextOverflow::Clip, TextAlign::Left);
    let spans = [span(OVERWIDE, None)];
    let px = render(|r| r.draw_paragraph_clipped(&style, &spans, X, Y, BOX_W, 0.0, H as f32));

    assert!(lit_from_column(&px, 0) > 100, "BUG: the line drew nothing");
    assert_eq!(lit_from_column(&px, past_box()), 0, "ink past the box");
    assert_eq!(
        lit_from_row(&px, second_line_row(&style)),
        0,
        "the line wrapped"
    );
}

#[test]
fn an_ellipsized_line_stays_inside_its_box() {
    let style = style(TextOverflow::Ellipsis, TextAlign::Left);
    let spans = [span(OVERWIDE, None)];
    let px = render(|r| r.draw_paragraph(&style, &spans, X, Y, BOX_W));

    assert!(lit_from_column(&px, 0) > 100, "BUG: the line drew nothing");
    assert_eq!(lit_from_column(&px, past_box()), 0, "ink past the box");
    assert_eq!(
        lit_from_row(&px, second_line_row(&style)),
        0,
        "the line wrapped"
    );
}

/// A line that fits draws exactly as it would wrapped, in every alignment,
/// so switching a fitting label to `Clip` moves no pixels.
#[test]
fn a_fitting_clip_draws_as_wrap_does() {
    let spans = [span("Hash", None)];
    for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
        let draw = |overflow| {
            let style = style(overflow, align);
            render(|r| r.draw_paragraph(&style, &spans, X, Y, 200.0))
        };
        let wrapped = draw(TextOverflow::Wrap);

        assert!(
            lit_from_column(&wrapped, 0) > 100,
            "BUG: the line drew nothing"
        );
        assert!(
            draw(TextOverflow::Clip) == wrapped,
            "{align:?} moved pixels"
        );
    }
}

/// At `line_height: 1.0` descenders hang below the line box,
/// and the cut through the tree takes only what overruns sideways.
#[test]
fn a_clipped_row_keeps_its_descenders() {
    let style = TextStyle {
        size: 48,
        line_height: 1.0,
        ..style(TextOverflow::Clip, TextAlign::Left)
    };
    let spans = vec![span("gjpqy gjpqy", None)];
    let line_box_bottom = style.size as usize;

    let unscissored = render(|r| r.draw_paragraph(&style, &spans, 0.0, 0.0, W as f32));
    assert!(
        lit_from_row(&unscissored, line_box_bottom) > 0,
        "BUG: these descenders stay inside the line box, so nothing here is tested"
    );

    // Under a root column: the root itself is pinned to the whole frame.
    let row = Node::Row(
        PropsData {
            width: BOX_W,
            ..PropsData::default()
        },
        vec![Node::Paragraph {
            props: PropsData::default(),
            base_style: style,
            spans,
        }],
    );
    let tree = Node::Column(PropsData::default(), vec![row]);
    let px = render(|r| {
        let mut interaction = InteractionState::new();
        let (mut modal_states, mut scroll_states) = (HashMap::new(), HashMap::new());
        let (mut animation_states, mut transition_states) = (HashMap::new(), HashMap::new());
        let mut taffy = TaffyTree::new();
        let mut ctx = ProcessContext {
            interaction: &mut interaction,
            modal_states: &mut modal_states,
            scroll_states: &mut scroll_states,
            animation_states: &mut animation_states,
            transition_states: &mut transition_states,
            taffy: &mut taffy,
            frame_counter: 1,
            delta_ms: 16,
            now_unix_secs: 0,
            emit: EmitMode::All,
            static_layer: LayerUse::Ignore,
            static_layer_key: "text-overflow-pixels",
            layout_outlines: false,
            stroke_outlines: false,
            damage_rects: &[],
        };
        layout_and_render(
            &tree,
            W as f32,
            H as f32,
            r,
            &mut FrameTimings::default(),
            &mut ctx,
        )
        .expect("BUG: laying out a text row must succeed");
    });

    assert!(
        lit_from_row(&px, line_box_bottom) > 0,
        "the descenders were cut at the line box"
    );
    assert_eq!(
        lit_from_column(&px, BOX_W as usize + 1),
        0,
        "ink past the box"
    );
}
