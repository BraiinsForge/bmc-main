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

//! Paragraph layout (cosmic-text) and rendering (FemtoVG).
//!
//! cosmic-text handles shaping + line-breaking, FemtoVG renders on the GPU.
//! Both use rustybuzz internally with the same font binaries (Braiins Sans and
//! Braiins Deck Sans), so glyph advances agree — with one measured exception:
//! for a pair the font kerns, cosmic-text applies the kern and FemtoVG does
//! not, which leaves cosmic-text the tighter of the two.
//!
//! [`ParagraphLayoutCache::draw`] takes each segment's origin from cosmic-text
//! but lets FemtoVG lay out the glyphs inside it, so every additional draw call
//! is a point where the two can disagree by up to a pixel. Do not read a span
//! boundary's spacing as evidence about shaping.
//!
//! # Coordinate model
//!
//! cosmic-text's [`LayoutRun`] exposes two Y fields:
//!
//! - **`line_top`** — top of the line box (advances by `line_height` per line).
//! - **`line_y`** — the **alphabetic baseline**, computed as
//!   `line_top + centering_offset + max_ascent` where
//!   `centering_offset = (line_height − glyph_height) / 2`.
//!
//! We render each text segment with [`femtovg::Baseline::Alphabetic`] at
//! `run.line_y` so that vertical placement matches what cosmic-text's own
//! `buffer.draw()` / swash path would produce.  Decorations (underline,
//! strikethrough) are positioned relative to this baseline.
//!
//! [`LayoutRun`]: cosmic_text::LayoutRun

#![expect(clippy::cast_precision_loss)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, LayoutGlyph, Metrics, Shaping, Style, Weight,
};
use femtovg::{Canvas, FontId, Paint, renderer::OpenGl};

use crate::tree::{AutoFit, FontFamily, FontWeight, SpanData, TextAlign, TextStyle};

/// Three weight-variant fonts used to render text spans. The renderer picks
/// the closest match for each span's `weight` via [`WeightedFonts::select`].
#[derive(Clone, Copy, Debug)]
pub struct WeightedFonts {
    pub regular: FontId,
    pub semibold: FontId,
    pub bold: FontId,
}

impl WeightedFonts {
    /// Pick the FemtoVG font for a CSS weight. Thresholds match the named
    /// `FontWeight` constants so an intermediate weight (e.g. `FontWeight(500)`
    /// for Medium) falls onto the closest available asset.
    #[must_use]
    pub fn select(self, weight: FontWeight) -> FontId {
        if weight >= FontWeight::BOLD {
            self.bold
        } else if weight >= FontWeight::SEMIBOLD {
            self.semibold
        } else {
            self.regular
        }
    }
}

/// Multi-family font set held by the renderer. Each family ships in three
/// weights; selection takes both the requested family and weight so widgets
/// can opt into the display face via [`FontFamily::DeckSans`].
#[derive(Clone, Copy, Debug)]
pub struct Fonts {
    pub sans: WeightedFonts,
    pub deck_sans: WeightedFonts,
}

impl Fonts {
    /// Pick the FemtoVG font for a `(family, weight)` pair.
    #[must_use]
    pub fn select(self, family: FontFamily, weight: FontWeight) -> FontId {
        match family {
            FontFamily::Sans => self.sans.select(weight),
            FontFamily::DeckSans => self.deck_sans.select(weight),
        }
    }
}

// ── Paragraph layout cache ──────────────────────────────────────────

/// Cached paragraph layout (cosmic-text Buffer kept for layout_runs during render).
struct ParagraphLayoutEntry {
    buffer: Buffer,
    width: f32,
    height: f32,
    last_used_frame: u64,
}

/// Frame-based paragraph layout cache. Evicts entries not accessed in the last 2 frames.
pub struct ParagraphLayoutCache {
    entries: HashMap<u64, ParagraphLayoutEntry>,
    frame_counter: u64,
}

impl std::fmt::Debug for ParagraphLayoutCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParagraphLayoutCache")
            .field("entries", &self.entries.len())
            .field("frame_counter", &self.frame_counter)
            .finish()
    }
}

impl ParagraphLayoutCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            frame_counter: 0,
        }
    }

    /// Advance frame counter and evict stale entries.
    pub fn begin_frame(&mut self, frame_counter: u64) {
        self.frame_counter = frame_counter;
        self.entries
            .retain(|_, e| e.last_used_frame + 1 >= frame_counter);
    }

    /// Measure paragraph dimensions, shaping if not cached.
    pub fn measure(
        &mut self,
        font_system: &mut FontSystem,
        base_style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
    ) -> (f32, f32) {
        let key = cache_key(base_style, spans, max_width);
        let frame = self.frame_counter;

        // Evict oldest if at capacity
        if self.entries.len() >= 256
            && !self.entries.contains_key(&key)
            && let Some(&oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_used_frame)
                .map(|(k, _)| k)
        {
            self.entries.remove(&oldest);
        }

        let entry = self.entries.entry(key).or_insert_with(|| {
            // Styles still come from `spans` — normalizing only edits text, never
            // the span count or order, so a glyph's span index stays valid.
            let normalized = normalize_carriage_returns(spans);
            let (buffer, width, height) =
                shape_paragraph(font_system, base_style, &normalized, max_width);
            ParagraphLayoutEntry {
                buffer,
                width,
                height,
                last_used_frame: frame,
            }
        });
        entry.last_used_frame = frame;
        (entry.width, entry.height)
    }

    /// Draw a paragraph using cached layout. Calls FemtoVG fill_text per span segment.
    #[expect(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        font_system: &mut FontSystem,
        canvas: &mut Canvas<OpenGl>,
        fonts: Fonts,
        base_style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
    ) {
        // Nothing to draw, and every `spans[..]` below — including
        // `span_for_glyph`'s first-span fallback — would be out of bounds.
        if spans.is_empty() {
            return;
        }

        // Ensure layout is cached
        self.measure(font_system, base_style, spans, Some(max_width));
        let key = cache_key(base_style, spans, Some(max_width));
        let entry = &self.entries[&key];

        let flush_segment = |canvas: &mut Canvas<OpenGl>,
                             span_idx: usize,
                             text: &str,
                             start_x: f32,
                             baseline_y: f32| {
            let style = spans[span_idx].resolve_style(base_style);
            draw_text_segment(canvas, fonts, text, x + start_x, baseline_y, &style);
            let w = segment_width(canvas, fonts, text, &style);
            draw_decorations_for_segment(canvas, x + start_x, baseline_y, w, &style);
        };

        for run in entry.buffer.layout_runs() {
            // run.line_y is the alphabetic baseline (not line top — see module docs)
            let baseline_y = y + run.line_y;

            // `Some` exactly while a segment is in flight — set and cleared
            // together with `segment_text`.
            let mut current_span: Option<usize> = None;
            let mut segment_start_x = 0.0_f32;
            let mut segment_text = String::new();

            for glyph in run.glyphs {
                let span_idx = span_for_glyph(glyph, spans.len());

                if let Some(prev_span) = current_span
                    && prev_span != span_idx
                    && !segment_text.is_empty()
                {
                    flush_segment(
                        canvas,
                        prev_span,
                        &segment_text,
                        segment_start_x,
                        baseline_y,
                    );
                    segment_text.clear();
                }

                if segment_text.is_empty() {
                    segment_start_x = glyph.x;
                    current_span = Some(span_idx);
                }

                // `get` rejects an out-of-range *or* non-char-boundary range, so a
                // bad offset drops one glyph instead of panicking. Checking only
                // the length would miss a range landing inside a multi-byte
                // character, which panics and tears down the widget slot.
                if let Some(text) = run.text.get(glyph.start..glyph.end) {
                    segment_text.push_str(text);
                } else {
                    tracing::error!(
                        "text: glyph range {}..{} does not slice line {} (len {})",
                        glyph.start,
                        glyph.end,
                        run.line_i,
                        run.text.len(),
                    );
                }
            }

            // Flush last segment
            if let Some(span_idx) = current_span
                && !segment_text.is_empty()
            {
                flush_segment(canvas, span_idx, &segment_text, segment_start_x, baseline_y);
            }
        }
    }
}

// ── Internal helpers ────────────────────────────────────────────────

/// Compute cache key from style + spans + max_width.
fn cache_key(base_style: &TextStyle, spans: &[SpanData], max_width: Option<f32>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    base_style.size.hash(&mut hasher);
    base_style.weight.hash(&mut hasher);
    (base_style.family as u8).hash(&mut hasher);
    base_style.italic.hash(&mut hasher);
    base_style.line_height.to_bits().hash(&mut hasher);
    (base_style.align as u8).hash(&mut hasher);
    base_style.max_width.hash(&mut hasher);
    base_style.color.hash(&mut hasher);

    spans.len().hash(&mut hasher);
    for span in spans {
        span.text.hash(&mut hasher);
        span.weight.hash(&mut hasher);
        span.color.hash(&mut hasher);
        span.italic.hash(&mut hasher);
        span.underline.hash(&mut hasher);
        span.strikethrough.hash(&mut hasher);
    }

    max_width.map(f32::to_bits).hash(&mut hasher);
    hasher.finish()
}

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
const NO_SPAN: usize = usize::MAX;

/// Shape a paragraph using cosmic-text. Returns (buffer, width, height).
///
/// Each span is tagged with its own index through [`Attrs::metadata`], which
/// cosmic-text copies into every glyph it shapes from that span. That is what
/// lets [`ParagraphLayoutCache::draw`] recover a glyph's span directly, instead
/// of mapping the glyph's line-relative byte offset back into the concatenated
/// span text — a mapping that needs a per-line start table and cannot be derived
/// from the line endings, because `set_rich_text` stamps every line with
/// `LineEnding::default()` whatever separator actually split it.
fn shape_paragraph(
    font_system: &mut FontSystem,
    base_style: &TextStyle,
    spans: &[SpanData],
    max_width: Option<f32>,
) -> (Buffer, f32, f32) {
    let line_height = base_style.size as f32 * base_style.line_height;
    let metrics = Metrics::new(base_style.size as f32, line_height);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, max_width, None);

    let rich_spans: Vec<_> = spans
        .iter()
        .enumerate()
        .map(|(i, span)| {
            let resolved = span.resolve_style(base_style);
            (span.text.as_str(), build_attrs(&resolved).metadata(i))
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
    let line_count = buffer.layout_runs().count().max(1);
    let height = line_count as f32 * line_height;

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
fn normalize_carriage_returns(spans: &[SpanData]) -> std::borrow::Cow<'_, [SpanData]> {
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
            SpanData {
                text,
                ..span.clone()
            }
        })
        .collect();
    std::borrow::Cow::Owned(normalized)
}

/// Span owning `glyph`, as tagged by [`shape_paragraph`].
///
/// A glyph outside `0..span_count` — [`NO_SPAN`] included — would mean it was
/// shaped from attrs we never tagged. Report it and fall back to the first span:
/// a panic here tears down the widget slot, and dropping the glyph would lose
/// text rather than just mis-style it.
///
/// Kept as its own function so `span_attribution_tests` can exercise the draw
/// path's span lookup without a GL context.
fn span_for_glyph(glyph: &LayoutGlyph, span_count: usize) -> usize {
    if glyph.metadata < span_count {
        return glyph.metadata;
    }
    tracing::error!(
        "text: glyph metadata {} is not one of the {span_count} span indices",
        glyph.metadata,
    );
    0
}

/// Convert RGBA u32 to femtovg Color.
#[must_use]
pub fn to_femtovg_color(color: u32) -> femtovg::Color {
    femtovg::Color::rgba(
        ((color >> 24) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    )
}

/// Draw a text segment with FemtoVG at the given **baseline** y-coordinate.
///
/// Uses [`femtovg::Baseline::Alphabetic`] because the y-coordinate comes from
/// cosmic-text's `LayoutRun::line_y` which is the alphabetic baseline
/// (= `line_top + centering_offset + max_ascent`).
fn draw_text_segment(
    canvas: &mut Canvas<OpenGl>,
    fonts: Fonts,
    text: &str,
    x: f32,
    baseline_y: f32,
    style: &TextStyle,
) {
    let font = fonts.select(style.family, style.weight);
    let mut paint = Paint::color(to_femtovg_color(style.color.to_u32()));
    paint.set_font(&[font]);
    paint.set_font_size(style.size as f32);
    paint.set_text_baseline(femtovg::Baseline::Alphabetic);
    let _ = canvas.fill_text(x, baseline_y, text, &paint);
}

/// Measure text segment width using FemtoVG.
fn segment_width(canvas: &mut Canvas<OpenGl>, fonts: Fonts, text: &str, style: &TextStyle) -> f32 {
    let font = fonts.select(style.family, style.weight);
    let mut paint = Paint::color(to_femtovg_color(style.color.to_u32()));
    paint.set_font(&[font]);
    paint.set_font_size(style.size as f32);
    canvas
        .measure_text(0.0, 0.0, text, &paint)
        .map_or(0.0, |m| m.width())
}

// ── Autofit helpers ─────────────────────────────────────────────────

/// Readability floor used when the caller does not specify `min_size`.
pub(crate) const DEFAULT_MIN_AUTOFIT: u32 = 12;

/// Inclusive `[lower, upper]` font-size search range for an autofit command.
/// `target_height` bounds growth when `max_size` is unset: a line at size S is
/// `S * line_height` px tall, so a fitting size never exceeds
/// `target_height / line_height`. Pure.
///
/// `pub(crate)` so the GPU renderer (sibling `gpu::renderer` module) can call it.
pub(crate) fn autofit_bounds(
    size: u32,
    min_size: u32,
    max_size: u32,
    mode: AutoFit,
    target_height: Option<f32>,
    line_height: f32,
) -> (u32, u32) {
    let floor_default = if min_size > 0 {
        min_size
    } else {
        // Never let the implicit readability floor exceed the configured `size`:
        // for a `size` below the floor, a Shrink search would otherwise be
        // clamped *up* to the floor and grow text in a shrink-only mode.
        DEFAULT_MIN_AUTOFIT.min(size.max(1))
    };
    let lower = match mode {
        AutoFit::Grow => size,
        AutoFit::Shrink | AutoFit::ShrinkAndGrow => floor_default,
    };
    let upper = match mode {
        AutoFit::Shrink => size,
        AutoFit::Grow | AutoFit::ShrinkAndGrow => {
            if max_size > 0 {
                max_size
            } else {
                target_height.map_or(size, |h| {
                    // A line at size S is `S * line_height` px tall, so the
                    // tallest size that can fit is `h / line_height`. Guard a
                    // non-positive line_height (degenerate) by falling back to
                    // `size`, which never grows the text.
                    if line_height <= 0.0 {
                        return size;
                    }
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "box height is small, finite and non-negative"
                    )]
                    let cap = (h / line_height).floor() as u32;
                    cap
                })
            }
        }
    };
    let lower = lower.max(1);
    let upper = upper.max(lower);
    (lower, upper)
}

/// Binary-search the largest integer size in `[lower, upper]` whose measured
/// layout fits both targets. `measure_at(size)` returns `(width, height)`;
/// an absent target is always satisfied. Returns `lower` if even `lower`
/// overflows. Pure; ~log2(range) calls to `measure_at`.
pub(crate) fn search_fit_size(
    lower: u32,
    upper: u32,
    target_width: Option<f32>,
    target_height: Option<f32>,
    mut measure_at: impl FnMut(u32) -> (f32, f32),
) -> u32 {
    let fits = |w: f32, h: f32| {
        target_width.is_none_or(|tw| w <= tw) && target_height.is_none_or(|th| h <= th)
    };
    let mut lo = lower;
    let mut hi = upper;
    let mut best = lower;
    while lo <= hi {
        #[expect(
            clippy::integer_division,
            reason = "binary search midpoint; truncation is correct"
        )]
        let mid = lo + (hi - lo) / 2;
        let (w, h) = measure_at(mid);
        if fits(w, h) {
            best = mid;
            lo = mid + 1;
        } else if mid == lower {
            break;
        } else {
            hi = mid - 1;
        }
    }
    best
}

/// Draw underline / strikethrough for a text segment.
/// `baseline_y` is the text baseline (from cosmic-text `line_y`).
fn draw_decorations_for_segment(
    canvas: &mut Canvas<OpenGl>,
    x: f32,
    baseline_y: f32,
    w: f32,
    style: &TextStyle,
) {
    let fs = style.size as f32;

    if style.underline {
        let uy = baseline_y + fs * 0.1;
        let uh = (fs * 0.07).max(1.0);
        let mut path = femtovg::Path::new();
        path.rect(x, uy, w, uh);
        canvas.fill_path(&path, &Paint::color(to_femtovg_color(style.color.to_u32())));
    }

    if style.strikethrough {
        let sy = baseline_y - fs * 0.3;
        let sh = (fs * 0.07).max(1.0);
        let mut path = femtovg::Path::new();
        path.rect(x, sy, w, sh);
        canvas.fill_path(&path, &Paint::color(to_femtovg_color(style.color.to_u32())));
    }
}

#[cfg(test)]
mod autofit_tests {
    use super::{DEFAULT_MIN_AUTOFIT, autofit_bounds, search_fit_size};
    use crate::tree::AutoFit;

    // ── autofit_bounds ──────────────────────────────────────────────
    #[test]
    fn shrink_bounds_are_floor_to_size() {
        assert_eq!(
            autofit_bounds(40, 14, 0, AutoFit::Shrink, Some(100.0), 1.4),
            (14, 40)
        );
    }
    #[test]
    fn shrink_without_min_uses_default_floor() {
        assert_eq!(
            autofit_bounds(40, 0, 0, AutoFit::Shrink, Some(100.0), 1.4),
            (DEFAULT_MIN_AUTOFIT, 40)
        );
    }
    #[test]
    fn grow_bounds_are_size_to_max() {
        assert_eq!(
            autofit_bounds(24, 0, 64, AutoFit::Grow, Some(100.0), 1.4),
            (24, 64)
        );
    }
    #[test]
    fn grow_without_max_uses_box_height_ceiling() {
        // At line_height 1.0 the ceiling is the box height itself.
        assert_eq!(
            autofit_bounds(24, 0, 0, AutoFit::Grow, Some(80.7), 1.0),
            (24, 80)
        );
    }
    #[test]
    fn grow_ceiling_accounts_for_line_height() {
        // A line at size S is `S * line_height` px tall, so a sub-unit
        // line_height lets the fitting size exceed the raw box height:
        // 80 / 0.8 = 100. A >= 1.0 line_height caps below it: 80 / 1.4 = 57.
        assert_eq!(
            autofit_bounds(24, 0, 0, AutoFit::Grow, Some(80.0), 0.8),
            (24, 100)
        );
        assert_eq!(
            autofit_bounds(24, 0, 0, AutoFit::Grow, Some(80.0), 1.4),
            (24, 57)
        );
    }
    #[test]
    fn grow_ceiling_guards_non_positive_line_height() {
        // A degenerate line_height must not divide by zero or grow the text.
        assert_eq!(
            autofit_bounds(24, 0, 0, AutoFit::Grow, Some(80.0), 0.0),
            (24, 24)
        );
    }
    #[test]
    fn grow_without_max_and_without_height_cannot_grow() {
        assert_eq!(autofit_bounds(24, 0, 0, AutoFit::Grow, None, 1.4), (24, 24));
    }
    #[test]
    fn shrink_and_grow_spans_min_to_max() {
        assert_eq!(
            autofit_bounds(999, 14, 64, AutoFit::ShrinkAndGrow, Some(100.0), 1.4),
            (14, 64)
        );
    }
    #[test]
    fn bounds_clamp_when_min_exceeds_max() {
        let (lo, hi) = autofit_bounds(40, 80, 20, AutoFit::ShrinkAndGrow, Some(100.0), 1.4);
        assert!(lo <= hi);
        assert_eq!((lo, hi), (80, 80));
    }
    #[test]
    fn shrink_below_default_floor_does_not_grow() {
        // size 10 is below DEFAULT_MIN_AUTOFIT (12); a shrink-only search must
        // not be clamped up to the floor and enlarge the text.
        assert_eq!(
            autofit_bounds(10, 0, 0, AutoFit::Shrink, Some(100.0), 1.4),
            (10, 10)
        );
    }

    // ── search_fit_size ─────────────────────────────────────────────
    // Synthetic measure: width == height == size px. So size N fits a target of N.
    fn linear(size: u32) -> (f32, f32) {
        (size as f32, size as f32)
    }

    #[test]
    fn search_picks_largest_fitting_size() {
        assert_eq!(search_fit_size(10, 40, Some(30.0), Some(30.0), linear), 30);
    }
    #[test]
    fn search_returns_upper_when_everything_fits() {
        assert_eq!(
            search_fit_size(10, 40, Some(100.0), Some(100.0), linear),
            40
        );
    }
    #[test]
    fn search_returns_lower_when_nothing_fits() {
        assert_eq!(search_fit_size(10, 40, Some(5.0), Some(5.0), linear), 10);
    }
    #[test]
    fn search_absent_target_is_satisfied() {
        assert_eq!(search_fit_size(10, 40, Some(20.0), None, linear), 20);
    }
    #[test]
    fn search_single_point_range() {
        assert_eq!(
            search_fit_size(16, 16, Some(100.0), Some(100.0), linear),
            16
        );
    }
}
