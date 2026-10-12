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
use crate::renderer::test_support::{DrawnParagraph, ShapingRecorder};

const FRAME_W: f32 = 400.0;
const FRAME_H: f32 = 200.0;
const OVERWIDE: &str = "Grayscale Bitcoin Mini Trust ETF";

fn style(text_overflow: TextOverflow) -> TextStyle {
    TextStyle {
        size: 20,
        text_overflow,
        ..TextStyle::default()
    }
}

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

fn para(base_style: TextStyle, text: &str) -> TreeNode {
    TreeNode::Paragraph {
        props: PropsData::default(),
        base_style,
        spans: vec![span(text)],
    }
}

fn sized(width: f32) -> PropsData {
    PropsData {
        width,
        ..PropsData::default()
    }
}

/// Lay `tree` out in a fresh frame, returning every paragraph drawn.
///
/// The frame's root takes the whole frame whatever its props say,
/// so `tree` goes under one for its own width to hold.
fn render(tree: &TreeNode) -> Vec<DrawnParagraph> {
    let root = TreeNode::Column(PropsData::default(), vec![tree.clone()]);
    let mut recorder = ShapingRecorder::new(FRAME_W, FRAME_H);
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
        static_layer_key: "text-overflow-test",
        damage_rects: &[],
    };
    let mut timings = FrameTimings::default();
    layout_and_render(
        &root,
        FRAME_W,
        FRAME_H,
        &mut recorder,
        &mut timings,
        &mut ctx,
    )
    .expect("BUG: laying out a text row must succeed");
    recorder.paragraphs
}

fn natural_width(base_style: TextStyle, text: &str) -> f32 {
    ShapingRecorder::default()
        .measure_paragraph(&base_style, &[span(text)], None)
        .0
}

const FULL_HEIGHT: Option<(f32, f32)> = Some((0.0, FRAME_H));

#[test]
fn an_overwide_clip_shrinks_to_its_row_and_is_clipped() {
    let drawn = render(&TreeNode::Row(
        sized(100.0),
        vec![para(style(TextOverflow::Clip), OVERWIDE)],
    ));

    assert!(
        drawn[0].max_width <= 100.0,
        "drawn {} wide",
        drawn[0].max_width
    );
    assert_eq!(drawn[0].clip, FULL_HEIGHT, "the overflow was not cut");
}

#[test]
fn a_clip_that_fits_is_drawn_unclipped() {
    let base_style = style(TextOverflow::Clip);
    let drawn = render(&TreeNode::Row(
        sized(FRAME_W),
        vec![para(base_style, "Hash")],
    ));

    assert!(
        (drawn[0].max_width - natural_width(base_style, "Hash")).abs() < 0.5,
        "a fitting line takes its own width, got {}",
        drawn[0].max_width
    );
    assert_eq!(drawn[0].clip, None);
}

#[test]
fn two_clipped_siblings_share_the_shrink() {
    let base_style = style(TextOverflow::Clip);
    let drawn = render(&TreeNode::Row(
        sized(100.0),
        vec![para(base_style, OVERWIDE), para(base_style, OVERWIDE)],
    ));

    assert!(
        drawn[0].max_width + drawn[1].max_width <= 101.0,
        "the pair overruns its row: {} + {}",
        drawn[0].max_width,
        drawn[1].max_width
    );
    assert!(
        drawn
            .iter()
            .all(|p| p.max_width > 0.0 && p.clip == FULL_HEIGHT)
    );
}

#[test]
fn max_width_caps_a_clip_stretched_across_a_column() {
    let base_style = TextStyle {
        max_width: 80,
        ..style(TextOverflow::Clip)
    };
    let drawn = render(&TreeNode::Column(
        sized(300.0),
        vec![para(base_style, OVERWIDE)],
    ));

    assert!(
        (drawn[0].max_width - 80.0).abs() < 0.5,
        "drawn {} wide",
        drawn[0].max_width
    );
    assert_eq!(drawn[0].clip, FULL_HEIGHT);
}

/// The cap narrows the box a stretching column lays the text into,
/// so every mode aligns a fitting line inside that same box.
#[test]
fn a_capped_right_aligned_line_lands_in_one_box_under_every_mode() {
    let boxes: Vec<(f32, f32)> = [
        TextOverflow::Wrap,
        TextOverflow::Clip,
        TextOverflow::Ellipsis,
    ]
    .into_iter()
    .map(|text_overflow| {
        let base_style = TextStyle {
            max_width: 120,
            align: TextAlign::Right,
            ..style(text_overflow)
        };
        let drawn = render(&TreeNode::Column(
            sized(300.0),
            vec![para(base_style, "Hash")],
        ));
        (drawn[0].x, drawn[0].max_width)
    })
    .collect();

    assert_eq!(boxes, [(0.0, 120.0); 3]);
}

#[test]
fn the_shrink_reaches_through_a_column_in_a_row() {
    let drawn = render(&TreeNode::Row(
        sized(100.0),
        vec![TreeNode::Column(
            PropsData::default(),
            vec![para(style(TextOverflow::Ellipsis), OVERWIDE)],
        )],
    ));

    assert!(
        drawn[0].max_width <= 100.0,
        "drawn {} wide",
        drawn[0].max_width
    );
}

#[test]
fn a_clipped_paragraph_stays_one_line_tall() {
    let base_style = style(TextOverflow::Clip);
    let drawn = render(&TreeNode::Column(
        sized(100.0),
        vec![
            para(base_style, OVERWIDE),
            para(style(TextOverflow::Wrap), "after"),
        ],
    ));
    let (_, one_line) =
        ShapingRecorder::default().measure_paragraph(&base_style, &[span("Hash")], None);

    assert!(
        (drawn[1].y - drawn[0].y - one_line).abs() < 0.5,
        "the next paragraph starts {} below, not one line ({one_line})",
        drawn[1].y - drawn[0].y
    );
}

#[test]
fn an_ellipsis_squeezed_to_nothing_keeps_its_ellipsis() {
    let base_style = style(TextOverflow::Ellipsis);
    let (ellipsis_only, _) =
        ShapingRecorder::default().measure_paragraph(&base_style, &[span(OVERWIDE)], Some(0.0));
    let drawn = render(&TreeNode::Row(sized(1.0), vec![para(base_style, OVERWIDE)]));

    assert!(
        (drawn[0].max_width - ellipsis_only).abs() < 0.5,
        "drawn {} wide, not the ellipsis's {ellipsis_only}",
        drawn[0].max_width
    );
}

#[test]
fn wrap_still_wraps() {
    let base_style = style(TextOverflow::Wrap);
    let drawn = render(&TreeNode::Column(
        sized(100.0),
        vec![para(base_style, OVERWIDE), para(base_style, "after")],
    ));
    let (_, one_line) =
        ShapingRecorder::default().measure_paragraph(&base_style, &[span("Hash")], None);

    assert_eq!(drawn[0].clip, None);
    assert!(
        drawn[1].y - drawn[0].y > one_line * 1.5,
        "the wrapped paragraph kept to one line"
    );
}
