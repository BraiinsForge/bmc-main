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

//! Clipped and ellipsized lines: where they cut, how they align,
//! and that the cache key tells them apart.
//! CPU-only — nothing here needs a GL context.

use super::{font_system, sized_span};

use cosmic_text::{Buffer, FontSystem};

use super::{cache_key, extract_lines, shape_paragraph};
use bmc_ui::{FontFamily, TextAlign, TextOverflow, TextStyle};

const OVERWIDE: &str = "Grayscale Bitcoin Mini Trust ETF";
const NARROW_BOX: f32 = 120.0;

fn overflow_style(text_overflow: TextOverflow, align: TextAlign) -> TextStyle {
    TextStyle {
        size: 20,
        family: FontFamily::Sans,
        align,
        text_overflow,
        ..TextStyle::default()
    }
}

/// Which glyphs of the first line are the ellipsis mark,
/// the only glyphs cosmic-text gives an empty byte range.
fn ellipsis_marks(buffer: &Buffer) -> Vec<bool> {
    buffer
        .layout_runs()
        .next()
        .expect("BUG: the paragraph laid out no line")
        .glyphs
        .iter()
        .map(|glyph| glyph.start == glyph.end)
        .collect()
}

#[test]
fn a_clipped_line_does_not_wrap() {
    let mut font_system = font_system();
    let style = overflow_style(TextOverflow::Clip, TextAlign::Left);
    let (buffer, width, _) = shape_paragraph(
        &mut font_system,
        &style,
        &[sized_span(OVERWIDE, None)],
        Some(NARROW_BOX),
    );

    assert_eq!(extract_lines(&buffer).len(), 1, "a clipped line wrapped");
    assert!(
        width > NARROW_BOX,
        "the line must keep its full width for the draw to cut, got {width}"
    );
}

#[test]
fn an_ellipsized_line_fits_its_box_and_ends_in_the_ellipsis() {
    let mut font_system = font_system();
    let style = overflow_style(TextOverflow::Ellipsis, TextAlign::Left);
    let spans = [sized_span(OVERWIDE, None)];
    let (buffer, width, _) = shape_paragraph(&mut font_system, &style, &spans, Some(NARROW_BOX));
    let lines = extract_lines(&buffer);
    let [line] = lines.as_slice() else {
        panic!("an ellipsized paragraph wrapped into {} lines", lines.len());
    };
    let last = line
        .glyphs
        .last()
        .expect("BUG: the cut line kept no glyphs");
    let marks = ellipsis_marks(&buffer);

    assert!(width <= NARROW_BOX, "{width} overruns the box");
    assert_eq!(
        marks.last(),
        Some(&true),
        "the line does not end in the ellipsis"
    );
    assert!(!marks[0], "the ellipsis replaced the whole line");
    assert!(
        last.metadata < spans.len(),
        "the ellipsis carries no span to take its paint from"
    );
}

/// cosmic-text shapes the ellipsis from the attributes at its line's first byte,
/// so a cut inside a later span still ends in the opening span's mark.
#[test]
fn an_ellipsis_takes_the_style_of_the_span_opening_its_line() {
    let mut font_system = font_system();
    let style = TextStyle {
        size: 40,
        ..overflow_style(TextOverflow::Ellipsis, TextAlign::Left)
    };
    let spans = [
        sized_span("21", None),
        sized_span(" million BTC, the supply cap", Some(20)),
    ];
    let (buffer, _, _) = shape_paragraph(&mut font_system, &style, &spans, Some(160.0));
    let lines = extract_lines(&buffer);
    let glyphs = &lines[0].glyphs;
    let (mark, cut) = glyphs
        .split_last()
        .expect("BUG: the cut line kept no glyphs");

    assert_eq!(
        ellipsis_marks(&buffer).last(),
        Some(&true),
        "BUG: no ellipsis"
    );
    assert!(
        cut.iter().any(|glyph| glyph.metadata == 1),
        "BUG: the cut fell inside the opening span, so this proves nothing"
    );
    assert_eq!((mark.metadata, mark.font_size), (0, 40.0));
}

#[test]
fn an_ellipsis_with_no_room_is_all_that_is_left() {
    let mut font_system = font_system();
    let style = overflow_style(TextOverflow::Ellipsis, TextAlign::Left);
    let (buffer, width, _) = shape_paragraph(
        &mut font_system,
        &style,
        &[sized_span(OVERWIDE, None)],
        Some(0.0),
    );
    let marks = ellipsis_marks(&buffer);

    assert!(
        !marks.is_empty() && marks.iter().all(|mark| *mark),
        "something beside the ellipsis survived: {marks:?}"
    );
    assert!(width > 0.0, "the lone ellipsis measured no width");
}

#[test]
fn an_ellipsis_with_room_leaves_the_line_alone() {
    let mut font_system = font_system();
    let spans = [sized_span(OVERWIDE, None)];
    let shape = |font_system: &mut FontSystem, overflow| {
        let style = overflow_style(overflow, TextAlign::Left);
        let (buffer, width, _) = shape_paragraph(font_system, &style, &spans, Some(1_000.0));
        let glyphs: Vec<u16> = extract_lines(&buffer)
            .iter()
            .flat_map(|line| line.glyphs.iter().map(|g| g.glyph_id))
            .collect();
        (glyphs, width.to_bits())
    };

    assert_eq!(
        shape(&mut font_system, TextOverflow::Ellipsis),
        shape(&mut font_system, TextOverflow::Wrap)
    );
}

#[test]
fn a_right_aligned_ellipsis_ends_at_its_box() {
    let mut font_system = font_system();
    let style = overflow_style(TextOverflow::Ellipsis, TextAlign::Right);
    let (buffer, _, _) = shape_paragraph(
        &mut font_system,
        &style,
        &[sized_span(OVERWIDE, None)],
        Some(NARROW_BOX),
    );
    let lines = extract_lines(&buffer);
    let last = lines[0]
        .glyphs
        .last()
        .expect("BUG: the cut line kept no glyphs");

    assert!(
        (last.x + last.w - NARROW_BOX).abs() < 1.0,
        "the ellipsis ends at {}, not at the box's right edge",
        last.x + last.w
    );
}

/// An overflowing line is start-aligned, as CSS does it,
/// so right-aligned text that overruns is cut on the right.
#[test]
fn an_overflowing_right_aligned_clip_starts_at_the_left() {
    let mut font_system = font_system();
    let style = overflow_style(TextOverflow::Clip, TextAlign::Right);
    let (buffer, _, _) = shape_paragraph(
        &mut font_system,
        &style,
        &[sized_span(OVERWIDE, None)],
        Some(NARROW_BOX),
    );
    let lines = extract_lines(&buffer);

    assert!(
        lines[0].glyphs[0].x.abs() < 1.0,
        "the overflowing line starts at {}",
        lines[0].glyphs[0].x
    );
}

#[test]
fn a_hard_break_still_breaks_a_clipped_paragraph() {
    let mut font_system = font_system();
    let style = overflow_style(TextOverflow::Clip, TextAlign::Left);
    let (buffer, _, _) = shape_paragraph(
        &mut font_system,
        &style,
        &[sized_span("first\nsecond", None)],
        Some(NARROW_BOX),
    );

    assert_eq!(extract_lines(&buffer).len(), 2);
}

#[test]
fn the_cache_key_tells_overflow_modes_apart() {
    let spans = [sized_span(OVERWIDE, None)];
    let key = |overflow| {
        cache_key(
            &overflow_style(overflow, TextAlign::Left),
            &spans,
            Some(NARROW_BOX),
        )
    };
    let keys = [
        key(TextOverflow::Wrap),
        key(TextOverflow::Clip),
        key(TextOverflow::Ellipsis),
    ];

    assert_ne!(keys[0], keys[1], "wrap and clip share a layout");
    assert_ne!(keys[0], keys[2], "wrap and ellipsis share a layout");
    assert_ne!(keys[1], keys[2], "clip and ellipsis share a layout");
}
