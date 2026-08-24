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
//! cosmic-text is the only shaper: it produces every glyph and every position,
//! and FemtoVG draws them as cached atlas quads (or, above
//! [`DIRECT_PATH_CUTOFF_PX`], as pre-positioned glyph runs). FemtoVG's own
//! shaper is never invoked, so its kerning divergence from cosmic-text
//! can no longer reach the screen.
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
//! Glyph quads are placed against `line_y`, so vertical placement matches what
//! cosmic-text's own `buffer.draw()` / swash path would produce. Decorations
//! (underline, strikethrough) are positioned relative to this baseline. Callers
//! anchoring text by any other [`femtovg::Baseline`] convert through
//! [`baseline_to_alphabetic`] first.
//!
//! [`LayoutRun`]: cosmic_text::LayoutRun

#![expect(clippy::cast_precision_loss)]

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, LayoutGlyph, Metrics, Scroll, Shaping, Style,
    Weight,
};
use femtovg::{Canvas, FontId, Paint, renderer::OpenGl};
use rgb::FromSlice as _;

use crate::gpu::glyph_cache::{
    GlyphCache, GlyphLookup, GlyphQuad, PageBackend, PageCreateFailed, PageFaultKind, RasterGlyph,
};
use crate::renderer::TextLayoutCounters;
use crate::tree::{AutoFit, FontFamily, FontWeight, SpanData, TextAlign, TextStyle};

// Capture profiling reports 341 entries for the two largest production working sets.
// 448 matches HashMap's capacity at the former limit.
const LAYOUT_CACHE_CAPACITY: usize = 448;

// ── Paragraph layout cache ──────────────────────────────────────────

/// Compact render-ready paragraph layout.
pub(crate) struct ParagraphLayoutEntry {
    pub(crate) lines: Vec<LineGlyphs>,
    pub(crate) width: f32,
    pub(crate) height: f32,
    last_used_access: u64,
}

/// Capacity-bounded paragraph layout cache.
pub struct ParagraphLayoutCache {
    entries: HashMap<u64, ParagraphLayoutEntry>,
    access_counter: u64,
    /// Distinguishes a cache hit from a reshape that replaces the same entry,
    /// which entry count and width stability both survive.
    counters: LayoutCacheCounters,
    profile: Option<Box<LayoutCacheProfile>>,
}

#[derive(Debug, Default)]
struct LayoutCacheCounters {
    hits: u64,
    shapes: u64,
    capacity_evictions: u64,
    peak_entries: usize,
}

#[derive(Clone, Copy)]
enum LookupPhase {
    Measure,
    Draw,
}

#[derive(Clone, Copy, Hash)]
enum LayoutDomain {
    SingleLine,
    Paragraph,
}

#[derive(Default)]
struct LayoutCacheProfile {
    counters: TextLayoutCounters,
    keys_this_frame: HashSet<u64>,
    shaped_this_frame: HashSet<u64>,
    measured_this_frame: HashSet<u64>,
    weighted_keys_this_frame: HashSet<u64>,
    distinct_glyphs_this_frame: HashSet<cosmic_text::CacheKey>,
    glyph_instances_this_frame: usize,
    entries: HashMap<u64, ProfiledEntry>,
}

#[derive(Clone, Copy)]
struct ProfiledEntry {
    domain: LayoutDomain,
    glyphs: usize,
}

impl std::fmt::Debug for ParagraphLayoutCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParagraphLayoutCache")
            .field("entries", &self.entries.len())
            .field("access_counter", &self.access_counter)
            .field("counters", &self.counters)
            .field("profiling_enabled", &self.profile.is_some())
            .finish()
    }
}

impl ParagraphLayoutCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            access_counter: 0,
            counters: LayoutCacheCounters::default(),
            profile: None,
        }
    }

    pub fn enable_profiling(&mut self) {
        self.profile = Some(Box::default());
    }

    #[must_use]
    pub fn counters(&self) -> TextLayoutCounters {
        let mut counters = self
            .profile
            .as_ref()
            .map_or_else(TextLayoutCounters::default, |profile| profile.counters);
        counters.layout_cache_hits = self.counters.hits;
        counters.layout_cache_shapes = self.counters.shapes;
        counters.layout_cache_capacity_evictions = self.counters.capacity_evictions;
        counters.layout_cache_peak_entries = self.counters.peak_entries;
        counters
    }

    /// Must run before the lookup that inserts: `or_insert_with` cannot say
    /// afterwards whether it shaped.
    fn record_lookup(&mut self, key: u64, domain: LayoutDomain, phase: LookupPhase) {
        let cache_hit = self.entries.contains_key(&key);
        if cache_hit {
            self.counters.hits += 1;
        } else {
            self.counters.shapes += 1;
        }
        let entries_after_lookup = self.entries.len() + usize::from(!cache_hit);
        self.counters.peak_entries = self.counters.peak_entries.max(entries_after_lookup);

        if let Some(profile) = self.profile.as_mut() {
            match (domain, cache_hit) {
                (LayoutDomain::SingleLine, true) => {
                    profile.counters.layout_cache_single_line_hits += 1;
                }
                (LayoutDomain::SingleLine, false) => {
                    profile.counters.layout_cache_single_line_shapes += 1;
                }
                (LayoutDomain::Paragraph, true) => {
                    profile.counters.layout_cache_paragraph_hits += 1;
                }
                (LayoutDomain::Paragraph, false) => {
                    profile.counters.layout_cache_paragraph_shapes += 1;
                }
            }
            profile.keys_this_frame.insert(key);
            profile.counters.layout_cache_peak_frame_keys = profile
                .counters
                .layout_cache_peak_frame_keys
                .max(profile.keys_this_frame.len());

            if matches!(phase, LookupPhase::Measure) {
                profile.measured_this_frame.insert(key);
            } else if !cache_hit && profile.measured_this_frame.contains(&key) {
                profile.counters.layout_cache_draw_misses_after_measure += 1;
            }

            if !cache_hit && !profile.shaped_this_frame.insert(key) {
                profile.counters.layout_cache_repeat_shapes_same_frame += 1;
            }
        }
    }

    /// Clear frame-local profiling state.
    pub fn begin_frame(&mut self) {
        if let Some(profile) = self.profile.as_mut() {
            profile.keys_this_frame.clear();
            profile.shaped_this_frame.clear();
            profile.measured_this_frame.clear();
            profile.weighted_keys_this_frame.clear();
            profile.distinct_glyphs_this_frame.clear();
            profile.glyph_instances_this_frame = 0;
        }
    }

    fn next_access(&mut self) -> u64 {
        self.access_counter = self
            .access_counter
            .checked_add(1)
            .expect("BUG: layout cache access sequence exhausted");
        self.access_counter
    }

    /// Evict the least recently used entry to make room for `key`.
    fn evict_for(&mut self, key: u64) {
        if self.entries.len() >= LAYOUT_CACHE_CAPACITY
            && !self.entries.contains_key(&key)
            && let Some(&oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used_access)
                .map(|(k, _)| k)
        {
            self.entries.remove(&oldest);
            self.counters.capacity_evictions += 1;
            if let Some(profile) = self.profile.as_mut()
                && let Some(entry) = profile.entries.remove(&oldest)
            {
                profile.counters.layout_cache_resident_glyphs = profile
                    .counters
                    .layout_cache_resident_glyphs
                    .checked_sub(entry.glyphs)
                    .expect("BUG: profiled layout glyph count underflowed");
                let domain_glyphs = match entry.domain {
                    LayoutDomain::SingleLine => {
                        profile.counters.layout_cache_single_line_entries = profile
                            .counters
                            .layout_cache_single_line_entries
                            .checked_sub(1)
                            .expect("BUG: profiled single-line entry count underflowed");
                        &mut profile.counters.layout_cache_single_line_resident_glyphs
                    }
                    LayoutDomain::Paragraph => {
                        profile.counters.layout_cache_paragraph_entries = profile
                            .counters
                            .layout_cache_paragraph_entries
                            .checked_sub(1)
                            .expect("BUG: profiled paragraph entry count underflowed");
                        &mut profile.counters.layout_cache_paragraph_resident_glyphs
                    }
                };
                *domain_glyphs = domain_glyphs
                    .checked_sub(entry.glyphs)
                    .expect("BUG: profiled domain glyph count underflowed");
            }
        }
    }

    fn record_profiled_entry(&mut self, key: u64, domain: LayoutDomain) {
        let Self {
            entries, profile, ..
        } = self;
        let Some(profile) = profile.as_mut() else {
            return;
        };
        let counters = &mut profile.counters;
        let entry = entries
            .get(&key)
            .expect("BUG: a profiled layout lookup must leave a cache entry");
        let glyphs = entry.lines.iter().map(|line| line.glyphs.len()).sum();

        if profile
            .entries
            .insert(key, ProfiledEntry { domain, glyphs })
            .is_none()
        {
            counters.layout_cache_resident_glyphs += glyphs;
            counters.layout_cache_peak_resident_glyphs = counters
                .layout_cache_peak_resident_glyphs
                .max(counters.layout_cache_resident_glyphs);
            match domain {
                LayoutDomain::SingleLine => {
                    counters.layout_cache_single_line_entries += 1;
                    counters.layout_cache_single_line_peak_entries = counters
                        .layout_cache_single_line_peak_entries
                        .max(counters.layout_cache_single_line_entries);
                    counters.layout_cache_single_line_resident_glyphs += glyphs;
                    counters.layout_cache_single_line_peak_resident_glyphs = counters
                        .layout_cache_single_line_peak_resident_glyphs
                        .max(counters.layout_cache_single_line_resident_glyphs);
                }
                LayoutDomain::Paragraph => {
                    counters.layout_cache_paragraph_entries += 1;
                    counters.layout_cache_paragraph_peak_entries = counters
                        .layout_cache_paragraph_peak_entries
                        .max(counters.layout_cache_paragraph_entries);
                    counters.layout_cache_paragraph_resident_glyphs += glyphs;
                    counters.layout_cache_paragraph_peak_resident_glyphs = counters
                        .layout_cache_paragraph_peak_resident_glyphs
                        .max(counters.layout_cache_paragraph_resident_glyphs);
                }
            }
        }

        if profile.weighted_keys_this_frame.insert(key) {
            profile.glyph_instances_this_frame += glyphs;
            counters.layout_cache_peak_frame_glyph_instances = counters
                .layout_cache_peak_frame_glyph_instances
                .max(profile.glyph_instances_this_frame);
            for glyph in entry.lines.iter().flat_map(|line| &line.glyphs) {
                profile.distinct_glyphs_this_frame.insert(glyph.key);
            }
            counters.layout_cache_peak_frame_distinct_glyphs = counters
                .layout_cache_peak_frame_distinct_glyphs
                .max(profile.distinct_glyphs_this_frame.len());
        }
    }

    /// Measure paragraph dimensions, shaping if not cached.
    pub fn measure(
        &mut self,
        font_system: &mut FontSystem,
        base_style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
    ) -> (f32, f32) {
        let entry = self.layout(font_system, base_style, spans, max_width);
        (entry.width, entry.height)
    }

    /// Lay a paragraph out, shaping if not cached.
    ///
    /// `pub(crate)`, not `pub`: [`ParagraphLayoutEntry`] is crate-private,
    /// and a public method returning it
    /// would be a private interface in a public module.
    pub(crate) fn layout(
        &mut self,
        font_system: &mut FontSystem,
        base_style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
    ) -> &ParagraphLayoutEntry {
        self.layout_for(
            font_system,
            base_style,
            spans,
            max_width,
            LookupPhase::Measure,
        )
    }

    fn layout_for(
        &mut self,
        font_system: &mut FontSystem,
        base_style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
        phase: LookupPhase,
    ) -> &ParagraphLayoutEntry {
        let key = cache_key(base_style, spans, max_width);
        let access = self.next_access();
        self.evict_for(key);
        self.record_lookup(key, LayoutDomain::Paragraph, phase);

        let make_entry = || {
            // Styles still come from `spans` — normalizing only edits text, never
            // the span count or order, so a glyph's span index stays valid.
            let normalized = normalize_carriage_returns(spans);
            let (buffer, width, height) =
                shape_paragraph(font_system, base_style, &normalized, max_width);
            let lines = extract_lines(&buffer);
            ParagraphLayoutEntry {
                lines,
                width,
                height,
                last_used_access: access,
            }
        };
        if self.profile.is_some() {
            {
                let entry = self.entries.entry(key).or_insert_with(make_entry);
                entry.last_used_access = access;
            }
            self.record_profiled_entry(key, LayoutDomain::Paragraph);
            return self
                .entries
                .get(&key)
                .expect("BUG: profiling must not remove the looked-up layout");
        }
        let entry = self.entries.entry(key).or_insert_with(make_entry);
        entry.last_used_access = access;
        entry
    }

    /// Lay a single line of uniformly styled text out, shaping if not cached.
    ///
    /// Kept apart from [`Self::layout`] because [`TextStyle::size`] is `u32`,
    /// which truncates every fractional and animated size to a whole pixel.
    pub(crate) fn layout_single_line(
        &mut self,
        font_system: &mut FontSystem,
        style: LineStyle,
        text: &str,
    ) -> &ParagraphLayoutEntry {
        self.layout_single_line_for(font_system, style, text, LookupPhase::Draw)
    }

    pub(crate) fn measure_single_line(
        &mut self,
        font_system: &mut FontSystem,
        style: LineStyle,
        text: &str,
    ) -> &ParagraphLayoutEntry {
        self.layout_single_line_for(font_system, style, text, LookupPhase::Measure)
    }

    fn layout_single_line_for(
        &mut self,
        font_system: &mut FontSystem,
        style: LineStyle,
        text: &str,
        phase: LookupPhase,
    ) -> &ParagraphLayoutEntry {
        let key = single_line_cache_key(style, text);
        let access = self.next_access();
        self.evict_for(key);
        self.record_lookup(key, LayoutDomain::SingleLine, phase);

        let make_entry = || {
            let (buffer, width, height) = shape_single_line(font_system, style, text);
            let lines = extract_lines(&buffer);
            ParagraphLayoutEntry {
                lines,
                width,
                height,
                last_used_access: access,
            }
        };
        if self.profile.is_some() {
            {
                let entry = self.entries.entry(key).or_insert_with(make_entry);
                entry.last_used_access = access;
            }
            self.record_profiled_entry(key, LayoutDomain::SingleLine);
            return self
                .entries
                .get(&key)
                .expect("BUG: profiling must not remove the looked-up layout");
        }
        let entry = self.entries.entry(key).or_insert_with(make_entry);
        entry.last_used_access = access;
        entry
    }

    /// Draw a paragraph using cached layout, one cached-glyph batch per span.
    #[expect(clippy::too_many_arguments, reason = "one paragraph's full draw state")]
    pub(crate) fn draw(
        &mut self,
        font_system: &mut FontSystem,
        canvas: &mut Canvas<OpenGl>,
        cache: &mut GlyphCache<femtovg::ImageId>,
        swash: &mut cosmic_text::SwashCache,
        font_table: &FontTable,
        base_style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
    ) {
        // Nothing to draw, and every `spans[..]` below — including
        // `span_groups`' first-span fallback — would be out of bounds.
        if spans.is_empty() {
            return;
        }

        let entry = self.layout_for(
            font_system,
            base_style,
            spans,
            Some(max_width),
            LookupPhase::Draw,
        );

        for line in &entry.lines {
            // baseline_y is the alphabetic baseline (not line top — see module docs)
            let baseline_y = y + line.baseline_y;
            for group in span_groups(&line.glyphs, spans.len()) {
                let style = spans[group.span].resolve_style(base_style);
                let paint = Paint::color(to_femtovg_color(style.color.to_u32()));
                draw_line_glyphs(
                    canvas,
                    cache,
                    swash,
                    font_system,
                    font_table,
                    group.glyphs,
                    x,
                    baseline_y,
                    &paint,
                    group.font_size,
                );
                draw_decorations_for_segment(
                    canvas,
                    x + group.start_x,
                    baseline_y,
                    group.width,
                    &style,
                );
            }
        }
    }
}

// ── Internal helpers ────────────────────────────────────────────────

fn layout_cache_hasher(domain: LayoutDomain) -> std::collections::hash_map::DefaultHasher {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    domain.hash(&mut hasher);
    hasher
}

/// Compute cache key from style + spans + max_width.
fn cache_key(base_style: &TextStyle, spans: &[SpanData], max_width: Option<f32>) -> u64 {
    let mut hasher = layout_cache_hasher(LayoutDomain::Paragraph);

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

// ── Single-line layout ──────────────────────────────────────────────

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

/// Compute the cache key of a single line.
///
/// `to_bits` is what makes the size hashable. Its quirks around `-0.0` and `NaN`
/// never bite, because a size reaching here is always positive.
fn single_line_cache_key(style: LineStyle, text: &str) -> u64 {
    let mut hasher = layout_cache_hasher(LayoutDomain::SingleLine);
    (style.family as u8).hash(&mut hasher);
    style.weight.hash(&mut hasher);
    style.italic.hash(&mut hasher);
    style.size.to_bits().hash(&mut hasher);
    text.hash(&mut hasher);
    hasher.finish()
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

    // Every embedded face is upright, so `Style::Italic` is what makes cosmic-text
    // flag the glyphs `FAKE_ITALIC` and the rasterizer skew them.
    if style.italic {
        attrs.style(Style::Italic)
    } else {
        attrs
    }
}

/// Shape one uniformly styled line, unwrapped. Returns (buffer, width, height).
fn shape_single_line(
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

// ── Per-line glyph extraction ───────────────────────────────────────

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

/// Span owning a glyph tagged `metadata`, as tagged by [`shape_paragraph`].
///
/// A glyph outside `0..span_count` — [`NO_SPAN`] included — would mean it was
/// shaped from attrs we never tagged. Report it and fall back to the first span:
/// a panic here tears down the widget slot, and dropping the glyph would lose
/// text rather than just mis-style it.
///
/// Kept as its own function so `span_attribution_tests` can exercise the draw
/// path's span lookup without a GL context.
fn span_for_glyph(metadata: usize, span_count: usize) -> usize {
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
fn span_groups(glyphs: &[PositionedGlyphInfo], span_count: usize) -> Vec<SpanGroup<'_>> {
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

// ── Glyph rasterization ─────────────────────────────────────────────

/// Rasterize one glyph into 8-bit alpha coverage.
///
/// Deliberately `get_image_uncached`:
/// cosmic-text's `get_image` memoizes into an unbounded map,
/// the very growth this cache exists to replace.
/// `None` covers everything with no coverage to upload — a missing font,
/// a colour or subpixel bitmap, an empty box such as a space.
pub(crate) fn rasterize_glyph(
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut FontSystem,
    key: cosmic_text::CacheKey,
) -> Option<RasterGlyph> {
    let image = swash.get_image_uncached(font_system, key)?;
    if image.content != cosmic_text::SwashContent::Mask {
        return None;
    }

    // The single u32 → usize boundary: swash sizes glyphs in u32, the cache
    // and everything under it in usize.
    let width = usize::try_from(image.placement.width).expect("BUG: swash dimension exceeds usize");
    let height =
        usize::try_from(image.placement.height).expect("BUG: swash dimension exceeds usize");
    if width * height == 0 {
        return None;
    }
    assert_eq!(
        image.data.len(),
        width * height,
        "BUG: swash mask is not a tightly packed bitmap of its placement"
    );

    Some(RasterGlyph {
        width,
        height,
        left: image.placement.left,
        top: image.placement.top,
        coverage: image.data,
    })
}

// ── Cached-glyph draw path ──────────────────────────────────────────

/// femtovg's own direct-path check (`lib.rs:1455`).
/// Above it femtovg path-renders per frame instead of atlasing,
/// so the cache owns exactly the range femtovg would have atlased.
pub(crate) const DIRECT_PATH_CUTOFF_PX: f32 = 92.0;

/// The skew cosmic-text synthesizes italic with (`swash.rs:68`);
/// no shipped face has a true italic.
const FAKE_ITALIC_SKEW_DEGREES: f32 = 14.0;

/// The embedded faces in load order, as `(post_script_name, weight)`.
/// [`build_font_table`] zips this order with femtovg's, so a divergence here
/// would map every glyph to the wrong face.
const EMBEDDED_FACES: [(&str, u16); 7] = [
    ("BraiinsSans", 400),
    ("BraiinsSans-SemiBold", 600),
    ("BraiinsSans-Bold", 700),
    ("BraiinsDeckSans-Regular", 400),
    ("BraiinsDeckSans-SemiBold", 600),
    ("BraiinsDeckSans-Bold", 700),
    ("NotoSans-Regular", 400),
];

/// cosmic-text's per-glyph face choice,
/// translated into femtovg's handle for the same font binary.
pub(crate) struct FontTable {
    pairs: Vec<(cosmic_text::fontdb::ID, FontId)>,
}

impl FontTable {
    pub(crate) fn femtovg_font(&self, id: cosmic_text::fontdb::ID) -> FontId {
        self.pairs
            .iter()
            .find_map(|&(face, font)| (face == id).then_some(font))
            .expect("BUG: a glyph came from a face outside the embedded font set")
    }
}

/// Pair each cosmic-text face, positionally,
/// with the femtovg font registered from the same bytes.
///
/// femtovg's [`FontId`] is opaque, so no pair can be *queried* for correctness —
/// it holds by construction, both libraries being fed the same seven byte arrays
/// in the same order. The check against [`EMBEDDED_FACES`] is unconditional:
/// `faces()` iterates fontdb's private slot map, and a release build whose order
/// had silently diverged would map glyphs to the wrong fonts forever after.
/// Seven faces once at init cost nothing.
pub(crate) fn build_font_table(
    font_system: &FontSystem,
    femtovg_ids: &[FontId; EMBEDDED_FACES.len()],
) -> FontTable {
    let faces: Vec<_> = font_system.db().faces().collect();
    assert_eq!(
        faces.len(),
        EMBEDDED_FACES.len(),
        "BUG: font table order diverged"
    );
    for (face, expected) in faces.iter().zip(EMBEDDED_FACES) {
        assert_eq!(
            (face.post_script_name.as_str(), face.weight.0),
            expected,
            "BUG: font table order diverged"
        );
    }

    FontTable {
        pairs: faces
            .iter()
            .map(|face| face.id)
            .zip(femtovg_ids.iter().copied())
            .collect(),
    }
}

/// Snap a glyph origin the way femtovg does (`text.rs:492-499`):
/// x truncated, y rounded.
/// Applied once in layout space, before any canvas transform,
/// so cached quads and delegated runs land on the same coordinates.
pub(crate) fn snap(x: f32, y: f32) -> (f32, f32) {
    (x.trunc(), y.round())
}

/// cosmic-text's synthetic italic, in canvas space,
/// about a glyph's baseline origin.
/// cosmic skews `x' = x + y·tan(14°)` in font space (y-up);
/// the canvas is y-down, so the conjugation flips the sign
/// to `x' = x − (y − oy)·tan(14°)`.
fn italic_about(ox: f32, oy: f32) -> femtovg::Transform2D {
    let mut transform = femtovg::Transform2D::translation(-ox, -oy);
    transform.skew_x(-FAKE_ITALIC_SKEW_DEGREES.to_radians());
    transform.translate(ox, oy);
    transform
}

/// One `fill_glyph_run` submission: contiguous glyphs sharing a font,
/// or a lone `FAKE_ITALIC` glyph the caller draws under its own skew.
#[derive(Debug)]
pub(crate) struct OversizedRun {
    pub font_id: FontId,
    pub italic: bool,
    pub glyphs: Vec<femtovg::PositionedGlyph>,
}

/// What one line's glyphs submit to, in draw order.
pub(crate) enum GlyphCommand<P> {
    Quads { page: P, quads: Vec<femtovg::Quad> },
    Direct(OversizedRun),
}

/// Split an oversized line into the runs femtovg can take:
/// one per font, and one per `FAKE_ITALIC` glyph, which femtovg's direct path
/// would otherwise fill from the unmodified outline and render upright.
pub(crate) fn chunk_oversized(
    glyphs: &[PositionedGlyphInfo],
    font_table: &FontTable,
    origin_x: f32,
    alphabetic_y: f32,
) -> Vec<OversizedRun> {
    let mut runs: Vec<OversizedRun> = Vec::new();
    for glyph in glyphs {
        let (x, y) = snap(origin_x + glyph.x, alphabetic_y + glyph.y);
        let font_id = font_table.femtovg_font(glyph.font_id);
        let italic = glyph
            .flags
            .contains(cosmic_text::CacheKeyFlags::FAKE_ITALIC);
        let positioned = femtovg::PositionedGlyph {
            x,
            y,
            glyph_id: glyph.glyph_id,
        };

        match runs.last_mut() {
            Some(run) if !italic && !run.italic && run.font_id == font_id => {
                run.glyphs.push(positioned);
            }
            Some(_) | None => runs.push(OversizedRun {
                font_id,
                italic,
                glyphs: vec![positioned],
            }),
        }
    }
    runs
}

/// Extend the batch in flight while its page holds; a page change closes it.
/// Grouping every quad of a page together instead would reorder the line,
/// letting a later glyph draw under an earlier one.
fn push_quad<P: Copy + Eq>(commands: &mut Vec<GlyphCommand<P>>, page: P, quad: femtovg::Quad) {
    match commands.last_mut() {
        Some(GlyphCommand::Quads { page: open, quads }) if *open == page => quads.push(quad),
        Some(GlyphCommand::Quads { .. } | GlyphCommand::Direct(_)) | None => {
            commands.push(GlyphCommand::Quads {
                page,
                quads: vec![quad],
            });
        }
    }
}

/// swash reports `top` as the coverage's height above the baseline,
/// so the canvas-space top edge is its negation.
fn glyph_quad<P>(cached: &GlyphQuad<P>, gx: f32, gy: f32) -> femtovg::Quad {
    let x0 = gx + cached.placement.left as f32;
    let y0 = gy - cached.placement.top as f32;
    femtovg::Quad {
        x0,
        y0,
        x1: x0 + cached.placement.width as f32,
        y1: y0 + cached.placement.height as f32,
        s0: cached.u0,
        t0: cached.v0,
        s1: cached.u1,
        t1: cached.v1,
    }
}

fn cached_glyph_quad<P>(
    backend: &mut impl PageBackend<PageId = P>,
    cache: &mut GlyphCache<P>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut FontSystem,
    glyph: &PositionedGlyphInfo,
    origin_x: f32,
    alphabetic_y: f32,
) -> Option<(P, femtovg::Quad)>
where
    P: Copy + Eq + core::fmt::Debug,
{
    let (gx, gy) = snap(origin_x + glyph.x, alphabetic_y + glyph.y);
    let lookup = cache.get_or_insert(backend, glyph.key, |key| {
        rasterize_glyph(swash, font_system, key.inner())
    });
    match lookup {
        GlyphLookup::Resident(cached) => Some((cached.page, glyph_quad(&cached, gx, gy))),
        GlyphLookup::Missing | GlyphLookup::Oversized | GlyphLookup::Dropped => None,
    }
}

/// Build what one line submits, without submitting it.
///
/// Split out from [`draw_line_glyphs`] because femtovg's submission calls
/// return nothing: every placement assertion — snapping, baselines, the cutoff —
/// is made against this output, which `draw_line_glyphs` submits exactly.
#[expect(clippy::too_many_arguments, reason = "one line's full draw state")]
pub(crate) fn build_glyph_commands<P>(
    backend: &mut impl PageBackend<PageId = P>,
    cache: &mut GlyphCache<P>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut FontSystem,
    font_table: &FontTable,
    glyphs: &[PositionedGlyphInfo],
    origin_x: f32,
    alphabetic_y: f32,
    font_size: f32,
) -> Vec<GlyphCommand<P>>
where
    P: Copy + Eq + core::fmt::Debug,
{
    if font_size > DIRECT_PATH_CUTOFF_PX {
        return chunk_oversized(glyphs, font_table, origin_x, alphabetic_y)
            .into_iter()
            .map(GlyphCommand::Direct)
            .collect();
    }

    let mut commands = Vec::new();
    for glyph in glyphs {
        if let Some((page, quad)) = cached_glyph_quad(
            backend,
            cache,
            swash,
            font_system,
            glyph,
            origin_x,
            alphabetic_y,
        ) {
            push_quad(&mut commands, page, quad);
        }
    }
    commands
}

/// Prepare one cached command per curved glyph before its individual transform.
/// Missing glyphs retain their slot so commands stay aligned with arc placements.
pub(crate) fn build_cached_curved_glyph_commands<P>(
    backend: &mut impl PageBackend<PageId = P>,
    cache: &mut GlyphCache<P>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut FontSystem,
    glyphs: &[PositionedGlyphInfo],
    alphabetic_y: f32,
) -> Vec<Option<GlyphCommand<P>>>
where
    P: Copy + Eq + core::fmt::Debug,
{
    glyphs
        .iter()
        .map(|glyph| {
            cached_glyph_quad(
                backend,
                cache,
                swash,
                font_system,
                glyph,
                curved_glyph_origin_x(glyph),
                alphabetic_y,
            )
            .map(|(page, quad)| GlyphCommand::Quads {
                page,
                quads: vec![quad],
            })
        })
        .collect()
}

/// Expand cached batches for a solid-colour outline.
/// All copies share one paint, so grouping offsets within each page batch
/// preserves the source-over result without allocating one vector per offset.
pub(crate) fn outline_glyph_commands<P: Copy>(
    commands: &[GlyphCommand<P>],
    rings: u32,
) -> Vec<GlyphCommand<P>> {
    assert!(rings > 0, "BUG: an outline must contain at least one ring");
    let repeats = usize::try_from(rings)
        .expect("BUG: outline ring count exceeds usize")
        .checked_mul(8)
        .expect("BUG: outline repeat count overflows usize");
    let mut outlined = Vec::with_capacity(commands.len());

    for command in commands {
        let GlyphCommand::Quads { page, quads } = command else {
            panic!("BUG: direct-path commands cannot be expanded as cached outline quads");
        };
        let capacity = quads
            .len()
            .checked_mul(repeats)
            .expect("BUG: outline quad count overflows usize");
        let mut repeated = Vec::with_capacity(capacity);
        for ring in 1..=rings {
            let d = ring as f32;
            for (dx, dy) in [
                (d, 0.0),
                (-d, 0.0),
                (0.0, d),
                (0.0, -d),
                (d, d),
                (-d, -d),
                (d, -d),
                (-d, d),
            ] {
                repeated.extend(quads.iter().map(|quad| femtovg::Quad {
                    x0: quad.x0 + dx,
                    y0: quad.y0 + dy,
                    x1: quad.x1 + dx,
                    y1: quad.y1 + dy,
                    ..*quad
                }));
            }
        }
        outlined.push(GlyphCommand::Quads {
            page: *page,
            quads: repeated,
        });
    }

    outlined
}

/// The paint femtovg's direct path needs.
/// The incoming paint carries colour only, and `Paint` defaults to 16 px
/// (`paint.rs:281-288`) — femtovg chooses path rendering over its own atlas
/// by the paint's font size alone (`lib.rs:1455`), so the colour-only paint
/// would populate that atlas instead.
fn direct_paint(paint: &Paint, font_size: f32) -> Paint {
    let mut direct = paint.clone();
    direct.set_font_size(font_size);
    direct
}

/// Draw one line's glyphs, cached below the cutoff and delegated above it.
///
/// Takes a slice rather than a [`LineGlyphs`] so callers can submit a subgroup
/// without building a temporary vector on every warm frame.
#[expect(clippy::too_many_arguments, reason = "one line's full draw state")]
pub(crate) fn draw_line_glyphs(
    canvas: &mut Canvas<OpenGl>,
    cache: &mut GlyphCache<femtovg::ImageId>,
    swash: &mut cosmic_text::SwashCache,
    font_system: &mut FontSystem,
    font_table: &FontTable,
    glyphs: &[PositionedGlyphInfo],
    origin_x: f32,
    alphabetic_y: f32,
    paint: &Paint,
    font_size: f32,
) {
    let commands = build_glyph_commands(
        &mut FemtovgPages { canvas },
        cache,
        swash,
        font_system,
        font_table,
        glyphs,
        origin_x,
        alphabetic_y,
        font_size,
    );

    submit_glyph_commands(canvas, cache, commands, paint, font_size);
}

/// Submit prepared commands in one cached batch or as direct-path runs.
pub(crate) fn submit_glyph_commands<I>(
    canvas: &mut Canvas<OpenGl>,
    #[cfg_attr(not(feature = "atlas-inspect"), expect(unused_variables))] cache: &mut GlyphCache<
        femtovg::ImageId,
    >,
    commands: I,
    paint: &Paint,
    font_size: f32,
) where
    I: IntoIterator<Item = GlyphCommand<femtovg::ImageId>>,
    I::IntoIter: ExactSizeIterator,
{
    let commands = commands.into_iter();
    if font_size > DIRECT_PATH_CUTOFF_PX {
        let mut oversized_paint = None;
        for command in commands {
            match command {
                GlyphCommand::Quads { .. } => {
                    panic!("BUG: direct-path text produced a cached-quad command");
                }
                GlyphCommand::Direct(run) => {
                    let paint =
                        oversized_paint.get_or_insert_with(|| direct_paint(paint, font_size));
                    if run.italic {
                        debug_assert_eq!(
                            run.glyphs.len(),
                            1,
                            "BUG: an italic run's skew pivots on its one glyph"
                        );
                        let pivot = run
                            .glyphs
                            .first()
                            .expect("BUG: an oversized run carries no glyph");
                        let italic = italic_about(pivot.x, pivot.y);
                        canvas.save();
                        canvas.set_transform(&italic);
                        direct_path::fill_oversized_run(canvas, run.font_id, run.glyphs, paint);
                        canvas.restore();
                    } else {
                        direct_path::fill_oversized_run(canvas, run.font_id, run.glyphs, paint);
                    }
                }
            }
        }
        return;
    }

    let mut alpha_glyphs = Vec::with_capacity(commands.len());
    for command in commands {
        let GlyphCommand::Quads { page, quads } = command else {
            panic!("BUG: atlas-range text produced a direct-path command");
        };
        #[cfg(feature = "atlas-inspect")]
        cache.record_drawn_page(page);
        alpha_glyphs.push(femtovg::DrawCommand {
            image_id: page,
            quads,
        });
    }
    if !alpha_glyphs.is_empty() {
        canvas.draw_glyph_commands(
            femtovg::GlyphDrawCommands {
                alpha_glyphs,
                color_glyphs: Vec::new(),
            },
            paint,
        );
    }
}

/// Origin that centers one glyph's advance on the arc point it is drawn at.
///
/// The `− x` cancels the glyph's position within its line, which
/// [`draw_line_glyphs`] adds back; without it every glyph after the first
/// is displaced by that position on top of its arc placement.
pub(crate) fn curved_glyph_origin_x(glyph: &PositionedGlyphInfo) -> f32 {
    -glyph.w / 2.0 - glyph.x
}

mod direct_path {
    use femtovg::{Canvas, FontId, Paint, PositionedGlyph, renderer::OpenGl};

    /// The one femtovg direct-path entry point. Everything
    /// > 92 px nominal routes here; femtovg path-renders it per frame.
    pub(super) fn fill_oversized_run(
        canvas: &mut Canvas<OpenGl>,
        font_id: FontId,
        glyphs: impl IntoIterator<Item = PositionedGlyph>,
        paint: &Paint,
    ) {
        debug_assert!(
            paint.font_size() > 92.0,
            "BUG: direct path for atlas-range size"
        );
        #[expect(
            clippy::disallowed_methods,
            reason = "sole >92px direct-path delegation"
        )]
        let _ = canvas.fill_glyph_run(font_id, glyphs, paint);
    }
}

/// The atlas pages, backed by femtovg images.
/// Dimensions stay `usize` end to end: femtovg's image calls take `usize`,
/// and so does [`PageBackend`].
pub(crate) struct FemtovgPages<'a> {
    pub(crate) canvas: &'a mut Canvas<OpenGl>,
}

impl PageBackend for FemtovgPages<'_> {
    type PageId = femtovg::ImageId;

    fn create_page(&mut self, size_px: usize) -> Result<Self::PageId, PageCreateFailed> {
        self.canvas
            .create_image_empty(
                size_px,
                size_px,
                femtovg::PixelFormat::Gray8,
                femtovg::ImageFlags::NEAREST,
            )
            .map_err(|_| PageCreateFailed)
    }

    fn upload(
        &mut self,
        page: Self::PageId,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        pixels: &[u8],
    ) -> Result<(), PageFaultKind> {
        // `as_gray` is a slice cast over a transparent wrapper,
        // so the padded buffer reaches the driver without a copy.
        let coverage = imgref::ImgRef::new(pixels.as_gray(), width, height);
        self.canvas
            .update_image(page, femtovg::ImageSource::Gray(coverage), x, y)
            .map_err(|error| upload_fault(&error))
    }
}

/// A rect that leaves the page, a format mismatch or an unknown image
/// are all bugs in what we asked for, and quarantining the page is what stops
/// the cache repeating them; anything else is the driver having a bad frame.
fn upload_fault(error: &femtovg::ErrorKind) -> PageFaultKind {
    if matches!(
        error,
        femtovg::ErrorKind::ImageUpdateOutOfBounds
            | femtovg::ErrorKind::ImageUpdateWithDifferentFormat
            | femtovg::ErrorKind::ImageIdNotFound
    ) {
        PageFaultKind::Invariant
    } else {
        PageFaultKind::Transient
    }
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

/// A pair the two shapers demonstrably kern apart at `size`,
/// from a fixed candidate list.
/// The module header records that cosmic-text and femtovg kern differently
/// without naming a pair, so the pair is discovered rather than pinned —
/// a font swap that made every candidate agree has to fail loudly
/// instead of leaving the tests built on it comparing nothing.
#[cfg(test)]
pub(crate) fn divergent_kerning_pair(font_system: &mut FontSystem, size: f32) -> &'static str {
    const CANDIDATES: [&str; 7] = ["AV", "To", "Ta", "LT", "AW", "Yo", "P,"];
    /// Below this the two agree to within their own rounding.
    const MIN_DIVERGENCE_PX: f32 = 0.5;

    let fonts = femtovg::TextContext::default();
    let mut paint = Paint::color(femtovg::Color::white());
    paint.set_font(&[fonts
        .add_font_mem(include_bytes!(
            "../../../assets/fonts/BraiinsSans-Regular.otf"
        ))
        .expect("BUG: font registration failed")]);
    paint.set_font_size(size);

    let mut layout = ParagraphLayoutCache::new();
    CANDIDATES
        .into_iter()
        .find(|pair| {
            let cosmic = layout
                .layout_single_line(
                    font_system,
                    crate::gpu::renderer::sans_line_style(size),
                    pair,
                )
                .width;
            #[expect(
                clippy::disallowed_methods,
                reason = "test compares shapers on purpose"
            )]
            let femtovg = fonts
                .measure_text(0.0, 0.0, pair, &paint)
                .expect("BUG: femtovg cannot measure the fixture")
                .width();
            (cosmic - femtovg).abs() > MIN_DIVERGENCE_PX
        })
        .expect("BUG: no candidate separates the shapers; the list needs a new pair")
}

/// Every glyph must be attributed to the span its characters actually came from,
/// across a hard break of any width and under soft wrap, and every glyph range
/// must slice the line it indexes. These run without a GL context, unlike the
/// end-to-end pixel regressions in `gpu::renderer`.
#[cfg(test)]
mod span_attribution_tests {
    use std::collections::{BTreeSet, HashMap};

    use cosmic_text::Buffer;

    use super::{NO_SPAN, normalize_carriage_returns, shape_paragraph, span_for_glyph};
    use crate::tree::{SpanData, TextStyle};

    fn span(text: &str) -> SpanData {
        SpanData {
            text: text.to_owned(),
            weight: None,
            color: None,
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
        let (buffer, _, _) =
            shape_paragraph(&mut font_system, &style, &normalized, Some(max_width));
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
        assert_glyph_ranges_slice_their_line(
            &[span("\u{1f600}\r"), span("\nGreen \u{1f600}")],
            400.0,
        );
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
}

/// What one line hands to the draw path, span by span.
/// CPU-only — nothing here needs a GL context.
#[cfg(test)]
mod span_group_tests {
    use super::{
        LineGlyphs, SpanGroup, extract_lines, normalize_carriage_returns, shape_paragraph,
        span_groups,
    };
    use crate::tree::{SpanData, TextStyle};

    /// An RTL script the embedded faces do not cover,
    /// so its glyphs come back as boxes —
    /// the bidi reordering under test is the shaper's, not the font's,
    /// and holds either way.
    const HEBREW: &str = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}";

    fn span(text: &str, underline: bool) -> SpanData {
        SpanData {
            text: text.to_owned(),
            weight: None,
            color: None,
            italic: false,
            underline,
            strikethrough: false,
        }
    }

    fn only_line(spans: &[SpanData]) -> LineGlyphs {
        let normalized = normalize_carriage_returns(spans);
        let mut font_system = crate::gpu::renderer::build_font_system();
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
}

/// Per-visual-line glyph extraction, the baseline contract,
/// and the single-line entry point.
/// CPU-only — nothing here needs a GL context.
#[cfg(test)]
mod line_layout_tests {
    use std::hash::Hasher as _;

    use cosmic_text::{Attrs, Buffer, CacheKeyFlags, Family, FontSystem, Metrics, Shaping};

    use super::{
        LAYOUT_CACHE_CAPACITY, LayoutDomain, LineStyle, ParagraphLayoutCache,
        baseline_to_alphabetic, extract_lines, layout_cache_hasher, single_line_cache_key,
    };
    use crate::tree::{FontFamily, FontWeight, SpanData, TextStyle};

    fn font_system() -> FontSystem {
        crate::gpu::renderer::build_font_system()
    }

    fn line_style(size: f32, italic: bool) -> LineStyle {
        LineStyle {
            family: FontFamily::Sans,
            weight: FontWeight::REGULAR,
            italic,
            size,
        }
    }

    #[test]
    fn layout_domains_use_distinct_hash_namespaces() {
        let paragraph = layout_cache_hasher(LayoutDomain::Paragraph).finish();
        let single_line = layout_cache_hasher(LayoutDomain::SingleLine).finish();

        assert_ne!(
            paragraph, single_line,
            "shared cache domains must not start from the same hash state"
        );
    }

    /// Two spans of very different size, forced onto separate lines by `max_width`.
    ///
    /// Built as a raw buffer because `SpanData` cannot express a per-span size:
    /// `resolve_style` overrides weight, colour, italic and decorations only.
    fn mixed_size_buffer(font_system: &mut FontSystem) -> Buffer {
        let mut buffer = Buffer::new(font_system, Metrics::new(40.0, 48.0));
        buffer.set_size(font_system, Some(130.0), None);
        let family = Family::Name("Braiins Sans");
        let small = Attrs::new()
            .family(family)
            .metrics(Metrics::new(14.0, 20.0))
            .metadata(0);
        let large = Attrs::new()
            .family(family)
            .metrics(Metrics::new(40.0, 48.0))
            .metadata(1);
        buffer.set_rich_text(
            font_system,
            [("tiny ", small), ("HUGE", large)],
            &Attrs::new().family(family),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        buffer
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
        let buffer = mixed_size_buffer(&mut font_system);
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

    /// Repeating a single-line draw must reuse the extracted glyph allocation;
    /// rebuilding it on a cache hit would restore the per-draw allocation cost.
    #[test]
    fn single_line_layout_is_cached() {
        let mut font_system = font_system();
        let mut cache = ParagraphLayoutCache::new();
        let style = line_style(24.0, false);

        assert_eq!(cache.counters().layout_cache_shapes, 0);
        let glyphs = &cache
            .layout_single_line(&mut font_system, style, "12:34")
            .lines[0]
            .glyphs;
        let glyph_allocation = glyphs.as_ptr();
        assert_eq!(
            cache.counters().layout_cache_shapes,
            1,
            "the first call must shape"
        );
        let cached_glyphs = &cache
            .layout_single_line(&mut font_system, style, "12:34")
            .lines[0]
            .glyphs;
        assert_eq!(
            cached_glyphs.as_ptr(),
            glyph_allocation,
            "a cache hit must retain the extracted glyph allocation"
        );
        assert_eq!(
            cache.counters().layout_cache_shapes,
            1,
            "the second call must not shape"
        );
        assert_eq!(
            cache.counters().layout_cache_hits,
            1,
            "the second call must count as a hit"
        );
    }

    #[test]
    fn layout_survives_an_intervening_widget_frame() {
        let mut font_system = font_system();
        let mut cache = ParagraphLayoutCache::new();
        let style = line_style(24.0, false);

        cache.begin_frame();
        cache.layout_single_line(&mut font_system, style, "widget A");
        cache.begin_frame();
        cache.layout_single_line(&mut font_system, style, "widget B");
        cache.begin_frame();
        cache.layout_single_line(&mut font_system, style, "widget A");

        let counters = cache.counters();
        assert_eq!(counters.layout_cache_shapes, 2);
        assert_eq!(counters.layout_cache_hits, 1);
        assert_eq!(cache.entries.len(), 2);
    }

    #[test]
    fn same_frame_hit_makes_layout_most_recently_used() {
        let mut font_system = font_system();
        let mut cache = ParagraphLayoutCache::new();
        let style = line_style(24.0, false);

        cache.begin_frame();
        for index in 0..LAYOUT_CACHE_CAPACITY {
            cache.layout_single_line(&mut font_system, style, &index.to_string());
        }
        cache.layout_single_line(&mut font_system, style, "0");
        cache.layout_single_line(&mut font_system, style, &LAYOUT_CACHE_CAPACITY.to_string());

        let counters = cache.counters();
        assert_eq!(cache.entries.len(), LAYOUT_CACHE_CAPACITY);
        assert_eq!(counters.layout_cache_capacity_evictions, 1);
        assert_eq!(counters.layout_cache_peak_entries, LAYOUT_CACHE_CAPACITY);
        assert_eq!(counters.layout_cache_hits, 1);
        assert!(
            cache
                .entries
                .contains_key(&single_line_cache_key(style, "0")),
            "a hit must refresh the oldest entry"
        );
        assert!(
            !cache
                .entries
                .contains_key(&single_line_cache_key(style, "1")),
            "the next-oldest entry must be evicted"
        );
    }

    #[test]
    fn profile_reports_a_measured_layout_lost_before_draw() {
        let mut font_system = font_system();
        let mut cache = ParagraphLayoutCache::new();
        let style = line_style(24.0, false);
        cache.enable_profiling();
        cache.begin_frame();

        cache.measure_single_line(&mut font_system, style, "measured");
        cache.measure_single_line(&mut font_system, style, "measured");
        cache.layout_single_line(&mut font_system, style, "measured");
        cache.measure_single_line(&mut font_system, style, "other");

        cache
            .entries
            .remove(&single_line_cache_key(style, "measured"));
        cache.layout_single_line(&mut font_system, style, "measured");

        let counters = cache.counters();
        assert_eq!(counters.layout_cache_peak_frame_keys, 2);
        assert_eq!(counters.layout_cache_repeat_shapes_same_frame, 1);
        assert_eq!(counters.layout_cache_draw_misses_after_measure, 1);
        assert_eq!(counters.layout_cache_single_line_hits, 2);
        assert_eq!(counters.layout_cache_single_line_shapes, 3);
        assert_eq!(counters.layout_cache_paragraph_hits, 0);
        assert_eq!(counters.layout_cache_paragraph_shapes, 0);
    }

    #[test]
    fn profile_does_not_carry_frame_local_state_into_the_next_frame() {
        let mut font_system = font_system();
        let mut cache = ParagraphLayoutCache::new();
        let style = line_style(24.0, false);
        let key = single_line_cache_key(style, "measured");
        cache.enable_profiling();

        cache.begin_frame();
        cache.measure_single_line(&mut font_system, style, "measured");
        cache.entries.remove(&key);
        cache.layout_single_line(&mut font_system, style, "measured");

        cache.begin_frame();
        cache.entries.remove(&key);
        cache.layout_single_line(&mut font_system, style, "measured");

        let counters = cache.counters();
        assert_eq!(counters.layout_cache_peak_frame_keys, 1);
        assert_eq!(counters.layout_cache_repeat_shapes_same_frame, 1);
        assert_eq!(counters.layout_cache_draw_misses_after_measure, 1);
    }

    #[test]
    fn profile_separates_paragraph_lookups_from_single_line_lookups() {
        let mut font_system = font_system();
        let mut cache = ParagraphLayoutCache::new();
        let style = TextStyle::default();
        let spans = [SpanData {
            text: "paragraph".to_owned(),
            weight: None,
            color: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }];
        cache.enable_profiling();
        cache.begin_frame();

        cache.layout(&mut font_system, &style, &spans, Some(200.0));
        cache.layout(&mut font_system, &style, &spans, Some(200.0));

        let counters = cache.counters();
        assert_eq!(counters.layout_cache_single_line_hits, 0);
        assert_eq!(counters.layout_cache_single_line_shapes, 0);
        assert_eq!(counters.layout_cache_paragraph_hits, 1);
        assert_eq!(counters.layout_cache_paragraph_shapes, 1);
        assert_eq!(counters.layout_cache_single_line_entries, 0);
        assert_eq!(counters.layout_cache_paragraph_entries, 1);
        assert_eq!(counters.layout_cache_paragraph_peak_entries, 1);
        assert!(
            counters.layout_cache_paragraph_resident_glyphs > 0,
            "the profiled paragraph must retain at least one glyph"
        );
        assert_eq!(
            counters.layout_cache_resident_glyphs,
            counters.layout_cache_paragraph_resident_glyphs
        );
        assert_eq!(
            counters.layout_cache_peak_frame_glyph_instances,
            counters.layout_cache_paragraph_resident_glyphs
        );
        assert!(
            counters.layout_cache_peak_frame_distinct_glyphs
                <= counters.layout_cache_peak_frame_glyph_instances,
            "distinct glyphs cannot outnumber retained glyph instances"
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

    /// Every embedded face is upright, so an italic single-line style renders
    /// as italic only if cosmic-text flags its glyphs `FAKE_ITALIC`
    /// and the rasterizer skews them.
    #[test]
    fn italic_line_style_emits_fake_italic_keys() {
        let mut font_system = font_system();
        let mut cache = ParagraphLayoutCache::new();

        for italic in [false, true] {
            let entry =
                cache.layout_single_line(&mut font_system, line_style(24.0, italic), "Slanted");
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

#[cfg(test)]
mod rasterize_tests {
    use cosmic_text::fontdb;

    use super::{FontSystem, rasterize_glyph};
    use crate::gpu::glyph_cache::PAGE_SIZE_PX;

    const CORPUS_SIZE_PX: f32 = 92.0;

    fn key_for(
        face: (fontdb::ID, fontdb::Weight),
        glyph_id: u16,
        flags: cosmic_text::CacheKeyFlags,
    ) -> cosmic_text::CacheKey {
        cosmic_text::CacheKey {
            font_id: face.0,
            glyph_id,
            font_size_bits: CORPUS_SIZE_PX.to_bits(),
            x_bin: cosmic_text::SubpixelBin::Zero,
            y_bin: cosmic_text::SubpixelBin::Zero,
            font_weight: face.1,
            flags,
        }
    }

    /// `FaceInfo` carries no glyph count, and `db()` borrows the font system
    /// immutably while `get_font` needs it mutably — so faces are snapshotted
    /// in two owned passes before anything is rasterized.
    fn corpus(font_system: &mut FontSystem) -> Vec<(fontdb::ID, fontdb::Weight, u16)> {
        let faces: Vec<(fontdb::ID, fontdb::Weight)> = font_system
            .db()
            .faces()
            .map(|face| (face.id, face.weight))
            .collect();
        faces
            .into_iter()
            .map(|(id, weight)| {
                let font = font_system
                    .get_font(id, weight)
                    .expect("BUG: font database face has no loaded font");
                let count = font.as_swash().glyph_metrics(&[]).glyph_count();
                (id, weight, count)
            })
            .collect()
    }

    #[test]
    fn corpus_rasterizes_within_page_bounds() {
        let mut font_system = crate::gpu::renderer::build_font_system();
        let mut swash = cosmic_text::SwashCache::new();

        let mut rasterized = 0_usize;
        let mut odd_width = 0_usize;
        let mut oversized = 0_usize;
        for (id, weight, glyph_count) in corpus(&mut font_system) {
            for glyph_id in 0..glyph_count {
                let key = key_for((id, weight), glyph_id, cosmic_text::CacheKeyFlags::empty());
                let Some(glyph) = rasterize_glyph(&mut swash, &mut font_system, key) else {
                    continue;
                };
                assert_eq!(
                    glyph.coverage.len(),
                    glyph.width * glyph.height,
                    "coverage must be a tightly packed mask of its placement"
                );
                assert!(
                    glyph.width > 0 && glyph.height > 0,
                    "empty coverage must be reported as a miss, not a zero-sized glyph"
                );
                rasterized += 1;
                odd_width += usize::from(glyph.width % 2 == 1);
                oversized +=
                    usize::from(glyph.width + 2 > PAGE_SIZE_PX || glyph.height + 2 > PAGE_SIZE_PX);
            }
        }

        assert!(rasterized > 0, "the shipped fonts must rasterize something");
        assert!(
            odd_width > 0,
            "the corpus must exercise odd widths, which stress row padding"
        );
        assert_eq!(
            oversized, 0,
            "no shipped glyph at {CORPUS_SIZE_PX} px may exceed a page once padded"
        );
    }

    #[test]
    fn fake_italic_key_produces_skewed_coverage() {
        let mut font_system = crate::gpu::renderer::build_font_system();
        let mut swash = cosmic_text::SwashCache::new();

        let (id, weight, _) = corpus(&mut font_system)
            .into_iter()
            .next()
            .expect("BUG: font database is empty");
        let font = font_system
            .get_font(id, weight)
            .expect("BUG: font database face has no loaded font");
        let glyph_id = font.as_swash().charmap().map('W');

        let upright = rasterize_glyph(
            &mut swash,
            &mut font_system,
            key_for((id, weight), glyph_id, cosmic_text::CacheKeyFlags::empty()),
        )
        .expect("BUG: 'W' does not rasterize");
        let italic = rasterize_glyph(
            &mut swash,
            &mut font_system,
            key_for(
                (id, weight),
                glyph_id,
                cosmic_text::CacheKeyFlags::FAKE_ITALIC,
            ),
        )
        .expect("BUG: skewed 'W' does not rasterize");

        assert_ne!(
            (upright.width, upright.coverage),
            (italic.width, italic.coverage),
            "FAKE_ITALIC is part of the key because it changes the raster"
        );
    }
}

/// Placement of cached quads and delegated runs: the snapping rule,
/// the contiguous batching fold, and the synthetic-italic transform.
#[cfg(test)]
mod glyph_draw_tests {
    use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, SwashCache};

    use super::{
        DIRECT_PATH_CUTOFF_PX, FAKE_ITALIC_SKEW_DEGREES, FontTable, GlyphCommand,
        baseline_to_alphabetic, build_cached_curved_glyph_commands, build_glyph_commands,
        chunk_oversized, direct_paint, extract_lines, italic_about, outline_glyph_commands,
        push_quad, rasterize_glyph, snap,
    };
    use crate::gpu::glyph_cache::{GlyphCache, GlyphKey, test_support::MockBackend};

    const SANS: &[u8] = include_bytes!("../../../assets/fonts/BraiinsSans-Regular.otf");
    const DECK_SANS: &[u8] = include_bytes!("../../../assets/fonts/BraiinsDeckSans-Regular.otf");

    /// Fractional on both axes and in both directions,
    /// so a rule that rounded x or truncated y would show.
    const ORIGIN_X: f32 = 10.7;
    const BASELINE_Y: f32 = 20.4;

    fn font_system() -> FontSystem {
        crate::gpu::renderer::build_font_system()
    }

    /// Never consulted below the cutoff, which is where the quad path lives.
    fn no_fonts() -> FontTable {
        FontTable { pairs: Vec::new() }
    }

    /// femtovg parity: `x.trunc()` but `y.round()` (`text.rs:492-499`).
    /// A symmetric rule would drift text by up to a pixel
    /// against the placement the direct path still takes from femtovg above 92 px.
    #[test]
    fn snap_is_trunc_x_round_y() {
        for ((x, y), expected) in [
            ((1.7_f32, 1.4_f32), (1.0_f32, 1.0_f32)),
            ((-1.7, -1.4), (-1.0, -1.0)),
            ((2.3, 2.5), (2.0, 3.0)),
        ] {
            assert_eq!(snap(x, y), expected, "snapping ({x}, {y})");
        }
    }

    /// Merging every quad of a page into one batch would reorder the line,
    /// so a page seen again after another page opens a batch of its own.
    #[test]
    fn batches_split_on_page_change_only_contiguously() {
        let mut commands = Vec::new();
        for page in [0_usize, 0, 1, 0] {
            push_quad(&mut commands, page, femtovg::Quad::default());
        }

        let shape: Vec<(usize, usize)> = commands
            .iter()
            .map(|command| match command {
                GlyphCommand::Quads { page, quads } => (*page, quads.len()),
                GlyphCommand::Direct(_) => panic!("BUG: the quad fold emitted a direct run"),
            })
            .collect();
        assert_eq!(shape, vec![(0, 2), (1, 1), (0, 1)]);
    }

    #[test]
    fn outline_expands_each_batch_without_repeating_page_commands() {
        let quad = femtovg::Quad {
            x0: 10.0,
            y0: 20.0,
            x1: 14.0,
            y1: 26.0,
            s0: 0.1,
            t0: 0.2,
            s1: 0.3,
            t1: 0.4,
        };
        let commands = vec![GlyphCommand::Quads {
            page: 7,
            quads: vec![quad],
        }];

        let outlined = outline_glyph_commands(&commands, 2);

        let [GlyphCommand::Quads { page, quads }] = outlined.as_slice() else {
            panic!("BUG: one cached batch must remain one outlined batch");
        };
        assert_eq!(*page, 7);
        assert_eq!(quads.len(), 16, "two rings must emit eight offsets each");
        assert_eq!((quads[0].x0, quads[0].y0), (11.0, 20.0));
        assert_eq!((quads[8].x0, quads[8].y0), (12.0, 20.0));
        assert_eq!(
            (quads[15].x0, quads[15].y0),
            (8.0, 22.0),
            "the final copy must be the second ring's bottom-left offset"
        );
        assert_eq!(
            (quads[0].s0, quads[0].t0, quads[0].s1, quads[0].t1),
            (0.1, 0.2, 0.3, 0.4)
        );
    }

    /// The skew's sign flips through the y-axis conjugation:
    /// cosmic skews in font space (y-up), the canvas is y-down.
    /// A wrong sign slants the text backwards, a wrong pivot shifts it sideways.
    ///
    /// The oracle is swash's own skewed coverage.
    /// 'W' is drawn entirely from straight segments, so its outline points
    /// *are* its extremes — a curved glyph's control points would inflate
    /// the bounds past the ink.
    #[test]
    fn italic_transform_matches_swash_skew() {
        let (ox, oy) = (12.0_f32, 34.0_f32);
        let italic = italic_about(ox, oy);
        let tan = FAKE_ITALIC_SKEW_DEGREES.to_radians().tan();
        for (x, y) in [(0.0_f32, 0.0_f32), (20.0, 10.0), (-5.0, 60.0)] {
            let (tx, ty) = italic.transform_point(x, y);
            assert!(
                (tx - (x - (y - oy) * tan)).abs() < 1e-3,
                "({x}, {y}) skewed to x {tx}"
            );
            assert!((ty - y).abs() < 1e-3, "({x}, {y}) moved to y {ty}");
        }

        let mut font_system = font_system();
        let mut swash = SwashCache::new();
        let upright_key = first_glyph_key(&mut font_system, "W", 92.0, false);
        let outline = swash
            .get_outline_commands_uncached(&mut font_system, upright_key)
            .expect("BUG: 'W' has no outline");
        let skewed_key = first_glyph_key(&mut font_system, "W", 92.0, true);
        let skewed = rasterize_glyph(&mut swash, &mut font_system, skewed_key)
            .expect("BUG: skewed 'W' does not rasterize");

        let about_origin = italic_about(0.0, 0.0);
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for point in outline_points(&outline) {
            // Font space is y-up about the baseline; the canvas is y-down.
            let (x, y) = about_origin.transform_point(point.0, -point.1);
            bounds = Some(bounds.map_or((x, x, y, y), |(x0, x1, y0, y1)| {
                (x0.min(x), x1.max(x), y0.min(y), y1.max(y))
            }));
        }
        let (min_x, max_x, min_y, max_y) = bounds.expect("BUG: 'W' outlines no points");

        let width = i32::try_from(skewed.width).expect("BUG: width outside i32");
        let height = i32::try_from(skewed.height).expect("BUG: height outside i32");
        for (transformed, rasterized, edge) in [
            (min_x, skewed.left, "left"),
            (max_x, skewed.left + width, "right"),
            (min_y, -skewed.top, "top"),
            (max_y, height - skewed.top, "bottom"),
        ] {
            assert!(
                (transformed - rasterized as f32).abs() <= 1.0,
                "{edge}: transformed to {transformed}, swash rasterized {rasterized}",
            );
        }
    }

    /// `set_transform` premultiplies, so the italic matrix alone composes
    /// inside whatever transform curved text already applied.
    /// Precomposing it with the current transform would apply that one twice.
    #[test]
    fn italic_delegation_composes_with_outer_transform() {
        // The transform stack lives on `Canvas`, above the renderer,
        // so the void renderer observes the same composition without GL.
        let mut canvas = femtovg::Canvas::new(femtovg::renderer::Void)
            .expect("BUG: void canvas creation failed");
        canvas.translate(30.0, 40.0);
        canvas.rotate(0.3);
        let outer = canvas.transform();

        let (gx, gy) = (17.0_f32, 61.0_f32);
        let italic = italic_about(gx, gy);
        canvas.save();
        canvas.set_transform(&italic);
        let mid = canvas.transform();
        assert_eq!(
            mid,
            italic * outer,
            "the italic must apply before the outer"
        );

        let (px, py) = (25.0_f32, 40.0_f32);
        let (ix, iy) = italic.transform_point(px, py);
        let expected = outer.transform_point(ix, iy);
        let actual = mid.transform_point(px, py);
        assert!(
            (actual.0 - expected.0).abs() < 1e-3 && (actual.1 - expected.1).abs() < 1e-3,
            "point {actual:?} against italic-then-outer {expected:?}",
        );

        canvas.restore();
        assert_eq!(canvas.transform(), outer, "restore must undo the italic");
    }

    /// femtovg fills the unmodified outline, so a `FAKE_ITALIC` glyph batched
    /// with its neighbours would render upright above the cutoff;
    /// each one has to be submitted alone under its own skew.
    /// Fonts break a run for a blunter reason:
    /// `fill_glyph_run` takes one font for the whole run.
    #[test]
    fn oversized_runs_chunk_per_font_and_break_at_fake_italic() {
        // Fonts live in femtovg's text context, not on the canvas,
        // so real `FontId`s cost no GL context here.
        let fonts = femtovg::TextContext::default();
        let sans = fonts
            .add_font_mem(SANS)
            .expect("BUG: font registration failed");
        let deck = fonts
            .add_font_mem(DECK_SANS)
            .expect("BUG: font registration failed");

        let mut font_system = font_system();
        let font_table = FontTable {
            pairs: vec![
                (face_id(&font_system, "BraiinsSans"), sans),
                (face_id(&font_system, "BraiinsDeckSans-Regular"), deck),
            ],
        };
        let size = DIRECT_PATH_CUTOFF_PX + 1.0;
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(size, size));
        buffer.set_rich_text(
            &mut font_system,
            [
                ("AB", named(SANS_FAMILY, false)),
                ("CD", named(DECK_FAMILY, false)),
                ("E", named(SANS_FAMILY, true)),
                ("F", named(SANS_FAMILY, false)),
            ],
            &named(SANS_FAMILY, false),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);
        let lines = extract_lines(&buffer);
        let [line] = lines.as_slice() else {
            panic!("BUG: the fixture must shape to one line");
        };

        let runs = chunk_oversized(&line.glyphs, &font_table, ORIGIN_X, BASELINE_Y);
        let shape: Vec<(femtovg::FontId, bool, usize)> = runs
            .iter()
            .map(|run| (run.font_id, run.italic, run.glyphs.len()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (sans, false, 2),
                (deck, false, 2),
                (sans, true, 1),
                (sans, false, 1),
            ],
        );

        let submitted: Vec<(f32, f32)> = runs
            .iter()
            .flat_map(|run| run.glyphs.iter())
            .map(|glyph| (glyph.x, glyph.y))
            .collect();
        let expected: Vec<(f32, f32)> = line
            .glyphs
            .iter()
            .map(|glyph| snap(ORIGIN_X + glyph.x, BASELINE_Y + glyph.y))
            .collect();
        assert_eq!(submitted, expected, "delegated glyphs must be pre-snapped");

        let mut swash = SwashCache::new();
        let mut cache = GlyphCache::new();
        let commands = build_glyph_commands(
            &mut MockBackend::default(),
            &mut cache,
            &mut swash,
            &mut font_system,
            &font_table,
            &line.glyphs,
            ORIGIN_X,
            BASELINE_Y,
            size,
        );
        let delegated: Vec<(femtovg::FontId, bool, usize)> = commands
            .iter()
            .map(|command| match command {
                GlyphCommand::Direct(run) => (run.font_id, run.italic, run.glyphs.len()),
                GlyphCommand::Quads { .. } => {
                    panic!("BUG: above the cutoff no glyph may reach the cache")
                }
            })
            .collect();
        assert_eq!(
            delegated, shape,
            "the draw path must submit the chunker's runs"
        );
    }

    /// `Paint` defaults to 16 px and femtovg picks the direct path
    /// from the paint's size alone,
    /// so a colour-only paint would quietly atlas a 93 px glyph.
    #[test]
    fn direct_paint_carries_the_nominal_size() {
        let paint = femtovg::Paint::color(femtovg::Color::white());
        let size = direct_paint(&paint, 93.0).font_size();
        assert!(
            (size - 93.0).abs() < f32::EPSILON,
            "the direct paint carries {size}, not the nominal 93",
        );
    }

    /// Curved text places one glyph per arc point, and the origin it submits
    /// has to cancel the glyph's position within its line —
    /// [`build_cached_curved_glyph_commands`] adds that position back.
    /// Without the cancellation every glyph after the first
    /// drifts by its own line offset, which a single-glyph fixture cannot see.
    /// Asserted on the pen origin the quad reconstructs to,
    /// not the quad's centre:
    /// side bearings put the ink off centre for perfectly placed glyphs.
    ///
    #[test]
    fn curved_glyphs_center_their_advance_on_the_arc() {
        let mut font_system = font_system();
        let mut swash = SwashCache::new();
        let size = 24.0;
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(size, size));
        // Kerned pairs: the second glyph's line position is not the first
        // glyph's advance, so a leaked line position cannot cancel out.
        buffer.set_text(
            &mut font_system,
            "AVATAR",
            &named(SANS_FAMILY, false),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);
        let lines = extract_lines(&buffer);
        let [line] = lines.as_slice() else {
            panic!("BUG: the fixture must shape to one line");
        };
        assert!(
            line.glyphs.windows(2).any(|pair| {
                let [left, right] = pair else {
                    unreachable!("BUG: windows(2) yields pairs")
                };
                (right.x - left.x - left.w).abs() > f32::EPSILON
            }),
            "the fixture must kern, or a leaked line position cancels out",
        );

        let mut backend = MockBackend::default();
        let mut cache = GlyphCache::new();
        let commands = build_cached_curved_glyph_commands(
            &mut backend,
            &mut cache,
            &mut swash,
            &mut font_system,
            &line.glyphs,
            BASELINE_Y,
        );
        assert_eq!(
            commands.len(),
            line.glyphs.len(),
            "each arc placement must retain its command slot"
        );
        for (glyph, command) in line.glyphs.iter().zip(commands) {
            let Some(GlyphCommand::Quads { quads, .. }) = command else {
                panic!("BUG: one glyph must submit one batch");
            };
            let [quad] = quads.as_slice() else {
                panic!("BUG: one glyph must submit one quad");
            };
            // Normalized, as the cache rasterized it: the raw key's subpixel
            // bins shift the placement by up to a pixel.
            let raster = rasterize_glyph(
                &mut swash,
                &mut font_system,
                GlyphKey::normalize(glyph.key).inner(),
            )
            .expect("BUG: a batched glyph has no coverage");
            let (expected_pen, _) = snap(-glyph.w / 2.0, BASELINE_Y);
            assert!(
                (quad.x0 - raster.left as f32 - expected_pen).abs() < f32::EPSILON,
                "glyph of advance {} drew from pen {}, not {expected_pen}",
                glyph.w,
                quad.x0 - raster.left as f32,
            );
        }
    }

    /// The cached quad has to land on the snapped origin,
    /// with swash's baseline-relative `top` negated into the canvas's y-down
    /// space. Getting that sign wrong hangs every glyph below its baseline.
    #[test]
    fn cached_quads_sit_at_the_snapped_origin() {
        let mut font_system = font_system();
        let mut swash = SwashCache::new();
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(24.0, 24.0));
        buffer.set_text(
            &mut font_system,
            "Hi",
            &named(SANS_FAMILY, false),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);
        let lines = extract_lines(&buffer);
        let [line] = lines.as_slice() else {
            panic!("BUG: the fixture must shape to one line");
        };

        let mut backend = MockBackend::default();
        let mut cache = GlyphCache::new();
        let commands = build_glyph_commands(
            &mut backend,
            &mut cache,
            &mut swash,
            &mut font_system,
            &no_fonts(),
            &line.glyphs,
            ORIGIN_X,
            BASELINE_Y,
            24.0,
        );

        let [GlyphCommand::Quads { quads, .. }] = commands.as_slice() else {
            panic!("BUG: one page must yield exactly one batch");
        };
        assert_eq!(quads.len(), line.glyphs.len());

        for (glyph, quad) in line.glyphs.iter().zip(quads) {
            let raster = rasterize_glyph(&mut swash, &mut font_system, glyph.key)
                .expect("BUG: a batched glyph has no coverage");
            let (gx, gy) = snap(ORIGIN_X + glyph.x, BASELINE_Y + glyph.y);
            assert_eq!(
                (quad.x0, quad.y0, quad.x1, quad.y1),
                (
                    gx + raster.left as f32,
                    gy - raster.top as f32,
                    gx + raster.left as f32 + raster.width as f32,
                    gy - raster.top as f32 + raster.height as f32,
                ),
            );
        }
    }

    // ── Boundary goldens ────────────────────────────────────────────

    /// The seven faces `build_font_system` loads, in its order.
    /// [`build_font_table`] pairs the two libraries positionally,
    /// so a fallback glyph resolves only when the whole set is registered.
    ///
    /// [`build_font_table`]: super::build_font_table
    const EMBEDDED_FONTS: [&[u8]; 7] = [
        SANS,
        include_bytes!("../../../assets/fonts/BraiinsSans-SemiBold.otf"),
        include_bytes!("../../../assets/fonts/BraiinsSans-Bold.otf"),
        DECK_SANS,
        include_bytes!("../../../assets/fonts/BraiinsDeckSans-SemiBold.otf"),
        include_bytes!("../../../assets/fonts/BraiinsDeckSans-Bold.otf"),
        include_bytes!("../../../assets/fonts/NotoSans-Regular.ttf"),
    ];

    /// Greek variant letters no Braiins face carries —
    /// verified against the embedded cmaps, every one of them shapes to Noto.
    /// Plain Greek would not do: the Braiins faces cover it,
    /// and a string that never leaves the primary face
    /// proves nothing about the fallback.
    const GREEK_FALLBACK: &str = "ϖϑϰϱϵ";

    /// The cosmic-to-femtovg font pairing `FemtoVgRenderer::new` builds,
    /// without its canvas: fonts live in the text context, so real `FontId`s
    /// cost no GL context here. The context is returned because the ids index
    /// into it.
    fn embedded_font_table(font_system: &FontSystem) -> (femtovg::TextContext, FontTable) {
        let fonts = femtovg::TextContext::default();
        let ids = EMBEDDED_FONTS.map(|data| {
            fonts
                .add_font_mem(data)
                .expect("BUG: font registration failed")
        });
        let table = super::build_font_table(font_system, &ids);
        (fonts, table)
    }

    /// The one visual line `text` shapes to through the embedded faces.
    fn shape_line(
        font_system: &mut FontSystem,
        text: &str,
        size: f32,
        italic: bool,
    ) -> super::LineGlyphs {
        let mut buffer = Buffer::new(font_system, Metrics::new(size, size));
        buffer.set_text(
            font_system,
            text,
            &named(SANS_FAMILY, italic),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        let mut lines = extract_lines(&buffer);
        assert_eq!(
            lines.len(),
            1,
            "the fixture {text:?} must shape to one line"
        );
        lines.pop().expect("BUG: a one-line fixture has a line")
    }

    /// Every quad one line submits, through a cache of its own so no raster
    /// carries over between the variants under comparison.
    fn quads_at(
        font_system: &mut FontSystem,
        swash: &mut SwashCache,
        glyphs: &[super::PositionedGlyphInfo],
        origin_x: f32,
        alphabetic_y: f32,
        size: f32,
    ) -> Vec<femtovg::Quad> {
        build_glyph_commands(
            &mut MockBackend::default(),
            &mut GlyphCache::new(),
            swash,
            font_system,
            &no_fonts(),
            glyphs,
            origin_x,
            alphabetic_y,
            size,
        )
        .into_iter()
        .flat_map(|command| match command {
            GlyphCommand::Quads { quads, .. } => quads,
            GlyphCommand::Direct(_) => panic!("BUG: below the cutoff nothing may delegate"),
        })
        .collect()
    }

    /// The coverage the cache stores for a glyph — keyed with the subpixel bins
    /// normalized away, which is what makes one raster serve every origin.
    fn normalized_raster(
        swash: &mut SwashCache,
        font_system: &mut FontSystem,
        glyph: &super::PositionedGlyphInfo,
    ) -> crate::gpu::glyph_cache::RasterGlyph {
        rasterize_glyph(swash, font_system, GlyphKey::normalize(glyph.key).inner())
            .expect("BUG: a cached glyph has no coverage")
    }

    /// A fractional origin may move a glyph only by the whole pixels it snaps
    /// to: the quad stays on the integer grid, within a pixel of the exact
    /// position, off a raster the origin never reaches. Sub-pixel drift is what
    /// makes a string shimmer as an animation slides it across the screen.
    #[test]
    fn fractional_origins_snap_consistently() {
        let mut font_system = font_system();
        let mut swash = SwashCache::new();
        let size = 24.0;
        // No spaces: an empty box submits no quad, and this compares per glyph.
        let line = shape_line(&mut font_system, "Wij.", size, false);
        let rasters: Vec<_> = line
            .glyphs
            .iter()
            .map(|glyph| normalized_raster(&mut swash, &mut font_system, glyph))
            .collect();

        let mut reference: Option<Vec<femtovg::Quad>> = None;
        let mut snapped_apart = false;
        for (origin_x, baseline_y) in [
            (10.0_f32, 20.0_f32),
            (10.4, 20.4),
            (10.9, 20.9),
            (-10.6, -20.6),
        ] {
            let quads = quads_at(
                &mut font_system,
                &mut swash,
                &line.glyphs,
                origin_x,
                baseline_y,
                size,
            );
            assert_eq!(
                quads.len(),
                line.glyphs.len(),
                "every fixture glyph must cache a quad"
            );

            for ((glyph, raster), quad) in line.glyphs.iter().zip(&rasters).zip(&quads) {
                assert_eq!(
                    (quad.x0.fract(), quad.y0.fract()),
                    (0.0, 0.0),
                    "quad ({}, {}) left the integer grid",
                    quad.x0,
                    quad.y0,
                );
                let exact_x = origin_x + glyph.x + raster.left as f32;
                let exact_y = baseline_y + glyph.y - raster.top as f32;
                assert!(
                    (quad.x0 - exact_x).abs() < 1.0,
                    "x snapped to {} from {exact_x}",
                    quad.x0,
                );
                assert!(
                    (quad.y0 - exact_y).abs() <= 0.5,
                    "y snapped to {} from {exact_y}",
                    quad.y0,
                );
            }

            match &reference {
                None => reference = Some(quads),
                Some(first) => {
                    for (quad, base) in quads.iter().zip(first) {
                        assert_eq!(
                            (
                                quad.x1 - quad.x0,
                                quad.y1 - quad.y0,
                                quad.s0,
                                quad.t0,
                                quad.s1,
                                quad.t1
                            ),
                            (
                                base.x1 - base.x0,
                                base.y1 - base.y0,
                                base.s0,
                                base.t0,
                                base.s1,
                                base.t1
                            ),
                            "the origin reached the raster",
                        );
                        snapped_apart |= (quad.x0 - base.x0).abs() > 0.5;
                    }
                }
            }
        }
        assert!(
            snapped_apart,
            "the origins must snap apart, or nothing is being compared"
        );
    }

    /// The four femtovg anchors must move a line by the offsets the layout's
    /// own ascent and descent describe. Deriving them from `line_y − line_top`
    /// instead folds in half the leading, which only a non-alphabetic anchor
    /// ever shows.
    #[test]
    fn all_baselines_place_against_cosmic_metrics() {
        let mut font_system = font_system();
        let mut swash = SwashCache::new();
        let size = 24.0;
        let line = shape_line(&mut font_system, "Hxg", size, false);
        assert!(
            line.max_ascent > 1.0 && line.max_descent > 1.0,
            "the metrics must separate the four anchors"
        );
        let anchor_y = 40.0_f32;

        let mut quads_for = |font_system: &mut FontSystem, baseline| {
            let alphabetic_y =
                baseline_to_alphabetic(anchor_y, baseline, line.max_ascent, line.max_descent);
            quads_at(
                font_system,
                &mut swash,
                &line.glyphs,
                ORIGIN_X,
                alphabetic_y,
                size,
            )
        };

        let alphabetic = quads_for(&mut font_system, femtovg::Baseline::Alphabetic);
        for (baseline, expected) in [
            (femtovg::Baseline::Top, line.max_ascent),
            (femtovg::Baseline::Bottom, -line.max_descent),
            (
                femtovg::Baseline::Middle,
                (line.max_ascent - line.max_descent) / 2.0,
            ),
        ] {
            for (quad, base) in quads_for(&mut font_system, baseline)
                .iter()
                .zip(&alphabetic)
            {
                let moved = quad.y0 - base.y0;
                assert!(
                    (moved - expected).abs() <= 1.0,
                    "{baseline:?} moved the line by {moved}, not the metric-derived {expected}",
                );
            }
        }
    }

    /// Where the cache hands over to femtovg's direct path the pen has to keep
    /// walking: a glyph at 93 px must sit where its 92 px twin sits, scaled.
    /// The two sizes are separate layouts, so what is compared is each glyph's
    /// displacement from the pen origin per pixel of nominal size.
    #[test]
    fn cutoff_is_continuous() {
        let mut font_system = font_system();
        let mut swash = SwashCache::new();
        let (_fonts, font_table) = embedded_font_table(&font_system);
        let cached_size = DIRECT_PATH_CUTOFF_PX;
        let direct_size = DIRECT_PATH_CUTOFF_PX + 1.0;

        for (text, italic) in [("AVAWij", false), ("AVAWij", true), (GREEK_FALLBACK, false)] {
            let cached = shape_line(&mut font_system, text, cached_size, italic);
            let direct = shape_line(&mut font_system, text, direct_size, italic);
            assert_eq!(
                cached.glyphs.len(),
                direct.glyphs.len(),
                "the two sizes must shape {text:?} to the same glyphs"
            );

            let quads = quads_at(
                &mut font_system,
                &mut swash,
                &cached.glyphs,
                ORIGIN_X,
                BASELINE_Y,
                cached_size,
            );
            assert_eq!(
                quads.len(),
                cached.glyphs.len(),
                "every fixture glyph must cache a quad"
            );
            let submitted: Vec<femtovg::PositionedGlyph> =
                chunk_oversized(&direct.glyphs, &font_table, ORIGIN_X, BASELINE_Y)
                    .into_iter()
                    .flat_map(|run| run.glyphs)
                    .collect();
            assert_eq!(submitted.len(), direct.glyphs.len());

            for ((glyph, quad), placed) in cached.glyphs.iter().zip(&quads).zip(&submitted) {
                let raster = normalized_raster(&mut swash, &mut font_system, glyph);
                for (axis, below, above) in [
                    (
                        "x",
                        quad.x0 - raster.left as f32 - ORIGIN_X,
                        placed.x - ORIGIN_X,
                    ),
                    (
                        "y",
                        quad.y0 + raster.top as f32 - BASELINE_Y,
                        placed.y - BASELINE_Y,
                    ),
                ] {
                    let jump = (below / cached_size - above / direct_size) * cached_size;
                    assert!(
                        jump.abs() < 1.0,
                        "{text:?} italic={italic}: {axis} jumps {jump} px across the cutoff",
                    );
                }
            }
        }
    }

    /// The delegated path must submit cosmic's positions, not femtovg's.
    /// `gpu::text`'s header documents that the two shapers kern differently
    /// but names no pair, so the pair is discovered here:
    /// the first candidate the two disagree on at 93 px
    /// is the only one whose placement can tell the shapers apart.
    /// A font swap that made every candidate agree
    /// fails the assertion rather than quietly testing nothing.
    #[test]
    fn delegated_runs_carry_cosmic_kerning() {
        let size = DIRECT_PATH_CUTOFF_PX + 1.0;
        let mut font_system = font_system();
        let (_, font_table) = embedded_font_table(&font_system);
        let divergent = super::divergent_kerning_pair(&mut font_system, size);

        let line = shape_line(&mut font_system, divergent, size, false);
        let submitted: Vec<(f32, f32)> =
            chunk_oversized(&line.glyphs, &font_table, ORIGIN_X, BASELINE_Y)
                .into_iter()
                .flat_map(|run| run.glyphs)
                .map(|glyph| (glyph.x, glyph.y))
                .collect();
        let expected: Vec<(f32, f32)> = line
            .glyphs
            .iter()
            .map(|glyph| snap(ORIGIN_X + glyph.x, BASELINE_Y + glyph.y))
            .collect();
        assert_eq!(
            submitted, expected,
            "{divergent:?} was delegated on femtovg's own advances",
        );
    }

    const SANS_FAMILY: &str = "Braiins Sans";
    const DECK_FAMILY: &str = "Braiins Deck Sans";

    fn named(family: &'static str, italic: bool) -> Attrs<'static> {
        let attrs = Attrs::new().family(Family::Name(family));
        if italic {
            attrs.style(Style::Italic)
        } else {
            attrs
        }
    }

    /// Every point an outline names, in canvas space (y-down).
    fn outline_points(commands: &[cosmic_text::Command]) -> Vec<(f32, f32)> {
        commands
            .iter()
            .flat_map(|command| match *command {
                cosmic_text::Command::MoveTo(p) | cosmic_text::Command::LineTo(p) => vec![p],
                cosmic_text::Command::QuadTo(c, p) => vec![c, p],
                cosmic_text::Command::CurveTo(c0, c1, p) => vec![c0, c1, p],
                cosmic_text::Command::Close => Vec::new(),
            })
            .map(|point| (point.x, point.y))
            .collect()
    }

    fn face_id(font_system: &FontSystem, post_script_name: &str) -> cosmic_text::fontdb::ID {
        font_system
            .db()
            .faces()
            .find(|face| face.post_script_name == post_script_name)
            .expect("BUG: the embedded set has no such face")
            .id
    }

    /// The key of the first glyph a single-word line shapes to,
    /// so the face, weight and flags are the ones cosmic really produces.
    fn first_glyph_key(
        font_system: &mut FontSystem,
        text: &str,
        size: f32,
        italic: bool,
    ) -> cosmic_text::CacheKey {
        let mut buffer = Buffer::new(font_system, Metrics::new(size, size));
        buffer.set_text(
            font_system,
            text,
            &named(SANS_FAMILY, italic),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        let lines = extract_lines(&buffer);
        lines
            .first()
            .and_then(|line| line.glyphs.first())
            .expect("BUG: the fixture shaped no glyph")
            .key
    }
}
