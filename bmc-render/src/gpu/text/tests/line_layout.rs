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

//! Per-visual-line glyph extraction, the baseline contract,
//! and the single-line entry point.
//! CPU-only — nothing here needs a GL context.

#![expect(clippy::cast_precision_loss)]

use super::{font_system, line_style, sized_span};
use std::hash::Hasher as _;

use cosmic_text::{Attrs, Buffer, CacheKeyFlags, Family, FontSystem, Metrics, Shaping};

use super::{
    LayoutDomain, ParagraphLayoutCache, baseline_to_alphabetic, cache_key, extract_lines,
    layout_cache_hasher, shape_paragraph,
};
use crate::tree::{FontFamily, TextStyle};

#[test]
fn layout_domains_use_distinct_hash_namespaces() {
    let paragraph = layout_cache_hasher(LayoutDomain::Paragraph).finish();
    let single_line = layout_cache_hasher(LayoutDomain::SingleLine).finish();

    assert_ne!(
        paragraph, single_line,
        "shared cache domains must not start from the same hash state"
    );
}

const MIXED_BASE_SIZE: u32 = 40;
const MIXED_SMALL_SIZE: u32 = 14;
const MIXED_LEADING: f32 = 1.2;

/// Two spans of very different size, forced onto separate lines
/// by `max_width`, built through the same path a widget's paragraph takes.
/// Returns the shaped buffer with the width and height reported for it.
fn mixed_size_paragraph(font_system: &mut FontSystem) -> (Buffer, f32, f32) {
    let base = TextStyle {
        size: MIXED_BASE_SIZE,
        line_height: MIXED_LEADING,
        family: FontFamily::Sans,
        ..TextStyle::default()
    };
    let spans = [
        sized_span("tiny ", Some(MIXED_SMALL_SIZE)),
        sized_span("HUGE", None),
    ];
    shape_paragraph(font_system, &base, &spans, Some(130.0))
}

/// A span's own size drives its shaping, so a unit set beside
/// a number renders smaller than it without leaving the number's line
/// — the one thing a sibling node cannot do, having no baseline to share.
#[test]
fn a_span_shapes_at_its_own_size() {
    let mut font_system = font_system();
    let base = TextStyle {
        size: 40,
        family: FontFamily::Sans,
        ..TextStyle::default()
    };
    let sized = [sized_span("500,0", None), sized_span(" PH/s", Some(14))];
    let uniform = [sized_span("500,0", None), sized_span(" PH/s", None)];

    let (sized_buffer, sized_width, _) = shape_paragraph(&mut font_system, &base, &sized, None);
    let (_, uniform_width, _) = shape_paragraph(&mut font_system, &base, &uniform, None);

    assert_eq!(
        extract_lines(&sized_buffer).len(),
        1,
        "the pair belongs on one line"
    );
    assert!(
        sized_width < uniform_width,
        "the smaller unit must shape narrower: {sized_width} against {uniform_width}"
    );
}

/// A smaller span joins a line without moving it.
/// cosmic-text takes a line's height from the glyphs announcing one,
/// so a span leaving its metrics unset hands the line to a smaller sibling
/// — which centres the tall run against a box too short for it.
#[test]
#[expect(
    clippy::float_cmp,
    reason = "the unit must leave the line where it found it, to the bit"
)]
fn a_smaller_span_does_not_move_the_line_it_joins() {
    let mut font_system = font_system();
    let base = TextStyle {
        size: 64,
        line_height: 1.0,
        family: FontFamily::Sans,
        ..TextStyle::default()
    };
    let alone = [sized_span("500,0", None)];
    let with_unit = [sized_span("500,0", None), sized_span(" PH/s", Some(24))];

    let (alone_buffer, _, alone_height) = shape_paragraph(&mut font_system, &base, &alone, None);
    let (unit_buffer, _, unit_height) = shape_paragraph(&mut font_system, &base, &with_unit, None);
    let (alone_lines, unit_lines) = (extract_lines(&alone_buffer), extract_lines(&unit_buffer));
    let ([alone_line], [unit_line]) = (alone_lines.as_slice(), unit_lines.as_slice()) else {
        panic!("BUG: each pair belongs on one line");
    };

    assert_eq!(
        unit_line.baseline_y, alone_line.baseline_y,
        "the number's baseline moved when the unit joined it"
    );
    assert_eq!(
        unit_height, alone_height,
        "the paragraph changed height when the unit joined it"
    );
}

/// A span's size reaches the shaping, so it has to reach the cache key
/// standing in for it. Two paragraphs alike but for one span's size
/// share an entry otherwise, and whichever asks second gets handed
/// the glyphs of the first.
#[test]
fn the_cache_key_tells_two_span_sizes_apart() {
    let base = TextStyle::default();
    let inherited = [sized_span("500,0", None), sized_span(" PH/s", None)];
    let small = [sized_span("500,0", None), sized_span(" PH/s", Some(24))];
    let large = [sized_span("500,0", None), sized_span(" PH/s", Some(32))];

    assert_ne!(
        cache_key(&base, &inherited, None),
        cache_key(&base, &small, None),
        "a span that asked for a size collides with one that inherited it"
    );
    assert_ne!(
        cache_key(&base, &small, None),
        cache_key(&base, &large, None),
        "two span sizes collide with each other"
    );
}

/// A wrapped line's metrics come from the glyphs on that line,
/// not from the buffer or from the tallest line in the paragraph.
/// Otherwise small text following large text sits against the large ascent.
#[test]
#[expect(
    clippy::float_cmp,
    reason = "two ascents differing at all is the point"
)]
fn wrapped_line_uses_its_own_metrics() {
    let mut font_system = font_system();
    let (buffer, _, _) = mixed_size_paragraph(&mut font_system);
    let lines = extract_lines(&buffer);

    assert_eq!(lines.len(), 2, "the two spans must land on separate lines");
    assert_ne!(
        lines[0].max_ascent, lines[1].max_ascent,
        "the two lines must differ in ascent, or this proves nothing",
    );
    for (i, line) in lines.iter().enumerate() {
        assert!(!line.glyphs.is_empty(), "line {i} drew no glyphs");
        assert!(
            line.glyphs.iter().all(|g| g.metadata == i),
            "line {i} carries glyphs from another wrap",
        );
    }
}

/// A paragraph's height is the sum of its lines' own heights.
/// Counting lines at the paragraph's height would size the small line
/// as the tall one, and only a wrapped mixed-size paragraph can tell.
#[test]
fn a_wrapped_mixed_size_paragraph_is_as_tall_as_its_lines_together() {
    let mut font_system = font_system();
    let (buffer, _, height) = mixed_size_paragraph(&mut font_system);
    assert_eq!(extract_lines(&buffer).len(), 2, "the two spans must wrap");

    let small_line = MIXED_SMALL_SIZE as f32 * MIXED_LEADING;
    let tall_line = MIXED_BASE_SIZE as f32 * MIXED_LEADING;
    let summed = small_line + tall_line;
    let counted = 2.0 * tall_line;

    assert!(
        (height - summed).abs() < 0.01,
        "height {height} is not the lines' sum {summed}"
    );
    assert!(
        (height - counted).abs() > 1.0,
        "height {height} is the line count at one height, {counted}"
    );
}

/// A paragraph whose one line holds only a span smaller than itself
/// is as short as that span: `extract_lines` draws it so, and a height
/// padded up to the base line would have the parent reserve blank room.
#[test]
fn a_paragraph_of_only_a_smaller_span_is_as_short_as_that_span() {
    let mut font_system = font_system();
    let base = TextStyle {
        size: MIXED_BASE_SIZE,
        line_height: MIXED_LEADING,
        family: FontFamily::Sans,
        ..TextStyle::default()
    };
    let spans = [sized_span("tiny", Some(MIXED_SMALL_SIZE))];
    let (_, _, height) = shape_paragraph(&mut font_system, &base, &spans, None);

    let small_line = MIXED_SMALL_SIZE as f32 * MIXED_LEADING;
    assert!(
        (height - small_line).abs() < 0.01,
        "height {height} is not the small span's own line, {small_line}"
    );
}

/// Every femtovg baseline must convert to the alphabetic baseline
/// that glyph positions are relative to; a wrong sign here shifts a line.
#[test]
#[expect(clippy::float_cmp, reason = "the conversion is exact arithmetic")]
fn baseline_conversion_matrix() {
    let (max_ascent, max_descent, y) = (10.0_f32, 4.0_f32, 100.0_f32);
    assert_eq!(
        baseline_to_alphabetic(y, femtovg::Baseline::Alphabetic, max_ascent, max_descent),
        100.0,
    );
    assert_eq!(
        baseline_to_alphabetic(y, femtovg::Baseline::Top, max_ascent, max_descent),
        110.0,
    );
    assert_eq!(
        baseline_to_alphabetic(y, femtovg::Baseline::Bottom, max_ascent, max_descent),
        96.0,
    );
    assert_eq!(
        baseline_to_alphabetic(y, femtovg::Baseline::Middle, max_ascent, max_descent),
        103.0,
    );
}

/// Fractional sizes must key distinct entries.
/// This is why the single-line path exists at all:
/// `TextStyle`'s `u32` size would collapse an animated 17.5 px onto 17 px.
#[test]
fn fractional_sizes_get_distinct_entries() {
    let mut font_system = font_system();
    let mut cache = ParagraphLayoutCache::new();

    let entry = cache.layout_single_line(&mut font_system, line_style(17.0, false), "size");
    let whole_size = entry.lines[0].glyphs[0].key.font_size_bits;
    let entry = cache.layout_single_line(&mut font_system, line_style(17.5, false), "size");
    let fractional_size = entry.lines[0].glyphs[0].key.font_size_bits;

    assert_eq!(
        cache.entries.len(),
        2,
        "a half-pixel step must not collapse"
    );
    assert_eq!(
        whole_size,
        17.0_f32.to_bits(),
        "the whole size must reach the glyph key unrounded",
    );
    assert_eq!(
        fractional_size,
        17.5_f32.to_bits(),
        "the fractional size must reach the glyph key unrounded",
    );
}

/// Every shipped face is upright, so an italic single-line style renders
/// as italic only if cosmic-text flags its glyphs `FAKE_ITALIC`
/// and the rasterizer skews them.
#[test]
fn italic_line_style_emits_fake_italic_keys() {
    let mut font_system = font_system();
    let mut cache = ParagraphLayoutCache::new();

    for italic in [false, true] {
        let entry = cache.layout_single_line(&mut font_system, line_style(24.0, italic), "Slanted");
        let glyphs: Vec<_> = entry.lines.iter().flat_map(|l| l.glyphs.iter()).collect();
        assert!(!glyphs.is_empty(), "italic={italic} shaped no glyphs");
        assert_eq!(
            glyphs
                .iter()
                .all(|g| g.key.flags.contains(CacheKeyFlags::FAKE_ITALIC)),
            italic,
            "italic={italic} did not reach the glyph keys",
        );
    }
}

/// Line height is leading, not a metric of the glyphs.
/// Folding half-leading into ascent or descent would move every baseline
/// the moment a widget asks for a non-default line height.
#[test]
fn non_default_line_height_does_not_shift_baselines() {
    let mut font_system = font_system();
    let metrics_of = |font_system: &mut FontSystem, scale: f32| {
        let mut buffer = Buffer::new(font_system, Metrics::new(24.0, 24.0 * scale));
        buffer.set_text(
            font_system,
            "Leading",
            &Attrs::new().family(Family::Name("Braiins Sans")),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        let lines = extract_lines(&buffer);
        assert_eq!(lines.len(), 1);
        (lines[0].max_ascent, lines[0].max_descent)
    };

    assert_eq!(
        metrics_of(&mut font_system, 1.0),
        metrics_of(&mut font_system, 2.0),
    );
}

/// A mark's placement lives in `x_offset`/`y_offset`, which the cache key
/// already folds in. Dropping them from the position
/// would stack every diacritic on its base letter's origin.
///
/// The bases below have no precomposed form,
/// so the shaper cannot collapse base and mark into a single glyph.
#[test]
#[expect(
    clippy::float_cmp,
    reason = "the folding must reproduce LayoutGlyph::physical bit for bit"
)]
fn nonzero_offsets_fold_into_position() {
    let mut font_system = font_system();
    let mut offset_x = 0_usize;
    let mut offset_y = 0_usize;

    for family in ["Braiins Sans", "Braiins Deck Sans", "Noto Sans"] {
        for text in ["b\u{301}", "q\u{308}", "e\u{301}\u{301}", "\u{25cc}\u{301}"] {
            let mut buffer = Buffer::new(&mut font_system, Metrics::new(32.0, 32.0));
            buffer.set_text(
                &mut font_system,
                text,
                &Attrs::new().family(Family::Name(family)),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut font_system, false);
            let extracted = extract_lines(&buffer);

            let raw: Vec<_> = buffer
                .lines
                .iter()
                .filter_map(|line| line.layout_opt())
                .flatten()
                .flat_map(|layout| layout.glyphs.iter())
                .collect();
            let positioned: Vec<_> = extracted.iter().flat_map(|l| l.glyphs.iter()).collect();
            assert_eq!(raw.len(), positioned.len());

            for (glyph, info) in raw.iter().zip(&positioned) {
                assert_eq!(info.x, glyph.x + glyph.font_size * glyph.x_offset);
                assert_eq!(info.y, glyph.y - glyph.font_size * glyph.y_offset);
                if glyph.x_offset != 0.0 {
                    assert_ne!(info.x, glyph.x, "{family} {text:?}: x offset was dropped");
                    offset_x += 1;
                }
                if glyph.y_offset != 0.0 {
                    assert_ne!(info.y, glyph.y, "{family} {text:?}: y offset was dropped");
                    offset_y += 1;
                }
            }
        }
    }

    assert!(
        offset_x > 0 && offset_y > 0,
        "no shipped face offset a mark in both axes, so nothing was proven",
    );
}
