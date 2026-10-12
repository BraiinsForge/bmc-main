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

/// Derived through `f32::from` rather than the cache's own `uv`,
/// so the expectation is arithmetic the implementation does not supply.
fn uv_of(px: usize) -> f32 {
    f32::from(u16::try_from(px).expect("BUG: test coordinate outside page range")) / 512.0
}

/// The odd 3 px width is deliberate: its padded row stride of 5 is
/// accepted by no GL unpack alignment above 1.
#[test]
fn miss_uploads_content_rect_with_zeroed_border() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    let key = test_cache_key(1);

    let quad = resident(cache.get_or_insert(&mut backend, key, |normalized| {
        assert_eq!(normalized, GlyphKey::normalize(key));
        Some(solid_glyph(3, 5))
    }));
    assert_eq!(cache.counters().rasterizations, 1);
    assert_eq!(cache.counters().uploads, 1);
    assert_eq!(cache.counters().negative_cache_hits, 0);

    let &[(page, x, y, width, height)] = backend.uploads.as_slice() else {
        panic!(
            "BUG: expected exactly one upload, got {:?}",
            backend.uploads
        );
    };
    assert_eq!((width, height), (5, 7));
    assert_eq!(
        quad,
        GlyphQuad {
            page,
            u0: uv_of(x + 1),
            v0: uv_of(y + 1),
            u1: uv_of(x + 4),
            v1: uv_of(y + 6),
            placement: RasterPlacement {
                left: 1,
                top: -2,
                width: 3,
                height: 5,
            },
        }
    );

    let pixels = &backend.pages[page].pixels;
    for row in 0..height {
        for col in 0..width {
            let on_border = row == 0 || row == height - 1 || col == 0 || col == width - 1;
            assert_eq!(
                pixels[(y + row) * PAGE_SIZE_PX + x + col],
                if on_border { 0x00 } else { 0xFF },
                "content texel ({col}, {row})"
            );
        }
    }
}

#[test]
fn hit_returns_same_quad_without_rasterizing() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    let key = test_cache_key(1);

    let first = resident(cache.get_or_insert(&mut backend, key, |_| Some(solid_glyph(3, 5))));
    let second = resident(cache.get_or_insert(&mut backend, key, |_| {
        panic!("BUG: a hit must not rasterize")
    }));

    assert_eq!(second, first);
    assert_eq!(backend.uploads.len(), 1);
    assert_eq!(backend.pages_created, 1);
}

#[test]
fn upload_failure_rolls_back_the_allocation() {
    let key = test_cache_key(1);
    let mut control_backend = test_support::MockBackend::default();
    let mut control = GlyphCache::new();
    let _ = control.get_or_insert(&mut control_backend, key, |_| Some(solid_glyph(3, 5)));
    let clean_rect = *control_backend
        .uploads
        .first()
        .expect("BUG: the control insertion uploaded nothing");

    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    backend.fail_next_upload = Some(PageFaultKind::Transient);

    assert_eq!(
        cache.get_or_insert(&mut backend, key, |_| Some(solid_glyph(3, 5))),
        GlyphLookup::Missing
    );
    assert_eq!(cache.counters().upload_transient_failures, 1);
    assert_eq!(cache.counters().rasterizations, 1);
    assert_eq!(cache.counters().uploads, 0);
    assert!(!cache.map.contains_key(&GlyphKey::normalize(key)));
    assert_eq!(cache.slab.len(), 0);
    assert!(backend.uploads.is_empty());

    let _ = resident(cache.get_or_insert(&mut backend, key, |_| Some(solid_glyph(3, 5))));
    assert_eq!(backend.uploads, vec![clean_rect]);
}

/// The sizes are chosen against etagere's shelf rule —
/// a shelf serves a request
/// only while its surplus height is at most the request's own height —
/// so the 16 px shelf the first rect opens still takes the second,
/// while anything shorter would open a shelf of its own.
///
/// Asserting the recorded upload rects rather than page texels is
/// deliberate: the page starts zeroed, so "texels outside stayed 0" would
/// also hold if the whole zero-filled shelf surplus had been uploaded.
#[test]
fn two_glyph_sizes_share_a_shelf_without_overlap() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();

    let _ =
        resident(cache.get_or_insert(&mut backend, test_cache_key(1), |_| Some(solid_glyph(8, 8))));
    let _ =
        resident(cache.get_or_insert(&mut backend, test_cache_key(2), |_| Some(solid_glyph(6, 6))));

    let &[
        (first_page, first_x, first_y, 10, 10),
        (second_page, second_x, second_y, 8, 8),
    ] = backend.uploads.as_slice()
    else {
        panic!(
            "BUG: expected one 10x10 and one 8x8 content rect, got {:?}",
            backend.uploads
        );
    };
    assert_eq!(first_page, second_page);
    assert_eq!(first_y, second_y, "both rects must sit on one shelf");
    assert!(
        first_x + 10 <= second_x || second_x + 8 <= first_x,
        "shelf mates overlap: {first_x}..{} and {second_x}..{}",
        first_x + 10,
        second_x + 8
    );
}

#[test]
fn oversized_rasters_are_memoized_without_page_creation() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();

    for (index, dimensions) in [(511, 20), (20, 511)].into_iter().enumerate() {
        let key = test_cache_key(index);
        assert_eq!(
            cache.get_or_insert(&mut backend, key, |_| {
                Some(solid_glyph(dimensions.0, dimensions.1))
            }),
            GlyphLookup::Oversized
        );
        assert_eq!(
            cache.get_or_insert(&mut backend, key, |_| panic!(
                "BUG: a known-oversized glyph must not rasterize"
            )),
            GlyphLookup::Oversized
        );
        assert!(!cache.map.contains_key(&GlyphKey::normalize(key)));
    }
    assert_eq!(backend.pages_created, 0);
    assert!(backend.uploads.is_empty());
    assert_eq!(cache.counters().rasterizations, 2);
    assert_eq!(cache.counters().uploads, 0);
    assert_eq!(cache.counters().negative_cache_hits, 2);
    assert_eq!(cache.counters().glyphs_oversized, 2);
}

#[test]
fn a_full_atlas_evicts_the_coldest_entry_to_serve_a_miss() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);

    let newcomer = test_cache_key(MAX_NORMAL_PAGES);
    let _ = resident(cache.get_or_insert(&mut backend, newcomer, |_| {
        Some(solid_glyph(PAGE_FILLING_GLYPH_PX, PAGE_FILLING_GLYPH_PX))
    }));

    assert_eq!(cache.counters().evictions, 1);
    assert!(
        !cache.map.contains_key(&test_key(0)),
        "the coldest entry is the one that must go"
    );
    assert!(cache.map.contains_key(&GlyphKey::normalize(newcomer)));
    assert_eq!(cache.slab.len(), MAX_NORMAL_PAGES);
    assert_eq!(cache.pages.len(), MAX_NORMAL_PAGES);
    assert_eq!(backend.pages_created, MAX_NORMAL_PAGES);
}

/// 4x4 padded rects are small enough that one page holds all 8192 of them,
/// so the entry cap binds while nineteen pages' worth of pixels stand free.
#[test]
fn entry_cap_evicts_before_pixel_space() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    for index in 0..MAX_RESIDENT_ENTRIES {
        let _ = resident(
            cache.get_or_insert(&mut backend, test_cache_key(index), |_| {
                Some(solid_glyph(2, 2))
            }),
        );
        cache.end_frame();
    }
    assert_eq!(cache.slab.len(), MAX_RESIDENT_ENTRIES);
    assert_eq!(cache.counters().evictions, 0);

    let newcomer = test_cache_key(MAX_RESIDENT_ENTRIES);
    let _ = resident(cache.get_or_insert(&mut backend, newcomer, |_| Some(solid_glyph(2, 2))));

    assert!(cache.counters().evictions > 0);
    assert!(cache.slab.len() <= MAX_RESIDENT_ENTRIES);
    assert_eq!(cache.slab.len(), cache.map.len());
    assert!(!cache.map.contains_key(&test_key(0)));
    assert!(cache.map.contains_key(&GlyphKey::normalize(newcomer)));
    assert!(
        cache.pages.len() < MAX_NORMAL_PAGES,
        "pixel space was never the constraint here"
    );
}

/// Overwriting a rect that a queued draw command still references
/// would corrupt the frame being built, so a cache touched end to end
/// keeps every entry however badly the miss needs the space.
#[test]
fn current_frame_entries_are_never_evicted() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);
    touch_every_resident_entry(&mut cache, &mut backend);

    let _ = cache.get_or_insert(&mut backend, test_cache_key(MAX_NORMAL_PAGES), |_| {
        Some(page_filling_glyph())
    });

    assert_eq!(cache.counters().evictions, 0);
    assert_eq!(cache.map.len(), MAX_NORMAL_PAGES);
    assert_eq!(
        cache.counters().hits,
        u64::try_from(MAX_NORMAL_PAGES).expect("BUG: page cap outside the counter range")
    );
}

/// A 302 px rect needs a shelf taller than any three 32 px shelves
/// coalesced. However long the loop runs, freeing cold entries can never
/// serve it — only the bound stops it.
#[test]
fn eviction_bound_holds() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    let inserted = fill_with_small_glyphs(&mut cache, &mut backend);
    let evictions_before = cache.counters().evictions;

    let _ = cache.get_or_insert(&mut backend, test_cache_key(inserted), |_| {
        Some(solid_glyph(300, 300))
    });

    let bound =
        u64::try_from(MAX_EVICTIONS_PER_MISS).expect("BUG: bound outside the counter range");
    assert_eq!(cache.counters().evictions - evictions_before, bound);
    assert_eq!(cache.counters().max_evictions_per_miss, bound);
}

/// An orphan is an eviction path that skipped the deallocate,
/// and counting live allocations is what catches one.
/// etagere's `allocated_space` cannot: it charges whole shelf rectangles
/// and returns nothing until an entire bucket drains.
#[test]
fn entry_cap_churn_does_not_leak_atlas_space() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    for index in 0..MAX_RESIDENT_ENTRIES * 3 {
        let _ = cache.get_or_insert(&mut backend, test_cache_key(index), |_| {
            Some(solid_glyph(2, 2))
        });
        cache.end_frame();
    }

    let live: usize = cache
        .pages
        .iter()
        .map(|page| page.allocs - page.deallocs)
        .sum();
    assert!(cache.counters().evictions > 0);
    assert_eq!(live, cache.slab.len());
    assert_eq!(live, cache.map.len());
    assert!(cache.slab.len() <= MAX_RESIDENT_ENTRIES);
}
