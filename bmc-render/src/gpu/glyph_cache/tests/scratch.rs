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

fn scratch_page_of(cache: &GlyphCache<usize>) -> usize {
    cache
        .scratch
        .expect("BUG: no scratch page was ever created")
}

/// The miss the protected cache cannot serve has to go somewhere,
/// and the scratch page is the only place left that costs no eviction.
#[test]
fn protected_cache_routes_to_scratch() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);
    touch_every_resident_entry(&mut cache, &mut backend);

    let quad = resident(cache.get_or_insert(
        &mut backend,
        test_cache_key(MAX_NORMAL_PAGES),
        |_| Some(page_filling_glyph()),
    ));

    assert_eq!(quad.page, scratch_page_of(&cache));
    assert_eq!(cache.counters().scratch_uses, 1);
    assert_eq!(cache.counters().evictions, 0);
    assert_eq!(cache.map.len(), MAX_NORMAL_PAGES);
    assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
}

/// A repeat of an unservable glyph must not re-run the eviction sweep:
/// every attempt destroys up to `MAX_EVICTIONS_PER_MISS` cold entries,
/// so one glyph repeated in a word would strip the cache several times.
#[test]
fn scratch_dedups_within_frame() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    let inserted = fill_with_small_glyphs(&mut cache, &mut backend);
    let evictions_before = cache.counters().evictions;
    let rasterizations_before = cache.counters().rasterizations;
    let counted_uploads_before = cache.counters().uploads;
    let uploads_before = backend.uploads.len();

    let key = test_cache_key(inserted);
    let rasterizations = Cell::new(0_usize);
    let quads: Vec<GlyphQuad<usize>> = (0..3)
        .map(|_| {
            resident(cache.get_or_insert(&mut backend, key, |_| {
                rasterizations.set(rasterizations.get() + 1);
                Some(solid_glyph(300, 300))
            }))
        })
        .collect();

    assert_eq!(rasterizations.get(), 1);
    assert_eq!(cache.counters().rasterizations, rasterizations_before + 1);
    assert!(quads.iter().all(|quad| *quad == quads[0]));
    assert_eq!(quads[0].page, scratch_page_of(&cache));
    assert_eq!(backend.uploads.len(), uploads_before + 1);
    assert_eq!(cache.counters().uploads, counted_uploads_before + 1);
    assert_eq!(cache.counters().scratch_uses, 3);
    assert_eq!(
        cache.counters().evictions - evictions_before,
        u64::try_from(MAX_EVICTIONS_PER_MISS).expect("BUG: bound outside the counter range"),
        "the repeats must not sweep the cache again"
    );
}

/// The image is what must survive the flush, never its contents:
/// a glyph on the scratch page next frame is one that got there again.
#[test]
fn scratch_resets_after_flush_but_keeps_image() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);
    touch_every_resident_entry(&mut cache, &mut backend);

    let key = test_cache_key(MAX_NORMAL_PAGES);
    let first = resident(cache.get_or_insert(&mut backend, key, |_| Some(page_filling_glyph())));
    let page = scratch_page_of(&cache);
    let pages_created = backend.pages_created;

    cache.end_frame();
    assert!(cache.scratch_map.is_empty());
    assert_eq!(cache.scratch, Some(page));

    touch_every_resident_entry(&mut cache, &mut backend);
    let rasterizations = Cell::new(0_usize);
    let second = resident(cache.get_or_insert(&mut backend, key, |_| {
        rasterizations.set(rasterizations.get() + 1);
        Some(page_filling_glyph())
    }));

    assert_eq!(rasterizations.get(), 1);
    assert_eq!(second, first);
    assert_eq!(backend.pages_created, pages_created);
}

/// Two page-filling glyphs cannot share the 512 px scratch page,
/// so the second one is what the frame gives up on.
#[test]
fn scratch_full_drops_and_counts() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);
    touch_every_resident_entry(&mut cache, &mut backend);

    let _ = resident(
        cache.get_or_insert(&mut backend, test_cache_key(MAX_NORMAL_PAGES), |_| {
            Some(page_filling_glyph())
        }),
    );
    assert_eq!(cache.counters().glyphs_dropped, 0);

    assert_eq!(
        cache.get_or_insert(
            &mut backend,
            test_cache_key(MAX_NORMAL_PAGES + 1),
            |_| Some(page_filling_glyph())
        ),
        GlyphLookup::Dropped
    );
    assert_eq!(cache.counters().glyphs_dropped, 1);
    assert_eq!(cache.counters().evictions, 0);
    assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
}

/// Tiny glyphs would pack thousands of frame-local entries into one page,
/// so the map — not the pixels — is what bounds the scratch metadata.
#[test]
fn scratch_map_cap_drops() {
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
    for index in 0..MAX_RESIDENT_ENTRIES {
        let _ = resident(
            cache.get_or_insert(&mut backend, test_cache_key(index), |_| {
                panic!("BUG: a resident glyph must not rasterize")
            }),
        );
    }

    for extra in 0..SCRATCH_MAP_CAP {
        let _ = resident(cache.get_or_insert(
            &mut backend,
            test_cache_key(MAX_RESIDENT_ENTRIES + extra),
            |_| Some(solid_glyph(12, 12)),
        ));
    }
    assert_eq!(cache.scratch_map.len(), SCRATCH_MAP_CAP);
    assert_eq!(cache.counters().glyphs_dropped, 0);

    assert_eq!(
        cache.get_or_insert(
            &mut backend,
            test_cache_key(MAX_RESIDENT_ENTRIES + SCRATCH_MAP_CAP),
            |_| Some(solid_glyph(12, 12))
        ),
        GlyphLookup::Dropped
    );
    assert_eq!(cache.counters().glyphs_dropped, 1);
    assert_eq!(cache.counters().evictions, 0);
}

#[test]
fn churn_never_exceeds_resident_and_scratch_page_cap() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);

    let mut key = MAX_NORMAL_PAGES;
    for _ in 0..8 {
        for _ in 0..MAX_NORMAL_PAGES + 5 {
            let _ = cache.get_or_insert(&mut backend, test_cache_key(key), |_| {
                Some(page_filling_glyph())
            });
            key += 1;
        }
        cache.end_frame();

        for _ in 0..64 {
            let _ = cache.get_or_insert(&mut backend, test_cache_key(key), |_| {
                Some(solid_glyph(30, 30))
            });
            key += 1;
        }
        cache.end_frame();
    }

    assert!(cache.counters().evictions > 0);
    assert!(cache.counters().scratch_uses > 0);
    assert!(cache.counters().glyphs_dropped > 0);
    assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
}

#[test]
fn scratch_create_failure_is_transient_and_counted() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);
    touch_every_resident_entry(&mut cache, &mut backend);

    let key = test_cache_key(MAX_NORMAL_PAGES);
    backend.fail_next_create = true;
    assert_eq!(
        cache.get_or_insert(&mut backend, key, |_| Some(page_filling_glyph())),
        GlyphLookup::Missing
    );
    assert_eq!(cache.counters().page_create_failures, 1);
    assert_eq!(cache.scratch, None);
    assert_eq!(backend.pages_created, MAX_NORMAL_PAGES);

    let quad = resident(cache.get_or_insert(&mut backend, key, |_| Some(page_filling_glyph())));
    assert_eq!(quad.page, scratch_page_of(&cache));
    assert_eq!(cache.counters().page_create_failures, 1);
    assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
}

/// A failed upload must return its reservation so the next glyph can
/// still use the whole scratch page.
#[test]
fn scratch_upload_failure_releases_allocation() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);
    touch_every_resident_entry(&mut cache, &mut backend);

    backend.fail_next_upload = Some(PageFaultKind::Transient);
    assert_eq!(
        cache.get_or_insert(&mut backend, test_cache_key(MAX_NORMAL_PAGES), |_| Some(
            page_filling_glyph()
        )),
        GlyphLookup::Missing
    );
    assert_eq!(cache.counters().upload_transient_failures, 1);
    assert!(cache.scratch_map.is_empty());
    assert!(cache.scratch_packer.is_empty());

    let padded = PAGE_FILLING_GLYPH_PX + 2;
    let _ = resident(cache.get_or_insert(
        &mut backend,
        test_cache_key(MAX_NORMAL_PAGES + 1),
        |_| Some(page_filling_glyph()),
    ));

    let &(page, x, y, width, height) = backend
        .uploads
        .last()
        .expect("BUG: the scratch insertion uploaded nothing");
    assert_eq!(
        (page, x, y, width, height),
        (scratch_page_of(&cache), 0, 0, padded, padded)
    );
    assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
}
