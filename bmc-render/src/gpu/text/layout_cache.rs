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

//! The paragraph layout cache, keyed by style, spans and box width.

use super::glyphs::FontTable;
use super::glyphs::draw_decorations_for_segment;
use super::glyphs::draw_line_glyphs;
use super::glyphs::to_femtovg_color;
use super::shaping::LineGlyphs;
use super::shaping::LineStyle;
use super::shaping::extract_lines;
use super::shaping::normalize_carriage_returns;
use super::shaping::shape_paragraph;
use super::shaping::shape_single_line;
use super::shaping::span_groups;
use crate::gpu::glyph_cache::GlyphCache;
use crate::gpu::lru::{LruQueue, LruStore};
use crate::renderer::TextLayoutCounters;
use bmc_ui::{Span, TextStyle};
use cosmic_text::FontSystem;
use femtovg::{Canvas, Paint, renderer::OpenGl};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

// Capture profiling reports 341 entries for the two largest production working sets.
// 448 matches HashMap's capacity at the former limit.
pub(super) const LAYOUT_CACHE_CAPACITY: usize = 448;

// The all-widget WASM capture peaked at 9_364 resident glyphs.
// 12 * 1024 leaves about 30% headroom for workloads outside the capture.
pub(super) const LAYOUT_CACHE_GLYPH_CAPACITY: usize = 12 * 1024;

/// Compact render-ready paragraph layout.
pub(crate) struct ParagraphLayoutEntry {
    pub(crate) lines: Vec<LineGlyphs>,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(super) glyph_count: usize,
    prev: Option<u64>,
    next: Option<u64>,
}

impl ParagraphLayoutEntry {
    fn new(lines: Vec<LineGlyphs>, width: f32, height: f32) -> Self {
        let glyph_count = lines.iter().map(|line| line.glyphs.len()).sum();
        Self {
            lines,
            width,
            height,
            glyph_count,
            prev: None,
            next: None,
        }
    }

    #[cfg(test)]
    pub(super) fn with_glyph_count(glyph_count: usize) -> Self {
        Self {
            lines: Vec::new(),
            width: 0.0,
            height: 0.0,
            glyph_count,
            prev: None,
            next: None,
        }
    }
}

const MISSING_LAYOUT_ENTRY: &str = "BUG: layout LRU key absent from the cache";

/// Capacity-bounded paragraph layout cache.
pub struct ParagraphLayoutCache {
    pub(super) entries: hashbrown::HashMap<u64, ParagraphLayoutEntry>,
    lru: LruQueue<u64>,
    transient_entry: Option<ParagraphLayoutEntry>,
    pub(super) resident_glyphs: usize,
    /// Distinguishes a cache hit from a reshape that replaces the same entry,
    /// which entry count and width stability both survive.
    pub(super) counters: LayoutCacheCounters,
    /// Shaping misses this frame. The counters beside it are lifetime totals;
    /// a frame summary wants what this frame paid, which tells a cold cache
    /// from a thrashing one. Reset by [`Self::begin_frame`], read by
    /// [`Self::stats`].
    misses_this_frame: u32,
    profile: Option<Box<LayoutCacheProfile>>,
}

impl LruStore<u64> for hashbrown::HashMap<u64, ParagraphLayoutEntry> {
    fn links(&self, key: u64) -> (Option<u64>, Option<u64>) {
        let entry = self.get(&key).expect(MISSING_LAYOUT_ENTRY);
        (entry.prev, entry.next)
    }

    fn set_links(&mut self, key: u64, prev: Option<u64>, next: Option<u64>) {
        let entry = self.get_mut(&key).expect(MISSING_LAYOUT_ENTRY);
        entry.prev = prev;
        entry.next = next;
    }

    fn set_prev(&mut self, key: u64, prev: Option<u64>) {
        self.get_mut(&key).expect(MISSING_LAYOUT_ENTRY).prev = prev;
    }

    fn set_next(&mut self, key: u64, next: Option<u64>) {
        self.get_mut(&key).expect(MISSING_LAYOUT_ENTRY).next = next;
    }
}

#[derive(Debug, Default)]
pub(super) struct LayoutCacheCounters {
    hits: u64,
    shapes: u64,
    pub(super) capacity_evictions: u64,
    peak_entries: usize,
}

#[derive(Clone, Copy)]
pub(super) enum LookupPhase {
    Measure,
    Draw,
}

#[derive(Clone, Copy, Hash)]
pub(super) enum LayoutDomain {
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
            .field("lru", &self.lru)
            .field(
                "transient_entry",
                &self.transient_entry.as_ref().map(|entry| entry.glyph_count),
            )
            .field("resident_glyphs", &self.resident_glyphs)
            .field("counters", &self.counters)
            .field("misses_this_frame", &self.misses_this_frame)
            .field("profiling_enabled", &self.profile.is_some())
            .finish()
    }
}

impl ParagraphLayoutCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: hashbrown::HashMap::new(),
            lru: LruQueue::new(),
            transient_entry: None,
            resident_glyphs: 0,
            counters: LayoutCacheCounters::default(),
            misses_this_frame: 0,
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

    fn record_lookup(
        &mut self,
        key: u64,
        domain: LayoutDomain,
        phase: LookupPhase,
        cache_hit: bool,
    ) {
        if cache_hit {
            self.counters.hits += 1;
        } else {
            self.counters.shapes += 1;
            self.misses_this_frame += 1;
        }
        self.counters.peak_entries = self.counters.peak_entries.max(self.entries.len());

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

    /// Reset the per-frame miss tally and clear frame-local profiling state.
    ///
    /// Deliberately no sweep by age. One cache serves every slot drawn
    /// through the host renderer, and a slot re-stamps its entries only on
    /// the frames it draws — so any age window narrower than the slot
    /// round-robin evicts whichever slot drew least recently, leaving every
    /// slot re-shaping its whole text set. Eviction is the capacity-based
    /// LRU in [`Self::measure`], which does not depend on how many slots
    /// are live.
    pub fn begin_frame(&mut self) {
        self.misses_this_frame = 0;
        if let Some(profile) = self.profile.as_mut() {
            profile.keys_this_frame.clear();
            profile.shaped_this_frame.clear();
            profile.measured_this_frame.clear();
            profile.weighted_keys_this_frame.clear();
            profile.distinct_glyphs_this_frame.clear();
            profile.glyph_instances_this_frame = 0;
        }
    }

    fn evict_for(&mut self, incoming_glyphs: usize) {
        let max_resident_glyphs = LAYOUT_CACHE_GLYPH_CAPACITY
            .checked_sub(incoming_glyphs)
            .expect("BUG: oversized layout reached resident eviction");
        while self.entries.len() >= LAYOUT_CACHE_CAPACITY
            || self.resident_glyphs > max_resident_glyphs
        {
            let oldest = self
                .lru
                .coldest()
                .expect("BUG: layout cache limits exceeded with an empty LRU");
            self.remove_resident(oldest);
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

    pub(super) fn remove_resident(&mut self, key: u64) -> ParagraphLayoutEntry {
        self.lru.unlink(&mut self.entries, key);
        let entry = self.entries.remove(&key).expect(MISSING_LAYOUT_ENTRY);
        self.resident_glyphs = self
            .resident_glyphs
            .checked_sub(entry.glyph_count)
            .expect("BUG: layout cache glyph count underflowed");
        entry
    }

    pub(super) fn lookup_or_insert(
        &mut self,
        key: u64,
        domain: LayoutDomain,
        phase: LookupPhase,
        make_entry: impl FnOnce() -> ParagraphLayoutEntry,
    ) -> &ParagraphLayoutEntry {
        self.transient_entry = None;

        if self.entries.contains_key(&key) {
            self.lru.promote(&mut self.entries, key);
            self.record_lookup(key, domain, phase, true);
            self.record_profiled_entry(key, domain);
            return self
                .entries
                .get(&key)
                .expect("BUG: layout cache hit disappeared");
        }

        let entry = make_entry();
        if entry.glyph_count > LAYOUT_CACHE_GLYPH_CAPACITY {
            self.record_lookup(key, domain, phase, false);
            self.record_profiled_transient(key, &entry);
            self.transient_entry = Some(entry);
            return self
                .transient_entry
                .as_ref()
                .expect("BUG: transient layout entry disappeared");
        }

        self.evict_for(entry.glyph_count);
        self.resident_glyphs = self
            .resident_glyphs
            .checked_add(entry.glyph_count)
            .expect("BUG: layout cache glyph count overflowed");
        assert!(
            self.entries.insert(key, entry).is_none(),
            "BUG: layout cache miss replaced an existing entry"
        );
        self.lru.push_hot(&mut self.entries, key);
        self.record_lookup(key, domain, phase, false);
        self.record_profiled_entry(key, domain);
        self.entries
            .get(&key)
            .expect("BUG: inserted layout cache entry disappeared")
    }

    fn record_profiled_transient(&mut self, key: u64, entry: &ParagraphLayoutEntry) {
        let Some(profile) = self.profile.as_mut() else {
            return;
        };
        if profile.weighted_keys_this_frame.insert(key) {
            profile.glyph_instances_this_frame += entry.glyph_count;
            profile.counters.layout_cache_peak_frame_glyph_instances = profile
                .counters
                .layout_cache_peak_frame_glyph_instances
                .max(profile.glyph_instances_this_frame);
            for glyph in entry.lines.iter().flat_map(|line| &line.glyphs) {
                profile.distinct_glyphs_this_frame.insert(glyph.key);
            }
            profile.counters.layout_cache_peak_frame_distinct_glyphs = profile
                .counters
                .layout_cache_peak_frame_distinct_glyphs
                .max(profile.distinct_glyphs_this_frame.len());
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
        let glyphs = entry.glyph_count;

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

    /// Shaping misses this frame, and current occupancy.
    #[must_use]
    pub fn stats(&self) -> (u32, usize) {
        (self.misses_this_frame, self.entries.len())
    }

    /// Measure paragraph dimensions, shaping if not cached.
    pub fn measure(
        &mut self,
        font_system: &mut FontSystem,
        base_style: &TextStyle,
        spans: &[Span],
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
        spans: &[Span],
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
        spans: &[Span],
        max_width: Option<f32>,
        phase: LookupPhase,
    ) -> &ParagraphLayoutEntry {
        let key = cache_key(base_style, spans, max_width);
        self.lookup_or_insert(key, LayoutDomain::Paragraph, phase, || {
            // Styles still come from `spans` — normalizing only edits text, never
            // the span count or order, so a glyph's span index stays valid.
            let normalized = normalize_carriage_returns(spans);
            let (buffer, width, height) =
                shape_paragraph(font_system, base_style, &normalized, max_width);
            let lines = extract_lines(&buffer);
            ParagraphLayoutEntry::new(lines, width, height)
        })
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
        self.lookup_or_insert(key, LayoutDomain::SingleLine, phase, || {
            let (buffer, width, height) = shape_single_line(font_system, style, text);
            let lines = extract_lines(&buffer);
            ParagraphLayoutEntry::new(lines, width, height)
        })
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
        spans: &[Span],
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

pub(super) fn layout_cache_hasher(
    domain: LayoutDomain,
) -> std::collections::hash_map::DefaultHasher {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    domain.hash(&mut hasher);
    hasher
}

/// Compute cache key from style + spans + max_width.
///
/// The style half is written out, since `TextStyle` carries floats
/// and cannot derive `Hash`. Naming its fields also keeps the key
/// to what `shape_paragraph` reads, so a paint-only change
/// (an outline colour, say) must not re-shape.
///
/// The spans hash themselves, which is what keeps a field added
/// to `Span` from going missing here.
pub(super) fn cache_key(base_style: &TextStyle, spans: &[Span], max_width: Option<f32>) -> u64 {
    let mut hasher = layout_cache_hasher(LayoutDomain::Paragraph);

    base_style.size.hash(&mut hasher);
    base_style.weight.hash(&mut hasher);
    (base_style.family as u8).hash(&mut hasher);
    base_style.italic.hash(&mut hasher);
    base_style.line_height.to_bits().hash(&mut hasher);
    (base_style.align as u8).hash(&mut hasher);
    (base_style.text_overflow as u8).hash(&mut hasher);
    base_style.max_width.hash(&mut hasher);
    base_style.color.hash(&mut hasher);

    spans.hash(&mut hasher);

    max_width.map(f32::to_bits).hash(&mut hasher);
    hasher.finish()
}

/// Compute the cache key of a single line.
///
/// `to_bits` is what makes the size hashable. Its quirks around `-0.0` and `NaN`
/// never bite, because a size reaching here is always positive.
pub(super) fn single_line_cache_key(style: LineStyle, text: &str) -> u64 {
    let mut hasher = layout_cache_hasher(LayoutDomain::SingleLine);
    (style.family as u8).hash(&mut hasher);
    style.weight.hash(&mut hasher);
    style.italic.hash(&mut hasher);
    style.size.to_bits().hash(&mut hasher);
    text.hash(&mut hasher);
    hasher.finish()
}
