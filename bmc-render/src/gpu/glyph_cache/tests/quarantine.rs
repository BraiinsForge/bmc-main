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

/// [`fill_cold_pages`] gives every glyph a page of its own in key order,
/// so this page holds exactly `test_cache_key(3)`.
const FAULTED_PAGE: usize = 3;

/// Quarantine is release-build behaviour —
/// a debug build asserts on the fault instead —
/// so the tests reach it through the handler, not an upload.
#[test]
fn invariant_fault_quarantines_the_page() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);

    cache.handle_invariant_fault(Faulted::Normal(FAULTED_PAGE));

    assert_eq!(cache.counters().cache_invariant_failures, 1);
    assert!(cache.pages[FAULTED_PAGE].alloc.is_none());
    assert!(!cache.map.contains_key(&test_key(FAULTED_PAGE)));
    assert_eq!(cache.map.len(), MAX_NORMAL_PAGES - 1);
    assert_eq!(cache.slab.len(), MAX_NORMAL_PAGES - 1);
    assert_eq!(
        cache.pages.len(),
        MAX_NORMAL_PAGES,
        "the faulted page stays retained, image and all"
    );

    let rasterized = Cell::new(false);
    let quad = resident(
        cache.get_or_insert(&mut backend, test_cache_key(FAULTED_PAGE), |_| {
            rasterized.set(true);
            Some(page_filling_glyph())
        }),
    );

    assert!(rasterized.get(), "a forgotten key must miss again");
    assert_ne!(quad.page, cache.pages[FAULTED_PAGE].id);
    assert_eq!(
        backend.pages_created, MAX_NORMAL_PAGES,
        "a quarantined page is never replaced"
    );
    assert_eq!(
        cache
            .pages
            .iter()
            .filter(|page| page.alloc.is_some())
            .count(),
        MAX_NORMAL_PAGES - 1,
        "capacity shrinks by the faulted page for good"
    );
}

/// Past the negative cache's capacity, so its oldest keys age out:
/// quarantine kept there would age out with them and retry the fault.
const NEGATIVE_CHURN_STEPS: usize = 300;

#[test]
fn quarantine_survives_negative_churn() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);
    cache.handle_invariant_fault(Faulted::Normal(FAULTED_PAGE));
    let faulted_id = cache.pages[FAULTED_PAGE].id;
    let uploads_before = backend.uploads.len();

    for step in 1..=NEGATIVE_CHURN_STEPS {
        assert_eq!(
            cache.get_or_insert(&mut backend, empty_key_at(step), |_| None),
            GlyphLookup::Missing
        );
    }
    assert_eq!(cache.negative.len, NEGATIVE_CACHE_CAP);

    let quad = resident(
        cache.get_or_insert(&mut backend, test_cache_key(FAULTED_PAGE), |_| {
            Some(page_filling_glyph())
        }),
    );

    assert_ne!(quad.page, faulted_id);
    assert!(cache.pages[FAULTED_PAGE].alloc.is_none());
    assert!(
        backend.uploads[uploads_before..]
            .iter()
            .all(|&(page, ..)| page != faulted_id),
        "nothing may be uploaded to a quarantined page"
    );
}

#[test]
fn scratch_fault_latches_scratch_off() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);
    touch_every_resident_entry(&mut cache, &mut backend);
    let _ = resident(
        cache.get_or_insert(&mut backend, test_cache_key(MAX_NORMAL_PAGES), |_| {
            Some(page_filling_glyph())
        }),
    );
    let pages_created = backend.pages_created;

    cache.handle_invariant_fault(Faulted::Scratch);

    assert!(cache.scratch_latched_off);
    assert!(cache.scratch_map.is_empty());
    assert_eq!(cache.counters().cache_invariant_failures, 1);
    assert_eq!(
        cache.get_or_insert(
            &mut backend,
            test_cache_key(MAX_NORMAL_PAGES + 1),
            |_| Some(page_filling_glyph())
        ),
        GlyphLookup::Dropped
    );
    assert_eq!(cache.counters().glyphs_dropped, 1);

    cache.end_frame();
    touch_every_resident_entry(&mut cache, &mut backend);
    assert_eq!(
        cache.get_or_insert(
            &mut backend,
            test_cache_key(MAX_NORMAL_PAGES + 2),
            |_| Some(page_filling_glyph())
        ),
        GlyphLookup::Dropped,
        "the latch outlives the frame that tripped it"
    );
    assert_eq!(cache.counters().glyphs_dropped, 2);
    assert_eq!(
        backend.pages_created, pages_created,
        "no replacement scratch page is ever created"
    );
}

#[test]
fn all_pages_quarantined_fails_closed() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);

    for page in 0..MAX_NORMAL_PAGES {
        assert!(!cache.failed_closed, "one healthy page is still a cache");
        cache.handle_invariant_fault(Faulted::Normal(page));
    }

    assert!(cache.failed_closed);
    assert!(cache.map.is_empty());
    assert_eq!(cache.slab.len(), 0);
    let pages_created = backend.pages_created;

    for index in 0..3 {
        assert_eq!(
            cache.get_or_insert(&mut backend, test_cache_key(index), |_| panic!(
                "BUG: a failed-closed cache must not rasterize"
            )),
            GlyphLookup::Dropped
        );
    }
    assert_eq!(cache.counters().glyphs_dropped, 3);
    assert_eq!(backend.pages_created, pages_created);
}

/// The mid-insertion state an upload fault rolls back:
/// a rect reserved on a page
/// and a slab slot the map and the LRU have never seen.
fn provisional_entry(
    cache: &mut GlyphCache<usize>,
    backend: &mut test_support::MockBackend,
    key: GlyphKey,
) -> (Reserved, SlotId) {
    let pending = Pending::new(&solid_glyph(30, 30));
    let Ok(reserved) = cache.allocate(backend, pending.size()) else {
        panic!("BUG: the test cache refused a rect it had room for");
    };
    let slot = cache
        .slab
        .alloc(Entry {
            key,
            page: reserved.page,
            alloc_id: reserved.alloc_id,
            content_x: reserved.content_x,
            content_y: reserved.content_y,
            placement: pending.placement,
            last_used: cache.generation,
            prev: None,
            next: None,
        })
        .expect("BUG: slab full in test");
    (reserved, slot)
}

/// Unlinking the provisional entry would rewrite the queue's ends
/// from its absent links, cutting every survivor loose —
/// which only entries on other pages, untouched by the quarantine,
/// can witness.
#[test]
fn invariant_fault_preserves_unrelated_lru_order() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    for index in 0..3 {
        let _ = resident(
            cache.get_or_insert(&mut backend, test_cache_key(index), |_| {
                Some(page_filling_glyph())
            }),
        );
        cache.end_frame();
    }
    let _ = resident(cache.get_or_insert(&mut backend, test_cache_key(1), |_| {
        panic!("BUG: a resident glyph must not rasterize")
    }));

    let (reserved, slot) = provisional_entry(&mut cache, &mut backend, test_key(9));
    assert_eq!(
        reserved.page, 0,
        "the provisional rect must land on the page the fault will quarantine"
    );
    let forgotten = cache.map[&test_key(0)];
    let expected: Vec<SlotId> = cold_to_hot(&cache.slab, &cache.lru)
        .into_iter()
        .filter(|entry| *entry != forgotten)
        .collect();

    cache.roll_back_insertion(reserved, slot);
    cache.handle_invariant_fault(Faulted::Normal(reserved.page));

    assert_eq!(cold_to_hot(&cache.slab, &cache.lru), expected);
    assert_eq!(
        hot_to_cold(&cache.slab, &cache.lru),
        expected.iter().rev().copied().collect::<Vec<SlotId>>()
    );
    assert!(cache.map.contains_key(&test_key(1)));
    assert!(cache.map.contains_key(&test_key(2)));
    assert_eq!(cache.map.len(), 2);
    assert_eq!(cache.slab.len(), 2);
}

/// The fault means our own rect geometry or page bookkeeping is wrong,
/// so a debug build stops at it instead of quarantining and carrying on.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "invariant fault")]
fn an_invariant_upload_fault_panics_in_debug() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::<usize>::new();
    backend.fail_next_upload = Some(PageFaultKind::Invariant);

    let _ = cache.get_or_insert(&mut backend, test_cache_key(1), |_| Some(solid_glyph(3, 5)));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "invariant fault")]
fn an_invariant_scratch_fault_panics_in_debug() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    fill_cold_pages(&mut cache, &mut backend);
    touch_every_resident_entry(&mut cache, &mut backend);
    backend.fail_next_upload = Some(PageFaultKind::Invariant);

    let _ = cache.get_or_insert(&mut backend, test_cache_key(MAX_NORMAL_PAGES), |_| {
        Some(page_filling_glyph())
    });
}
