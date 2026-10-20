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

//! What one line hands to the draw path, span by span.
//! CPU-only — nothing here needs a GL context.

use super::{
    LineGlyphs, SpanGroup, extract_lines, normalize_carriage_returns, shape_paragraph, span_groups,
};
use crate::tree::{SpanData, TextStyle};

/// An RTL script the shipped faces do not cover,
/// so its glyphs come back as boxes —
/// the bidi reordering under test is the shaper's, not the font's,
/// and holds either way.
const HEBREW: &str = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}";

fn span(text: &str, underline: bool) -> SpanData {
    SpanData {
        text: text.to_owned(),
        weight: None,
        color: None,
        size: None,
        italic: false,
        underline,
        strikethrough: false,
    }
}

fn only_line(spans: &[SpanData]) -> LineGlyphs {
    let normalized = normalize_carriage_returns(spans);
    let mut font_system = super::font_system();
    let (buffer, _, _) = shape_paragraph(
        &mut font_system,
        &TextStyle::default(),
        &normalized,
        Some(400.0),
    );
    let mut lines = extract_lines(&buffer);
    assert_eq!(lines.len(), 1, "the fixture must shape to one line");
    lines.remove(0)
}

/// Every glyph of the group must sit inside the extent its decorations use.
fn assert_covers_its_glyphs(group: &SpanGroup<'_>) {
    for glyph in group.glyphs {
        assert!(
            glyph.x >= group.start_x && glyph.x + glyph.w <= group.start_x + group.width,
            "glyph at {}..{} escapes the extent {}..{}",
            glyph.x,
            glyph.x + glyph.w,
            group.start_x,
            group.start_x + group.width,
        );
    }
}

/// cosmic emits an RTL run's glyphs at descending x,
/// so first-glyph to last-glyph reads as a zero or negative width
/// and the underline vanishes.
#[test]
fn rtl_span_extent_covers_its_glyphs() {
    let spans = [span(HEBREW, true)];
    let line = only_line(&spans);
    let groups = span_groups(&line.glyphs, spans.len());

    let [group] = groups.as_slice() else {
        panic!("BUG: one span must yield one group");
    };
    assert!(group.glyphs.len() > 1, "the fixture must shape >1 glyph");
    assert!(
        group.glyphs[0].x > group.glyphs[group.glyphs.len() - 1].x,
        "the fixture must shape right-to-left",
    );
    assert!(group.width > 0.0, "the extent is {} wide", group.width);
    assert_covers_its_glyphs(group);
}

/// A bidi line's two spans each own a disjoint stretch of the line,
/// in the visual order the extents place them.
#[test]
fn bidi_spans_get_disjoint_ordered_extents() {
    let spans = [span("abc ", true), span(HEBREW, true)];
    let line = only_line(&spans);
    let groups = span_groups(&line.glyphs, spans.len());

    let [latin, hebrew] = groups.as_slice() else {
        panic!("BUG: two spans must yield two groups");
    };
    assert_eq!((latin.span, hebrew.span), (0, 1));
    assert!(
        latin.start_x + latin.width <= hebrew.start_x,
        "extents {}..{} and {}..{} overlap",
        latin.start_x,
        latin.start_x + latin.width,
        hebrew.start_x,
        hebrew.start_x + hebrew.width,
    );
    assert_covers_its_glyphs(latin);
    assert_covers_its_glyphs(hebrew);
}
