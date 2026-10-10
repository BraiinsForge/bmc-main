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

//! Shaping text with cosmic-text and reading its lines and span groups back.

#![expect(clippy::cast_precision_loss)]

use bmc_ui::{FontFamily, FontWeight, Span, TextAlign, TextOverflow, TextStyle};
use cosmic_text::{
    Align, Attrs, Buffer, Color, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, LayoutGlyph,
    Metrics, Scroll, Shaping, Style, Weight, Wrap,
};

/// Metadata value on the paragraph's default attrs. Every span carries its own
/// index instead, so no glyph should ever come back tagged with this.
///
/// Deliberately not `0`, for detectability rather than correctness.
/// `set_rich_text` records a span only when its attrs differ from the defaults,
/// and `Attrs` equality covers `metadata`, so a `0` default could only ever skip
/// span 0 styled exactly like the base — whose glyphs then inherit `0` and
/// resolve back to span 0 anyway. What the distinct default buys is that an
/// untagged glyph surfaces as `NO_SPAN` and gets logged, instead of
/// masquerading as span 0.
pub(super) const NO_SPAN: usize = usize::MAX;

/// Shape a paragraph using cosmic-text. Returns (buffer, width, height).
///
/// Each span is tagged with its own index through [`Attrs::metadata`], which
/// cosmic-text copies into every glyph it shapes from that span. That is what
/// lets [`super::ParagraphLayoutCache::draw`] recover a glyph's span directly, instead
/// of mapping the glyph's line-relative byte offset back into the concatenated
/// span text — a mapping that needs a per-line start table and cannot be derived
/// from the line endings, because `set_rich_text` stamps every line with
/// `LineEnding::default()` whatever separator actually split it.
pub(super) fn shape_paragraph(
    font_system: &mut FontSystem,
    base_style: &TextStyle,
    spans: &[Span],
    max_width: Option<f32>,
) -> (Buffer, f32, f32) {
    let line_height = base_style.size as f32 * base_style.line_height;
    let metrics = Metrics::new(base_style.size as f32, line_height);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, max_width, None);
    // `Clip` only stops the wrapping; cutting what overflows is the draw's scissor.
    match base_style.text_overflow {
        TextOverflow::Wrap => {}
        TextOverflow::Clip => buffer.set_wrap(font_system, Wrap::None),
        TextOverflow::Ellipsis => {
            buffer.set_wrap(font_system, Wrap::None);
            buffer.set_ellipsize(font_system, Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
        }
    }

    let rich_spans: Vec<_> = spans
        .iter()
        .enumerate()
        .map(|(i, span)| {
            let resolved = span.resolve_style(base_style);
            // Every span carries metrics, not just one that asked for its own size.
            // cosmic-text takes a line's height from the glyphs announcing one,
            // so a span left silent hands the whole line to a smaller sibling,
            // which centres the tall run against a box too short for it.
            let size = resolved.size as f32;
            let attrs = build_attrs(&resolved)
                .metadata(i)
                .metrics(Metrics::new(size, size * base_style.line_height));
            (span.text.as_str(), attrs)
        })
        .collect();

    buffer.set_rich_text(
        font_system,
        rich_spans,
        &build_attrs(base_style).metadata(NO_SPAN),
        Shaping::Advanced,
        None,
    );

    let cosmic_align = match base_style.align {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
    };
    for line in &mut buffer.lines {
        line.set_align(Some(cosmic_align));
    }

    buffer.shape_until_scroll(font_system, false);

    let width = buffer
        .layout_runs()
        .map(|run| run.line_w)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(0.0)
        .ceil();

    // A span carrying its own metrics makes its line taller
    // or shorter than the paragraph's, so the block is the sum
    // of the lines rather than a count times one height.
    // `extract_lines` places them by the same rule.
    let mut block_height = 0.0_f32;
    for buffer_line in &buffer.lines {
        let Some(layout) = buffer_line.layout_opt() else {
            break;
        };
        for layout_line in layout {
            block_height += layout_line.line_height_opt.unwrap_or(line_height);
        }
    }

    // Only a paragraph with no lines at all stands one base line tall;
    // one whose lines are all short is as short as they are.
    let height = if block_height > 0.0 {
        block_height
    } else {
        line_height
    };

    (buffer, width, height)
}

/// Build cosmic_text Attrs from a resolved TextStyle.
#[must_use]
pub fn build_attrs(style: &TextStyle) -> Attrs<'static> {
    let family_name = match style.family {
        FontFamily::Sans => "Braiins Sans",
        FontFamily::DeckSans => "Braiins Deck Sans",
    };
    let mut attrs = Attrs::new()
        .family(Family::Name(family_name))
        .weight(Weight(u16::from(style.weight)));

    if style.italic {
        attrs = attrs.style(Style::Italic);
    }

    attrs.color(Color::rgba(
        style.color.red(),
        style.color.green(),
        style.color.blue(),
        style.color.alpha(),
    ))
}

/// Style of one uniformly styled line of text.
///
/// Carries the size as `f32`, which [`TextStyle`] cannot:
/// its `u32` size expresses neither a fractional nor an animated size.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineStyle {
    pub family: FontFamily,
    pub weight: FontWeight,
    pub italic: bool,
    pub size: f32,
}

/// Build cosmic_text Attrs for a single line. No colour: the single-line callers
/// paint through FemtoVG rather than through cosmic-text.
fn line_attrs(style: LineStyle) -> Attrs<'static> {
    let family_name = match style.family {
        FontFamily::Sans => "Braiins Sans",
        FontFamily::DeckSans => "Braiins Deck Sans",
    };
    let attrs = Attrs::new()
        .family(Family::Name(family_name))
        .weight(Weight(u16::from(style.weight)));

    // Every shipped face is upright, so `Style::Italic` is what makes cosmic-text
    // flag the glyphs `FAKE_ITALIC` and the rasterizer skew them.
    if style.italic {
        attrs.style(Style::Italic)
    } else {
        attrs
    }
}

/// Shape one uniformly styled line, unwrapped. Returns (buffer, width, height).
pub(super) fn shape_single_line(
    font_system: &mut FontSystem,
    style: LineStyle,
    text: &str,
) -> (Buffer, f32, f32) {
    let metrics = Metrics::new(style.size, style.size);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_text(
        font_system,
        text,
        &line_attrs(style),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);

    // Unrounded, unlike the paragraph path.
    // A single-line measurement feeds layout arithmetic
    // that a whole-pixel ceiling would visibly shift.
    let width = buffer
        .layout_runs()
        .map(|run| run.line_w)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(0.0);
    let line_count = buffer.layout_runs().count().max(1);
    let height = line_count as f32 * metrics.line_height;

    (buffer, width, height)
}

/// One visual line's glyphs, with the metrics needed to place it.
#[cfg_attr(test, derive(Clone))]
pub(crate) struct LineGlyphs {
    pub glyphs: Vec<PositionedGlyphInfo>,
    pub max_ascent: f32,
    pub max_descent: f32,
    /// Paragraph-relative alphabetic baseline of this visual line.
    pub baseline_y: f32,
}

/// One glyph, positioned relative to its line's alphabetic baseline.
#[cfg_attr(test, derive(Clone))]
pub(crate) struct PositionedGlyphInfo {
    pub key: cosmic_text::CacheKey,
    pub x: f32,
    /// Alphabetic-baseline-relative, with the shaper's offset folded in.
    pub y: f32,
    /// Advance width. Origins alone recover neither the last glyph's advance
    /// nor the extents of an RTL run.
    pub w: f32,
    pub font_id: cosmic_text::fontdb::ID,
    pub font_size: f32,
    pub flags: cosmic_text::CacheKeyFlags,
    pub glyph_id: u16,
    /// Span index, as tagged through `Attrs::metadata`.
    /// Selects the glyph's paint and decorations.
    pub metadata: usize,
}

/// Fold the shaper's offsets into the glyph position,
/// exactly as [`LayoutGlyph::physical`] does before deriving the cache key.
/// The key already accounts for them, so dropping them here
/// would ask for a mark's raster and then draw it on the base letter's origin.
fn positioned_glyph(glyph: &LayoutGlyph) -> PositionedGlyphInfo {
    PositionedGlyphInfo {
        key: glyph.physical((0.0, 0.0), 1.0).cache_key,
        x: glyph.x + glyph.font_size * glyph.x_offset,
        y: glyph.y - glyph.font_size * glyph.y_offset,
        w: glyph.w,
        font_id: glyph.font_id,
        font_size: glyph.font_size,
        flags: glyph.cache_key_flags,
        glyph_id: glyph.glyph_id,
        metadata: glyph.metadata,
    }
}

/// Extract every visual line of a shaped buffer, each with its own metrics.
///
/// Walks `LayoutLine`s directly rather than `layout_runs`,
/// which exposes neither `max_ascent` nor `max_descent` —
/// the two metrics the baseline contract is defined in terms of.
/// `baseline_y` reimplements `LayoutRunIter`'s vertical advance,
/// which is equivalent only for an unscrolled, height-unbounded buffer.
pub(crate) fn extract_lines(buffer: &Buffer) -> Vec<LineGlyphs> {
    assert_eq!(
        buffer.scroll(),
        Scroll::default(),
        "BUG: extract_lines cannot place the lines of a scrolled buffer"
    );
    assert!(
        buffer.size().1.is_none(),
        "BUG: extract_lines cannot place the lines of a height-bounded buffer"
    );

    let default_line_height = buffer.metrics().line_height;
    let mut lines = Vec::new();
    let mut line_top = 0.0_f32;
    for buffer_line in &buffer.lines {
        // `layout_runs` stops at the first unshaped line rather than skipping it.
        let Some(layout) = buffer_line.layout_opt() else {
            break;
        };
        for layout_line in layout {
            let line_height = layout_line.line_height_opt.unwrap_or(default_line_height);
            let glyph_height = layout_line.max_ascent + layout_line.max_descent;
            let centering_offset = (line_height - glyph_height) / 2.0;
            lines.push(LineGlyphs {
                glyphs: layout_line.glyphs.iter().map(positioned_glyph).collect(),
                max_ascent: layout_line.max_ascent,
                max_descent: layout_line.max_descent,
                baseline_y: line_top + centering_offset + layout_line.max_ascent,
            });
            line_top += line_height;
        }
    }

    #[cfg(debug_assertions)]
    assert_lines_match_layout_runs(buffer, &lines);

    lines
}

/// Pin the reimplemented vertical advance to the original,
/// which it matches only under the preconditions `extract_lines` asserts.
#[cfg(debug_assertions)]
#[expect(
    clippy::float_cmp,
    reason = "the accumulation must reproduce layout_runs bit for bit"
)]
fn assert_lines_match_layout_runs(buffer: &Buffer, lines: &[LineGlyphs]) {
    let runs: Vec<f32> = buffer.layout_runs().map(|run| run.line_y).collect();
    debug_assert_eq!(
        runs.len(),
        lines.len(),
        "BUG: extract_lines disagrees with layout_runs on the line count"
    );
    for (line_i, (run_y, line)) in runs.iter().zip(lines).enumerate() {
        debug_assert_eq!(
            *run_y, line.baseline_y,
            "BUG: extract_lines disagrees with layout_runs on line {line_i}'s baseline"
        );
    }
}

/// Convert a FemtoVG baseline anchor to the alphabetic baseline
/// that [`PositionedGlyphInfo::y`] is relative to.
///
/// Derived from the line's own `max_ascent`/`max_descent`.
/// Never from `line_y − line_top`, which carries half of the line's leading.
pub(crate) fn baseline_to_alphabetic(
    y: f32,
    baseline: femtovg::Baseline,
    max_ascent: f32,
    max_descent: f32,
) -> f32 {
    match baseline {
        femtovg::Baseline::Alphabetic => y,
        femtovg::Baseline::Top => y + max_ascent,
        femtovg::Baseline::Bottom => y - max_descent,
        femtovg::Baseline::Middle => y + (max_ascent - max_descent) / 2.0,
    }
}

/// Rewrite `\r` and `\r\n` to a single `\n` before shaping, so a carriage return
/// breaks the line the same way whichever cosmic-text code path the text takes.
///
/// CR is the one separator cosmic-text is inconsistent about. `BidiParagraphs`'
/// ASCII fast path splits only on `\n`, so a bare CR renders inline as a glyph on
/// one line — while the same text plus one non-ASCII character takes the
/// `BidiInfo` path and does break. `\r\n` is worse: ASCII gets one break,
/// non-ASCII gets a break plus a spurious empty line.
///
/// Borrows unchanged when there is no CR, which is the common case. Span count
/// and order are preserved, so span indices stay valid for styling.
pub(super) fn normalize_carriage_returns(spans: &[Span]) -> std::borrow::Cow<'_, [Span]> {
    if !spans.iter().any(|s| s.text.contains('\r')) {
        return std::borrow::Cow::Borrowed(spans);
    }

    // A `\r\n` split across a span boundary must still collapse to one break,
    // so the trailing-CR state carries into the next span.
    let mut pending_cr = false;
    let normalized = spans
        .iter()
        .map(|span| {
            let mut chars = span.text.chars().peekable();
            if pending_cr && chars.peek() == Some(&'\n') {
                chars.next();
            }
            // If there's an empty span between one trailing \r and one
            // with leading \n, there'll be a spurious blank line:
            //
            // spans = ["a\r", "", "\nb"]
            // normalized full_text = "a\n\nb"
            // buffer lines         = ["a", "", "b"] // expected ["a", "b"]
            //
            // Hence, we ignore empty spans.
            if !span.text.is_empty() {
                pending_cr = span.text.ends_with('\r');
            }

            let mut text = String::with_capacity(span.text.len());
            while let Some(c) = chars.next() {
                match c {
                    '\r' => {
                        text.push('\n');
                        // Consume the LF of a CRLF pair: one break, not two.
                        if chars.peek() == Some(&'\n') {
                            chars.next();
                        }
                    }
                    _ => text.push(c),
                }
            }
            Span {
                text,
                ..span.clone()
            }
        })
        .collect();
    std::borrow::Cow::Owned(normalized)
}

/// Span owning a glyph tagged `metadata`, as tagged by [`shape_paragraph`].
///
/// A glyph outside `0..span_count` — [`NO_SPAN`] included — would mean it was
/// shaped from attrs we never tagged. Report it and fall back to the first span:
/// a panic here tears down the widget slot, and dropping the glyph would lose
/// text rather than just mis-style it.
///
/// Kept as its own function so `span_attribution_tests` can exercise the draw
/// path's span lookup without a GL context.
pub(super) fn span_for_glyph(metadata: usize, span_count: usize) -> usize {
    if metadata < span_count {
        return metadata;
    }
    tracing::error!("text: glyph metadata {metadata} is not one of the {span_count} span indices");
    0
}

/// One span's contiguous glyphs on a line, with what drawing them needs.
pub(crate) struct SpanGroup<'a> {
    pub span: usize,
    pub glyphs: &'a [PositionedGlyphInfo],
    /// Line-relative left edge of the decorations under this group.
    pub start_x: f32,
    /// Decoration width, spanning every glyph of the group.
    pub width: f32,
    /// The size these glyphs were shaped at, which the delegated path
    /// must paint with for femtovg to scale the outlines the same.
    pub font_size: f32,
}

/// Split a line into per-span groups, in draw order.
///
/// The extent is `min(x)` to `max(x + w)` over the whole group, not
/// first glyph to last: cosmic emits an RTL run's glyphs at descending x
/// (`shape.rs:2708-2717`), which first-to-last reads as a negative width.
pub(super) fn span_groups(glyphs: &[PositionedGlyphInfo], span_count: usize) -> Vec<SpanGroup<'_>> {
    glyphs
        .chunk_by(|a, b| a.metadata == b.metadata)
        .map(|group| {
            let first = group.first().expect("BUG: chunk_by yielded an empty group");
            let (start_x, end_x) = group.iter().fold((f32::MAX, f32::MIN), |(lo, hi), glyph| {
                (lo.min(glyph.x), hi.max(glyph.x + glyph.w))
            });
            SpanGroup {
                span: span_for_glyph(first.metadata, span_count),
                glyphs: group,
                start_x,
                width: end_x - start_x,
                font_size: first.font_size,
            }
        })
        .collect()
}
