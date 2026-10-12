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

//! Every glyph must be attributed to the span its characters actually came from,
//! across a hard break of any width and under soft wrap, and every glyph range
//! must slice the line it indexes. These run without a GL context, unlike the
//! end-to-end pixel regressions in `gpu::renderer`.

use std::collections::{BTreeSet, HashMap};

use cosmic_text::Buffer;

use super::{NO_SPAN, normalize_carriage_returns, shape_paragraph, span_for_glyph};
use crate::tree::{SpanData, TextStyle};

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

/// Shape exactly as the draw path does: normalize first, so the buffer is the
/// one whose glyphs `draw` would resolve.
fn shape(spans: &[SpanData], max_width: f32) -> Buffer {
    let normalized = normalize_carriage_returns(spans);
    let mut font_system = crate::gpu::renderer::build_font_system();
    let style = TextStyle::default();
    let (buffer, _, _) = shape_paragraph(&mut font_system, &style, &normalized, Some(max_width));
    buffer
}

/// Each hard line's text, in the same normalized text the draw path slices.
fn line_texts(spans: &[SpanData]) -> Vec<String> {
    shape(spans, 400.0)
        .lines
        .iter()
        .map(|line| line.text().to_owned())
        .collect()
}

/// Every character cosmic-text may split a hard line on — the `BidiClass::B`
/// set.
fn is_paragraph_separator(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2029}'
    )
}

/// Attributed by the character itself, so the oracle shares nothing with the
/// code under test. Separators are consumed into the line ending, and a space
/// is the one character the inputs below reuse across spans.
fn is_unattributable(c: char) -> bool {
    c == ' ' || is_paragraph_separator(c)
}

/// Resolve every glyph through [`span_for_glyph`] — the same call `draw`
/// makes — and assert it lands on the span holding its characters. Returns the
/// spans actually observed, so a caller can reject a vacuous pass.
///
/// The oracle is the character itself: each span in `spans` must draw from an
/// alphabet no other span uses, so the owning span follows from a glyph's text
/// alone. Nothing here reconstructs a byte offset, which is the point — that
/// arithmetic is what [`super::shape_paragraph`]'s tagging removed.
fn assert_span_attribution(spans: &[SpanData], max_width: f32) -> BTreeSet<usize> {
    let mut owner = HashMap::new();
    for (i, s) in spans.iter().enumerate() {
        for c in s.text.chars().filter(|c| !is_unattributable(*c)) {
            assert!(
                owner.insert(c, i).is_none_or(|prev| prev == i),
                "test input reuses {c:?} across spans, so it cannot attribute a glyph",
            );
        }
    }

    let buffer = shape(spans, max_width);
    let mut observed = BTreeSet::new();
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let text = run
                .text
                .get(glyph.start..glyph.end)
                .expect("glyph range must slice its own line");
            assert_ne!(
                glyph.metadata, NO_SPAN,
                "glyph {text:?} kept the default attrs instead of a span's",
            );
            let resolved = span_for_glyph(glyph.metadata, spans.len());
            observed.insert(resolved);
            for c in text.chars().filter(|c| !is_unattributable(*c)) {
                assert_eq!(
                    owner.get(&c).copied(),
                    Some(resolved),
                    "line {} glyph {text:?} resolved to span {resolved} ({:?})",
                    run.line_i,
                    spans[resolved].text,
                );
            }
        }
    }
    observed
}

/// Every glyph range must slice the line it indexes. `draw` uses `get`, so a
/// range landing out of bounds or inside a multi-byte character silently drops
/// text there; here it fails loudly instead.
fn assert_glyph_ranges_slice_their_line(spans: &[SpanData], max_width: f32) {
    let buffer = shape(spans, max_width);
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            assert!(
                run.text.get(glyph.start..glyph.end).is_some(),
                "line {} glyph {}..{} does not slice {:?}",
                run.line_i,
                glyph.start,
                glyph.end,
                run.text,
            );
        }
    }
}

/// The property `draw` rests on, in its plainest form: a span boundary at a
/// hard break and another mid-line, with no style overrides anywhere — so the
/// tag is the only thing distinguishing the three spans, both from each other
/// and from the paragraph's default attrs.
#[test]
fn each_glyph_resolves_to_the_span_holding_its_characters() {
    let spans = [span("aaa\n"), span("bbb"), span("ccc")];
    assert_eq!(
        assert_span_attribution(&spans, 400.0),
        BTreeSet::from([0, 1, 2])
    );
}

/// Every hard break must produce exactly one extra line and hand the text
/// after it to its own span — whichever separator the caller used, and whether
/// or not the text is ASCII, since cosmic-text takes a different code path for
/// each. A separator wider than one byte used to shift every offset after it;
/// nothing measures it now.
#[test]
fn every_hard_break_splits_once_and_keeps_span_attribution() {
    for sep in ["\n", "\r\n", "\r", "\u{85}", "\u{2029}"] {
        for prefix in ["aaa", "a\u{e9}a"] {
            let spans = [span(&format!("{prefix}{sep}")), span("ggg")];
            assert_eq!(
                line_texts(&spans).len(),
                2,
                "separator {sep:?} with prefix {prefix:?}: expected exactly 2 hard lines",
            );
            assert_eq!(
                assert_span_attribution(&spans, 400.0),
                BTreeSet::from([0, 1]),
                "separator {sep:?} with prefix {prefix:?}: a span drew no glyphs",
            );
        }
    }
}

/// A `\r\n` straddling a span boundary is still one break, not two — and an
/// empty span between the two halves must not drop the pending-CR state and
/// let the `\n` open a blank line of its own.
#[test]
fn crlf_split_across_spans_collapses_to_one_break() {
    let texts = line_texts(&[span("aaa\r"), span("\nggg")]);
    assert_eq!(texts.len(), 2, "expected one break, got {texts:?}");

    let spans = [span("aaa\r"), span(""), span("\nggg")];
    let texts = line_texts(&spans);
    assert_eq!(
        texts.len(),
        2,
        "empty span between CR and LF split one break into two, got {texts:?}",
    );
    assert_eq!(
        assert_span_attribution(&spans, 400.0),
        BTreeSet::from([0, 2])
    );
}

/// A blank hard line carries no glyphs of its own, so what must hold is that
/// it is a line — and that the text after it still belongs to its own span.
#[test]
fn blank_line_between_breaks_is_its_own_line() {
    let spans = [span("aaa\n\n"), span("ccc")];
    assert_eq!(line_texts(&spans), vec!["aaa", "", "ccc"]);
    assert_eq!(
        assert_span_attribution(&spans, 400.0),
        BTreeSet::from([0, 1])
    );
}

/// Guard against cosmic-text changing what it breaks on. Whatever it decides
/// to split, the text after the break must still resolve to its own span —
/// however wide the separator is, because nothing derives a span from an
/// offset any more. Only `\r` is normalized away first, so a newly-splitting
/// character needs no code change to stay correct here.
#[test]
fn every_splitting_separator_keeps_span_attribution() {
    let candidates = [
        '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
        '\u{2029}',
    ];
    for c in candidates {
        for prefix in ["aaa", "a\u{e9}a"] {
            let spans = [span(&format!("{prefix}{c}")), span("ggg")];
            if line_texts(&spans).len() < 2 {
                continue; // does not break — nothing to attribute across it
            }
            assert_eq!(
                assert_span_attribution(&spans, 400.0),
                BTreeSet::from([0, 1]),
                "U+{:04X} with prefix {prefix:?}: a span drew no glyphs",
                c as u32,
            );
        }
    }
}

/// A separator must never survive into a line's text, or `draw` would slice it
/// out of `run.text` and hand FemtoVG a control character to render.
/// `BidiParagraphs` trims one trailing separator per line, so the case worth
/// pinning is two adjacent separators of different kinds: each has to end a
/// line of its own rather than leave the first one sitting inline. Only `\r`
/// is normalized away beforehand, so the rest reach cosmic-text as they are.
#[test]
fn no_separator_survives_into_a_line() {
    let separators = [
        "\n",
        "\r\n",
        "\r",
        "\u{85}",
        "\u{2029}",
        "\u{85}\n",
        "\n\u{85}",
        "\u{2029}\n",
        "\n\u{2029}",
        "\u{85}\u{2029}",
        "\r\u{85}",
        "\u{85}\r",
        "\r\n\u{85}",
        "\n\n",
        "\u{85}\u{85}",
    ];
    for sep in separators {
        let spans = [span(&format!("aaa{sep}ggg"))];
        for text in line_texts(&spans) {
            assert!(
                !text.contains(is_paragraph_separator),
                "separator {sep:?} left {text:?} inside a line",
            );
        }
    }
}

/// Soft wrap puts many runs on one `line_i`, and a wrapped run's `run.text` is
/// still the whole hard line. Attribution has to hold per glyph regardless,
/// including for a span boundary that lands mid-line and one that lands after
/// a wrap.
#[test]
fn soft_wrap_keeps_span_attribution() {
    let spans = [
        span("aaa bbb ccc ddd eee "),
        span("ggg hhh iii"),
        span("\njjj kkk lll"),
    ];
    assert_eq!(
        assert_span_attribution(&spans, 60.0),
        BTreeSet::from([0, 1, 2])
    );
}

/// Every input the ticket lists as a confirmed trigger. Emoji and Hebrew shape
/// to no glyphs in the Braiins faces, so these prove the ranges stay sliceable
/// rather than proving attribution; the pixels are the GL tests' job.
#[test]
fn glyph_ranges_slice_their_line_for_every_ticket_input() {
    assert_glyph_ranges_slice_their_line(&[span("\u{201c}Fact\u{201d}\nSecond line")], 400.0);
    assert_glyph_ranges_slice_their_line(&[span("a\u{1f600}b\nsecond \u{1f600} line")], 400.0);
    assert_glyph_ranges_slice_their_line(&[span("e\u{301}cole\nde\u{301}ja\u{300} vu")], 400.0);
    assert_glyph_ranges_slice_their_line(&[span("\u{5e9}\u{5dc}\u{5d5}\u{5dd}\nworld")], 400.0);
    assert_glyph_ranges_slice_their_line(&[span("Red\r\n"), span("Green")], 400.0);
    assert_glyph_ranges_slice_their_line(&[span("\u{1f600}\r"), span("\nGreen \u{1f600}")], 400.0);
    // Narrow enough to soft-wrap both hard lines.
    assert_glyph_ranges_slice_their_line(
        &[span(
            "abc \u{5e9}\u{5dc}\u{5d5}\u{5dd} def ghi\nsecond \u{5e9}\u{5dc}\u{5d5}\u{5dd} line",
        )],
        60.0,
    );
    assert_glyph_ranges_slice_their_line(
        &[span("first\n"), span("aaaa bbbb cccc dddd"), span("EEEE")],
        60.0,
    );
}

/// Combining marks and typographic quotes must resolve like anything else.
/// They are only remarkable because their bytes outnumber their characters,
/// which is what made the offset shift they replaced so easy to get wrong —
/// and the ticket's device repro was exactly a quote before a hard break.
#[test]
fn multibyte_characters_resolve_to_their_own_span() {
    let spans = [
        span("e\u{301}\n"),
        span("a\u{300}"),
        span("\u{201c}u\u{201d}"),
    ];
    assert_eq!(
        assert_span_attribution(&spans, 400.0),
        BTreeSet::from([0, 1, 2])
    );
}
