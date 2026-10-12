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

//! The layout cache: hits across frames, LRU and glyph-limit eviction,
//! and the profile counters.
//! CPU-only — nothing here needs a GL context.

use super::{font_system, line_style};

use super::{
    LAYOUT_CACHE_CAPACITY, LAYOUT_CACHE_GLYPH_CAPACITY, LayoutDomain, LookupPhase,
    ParagraphLayoutCache, ParagraphLayoutEntry, single_line_cache_key,
};
use crate::tree::{SpanData, TextStyle};

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
fn large_layout_evicts_multiple_entries_in_lru_order() {
    fn insert(cache: &mut ParagraphLayoutCache, key: u64, glyph_count: usize) {
        cache.lookup_or_insert(key, LayoutDomain::SingleLine, LookupPhase::Draw, || {
            ParagraphLayoutEntry::with_glyph_count(glyph_count)
        });
    }

    let mut cache = ParagraphLayoutCache::new();
    let small_glyphs = LAYOUT_CACHE_GLYPH_CAPACITY
        .checked_div(4)
        .expect("BUG: nonzero divisor rejected");
    for key in 0..4 {
        insert(&mut cache, key, small_glyphs);
    }
    cache.lookup_or_insert(0, LayoutDomain::SingleLine, LookupPhase::Draw, || {
        panic!("BUG: the promoted layout unexpectedly missed")
    });

    insert(&mut cache, 4, 3 * small_glyphs);

    assert_eq!(cache.counters.capacity_evictions, 3);
    assert_eq!(cache.resident_glyphs, LAYOUT_CACHE_GLYPH_CAPACITY);
    assert!(
        cache.entries.contains_key(&0),
        "the promoted entry must survive"
    );
    assert!(
        cache.entries.contains_key(&4),
        "the incoming entry must be resident"
    );
    for key in 1..4 {
        assert!(
            !cache.entries.contains_key(&key),
            "cold entry {key} must be evicted"
        );
    }
}

#[test]
fn resident_glyph_limit_evicts_before_entry_limit() {
    let mut font_system = font_system();
    let mut cache = ParagraphLayoutCache::new();
    let style = line_style(24.0, false);

    for index in 0..128 {
        let text = format!("{index:03}{}", "x".repeat(125));
        cache.layout_single_line(&mut font_system, style, &text);
    }

    let retained_glyphs = cache
        .entries
        .values()
        .map(|entry| entry.glyph_count)
        .sum::<usize>();
    assert_eq!(cache.resident_glyphs, retained_glyphs);
    assert!(
        cache.resident_glyphs <= LAYOUT_CACHE_GLYPH_CAPACITY,
        "retained layouts must stay within the glyph ceiling"
    );
    assert!(
        cache.entries.len() < LAYOUT_CACHE_CAPACITY,
        "the glyph ceiling must apply before the entry ceiling"
    );
    assert!(
        cache.counters.capacity_evictions > 0,
        "crossing the glyph ceiling must evict cached layouts"
    );
}

#[test]
fn layout_larger_than_glyph_limit_is_not_cached() {
    let mut font_system = font_system();
    let mut cache = ParagraphLayoutCache::new();
    let style = line_style(24.0, false);
    let text = "x".repeat(LAYOUT_CACHE_GLYPH_CAPACITY + 1);

    let glyph_count = cache
        .layout_single_line(&mut font_system, style, &text)
        .glyph_count;

    assert!(glyph_count > LAYOUT_CACHE_GLYPH_CAPACITY);
    assert!(cache.entries.is_empty());
    assert_eq!(cache.resident_glyphs, 0);
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

    cache.remove_resident(single_line_cache_key(style, "measured"));
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
    cache.remove_resident(key);
    cache.layout_single_line(&mut font_system, style, "measured");

    cache.begin_frame();
    cache.remove_resident(key);
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
        size: None,
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
