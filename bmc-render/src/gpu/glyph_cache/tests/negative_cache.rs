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

#[test]
fn a_glyph_with_no_coverage_rasterizes_once() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    let key = test_cache_key(1);

    assert_eq!(
        cache.get_or_insert(&mut backend, key, |_| None),
        GlyphLookup::Missing
    );
    assert_eq!(
        cache.get_or_insert(&mut backend, key, |_| panic!(
            "BUG: a known-empty glyph must not rasterize"
        )),
        GlyphLookup::Missing
    );
    assert_eq!(backend.pages_created, 0);
    assert!(backend.uploads.is_empty());
    assert_eq!(cache.counters().rasterizations, 1);
    assert_eq!(cache.counters().uploads, 0);
    assert_eq!(cache.counters().negative_cache_hits, 1);
}

#[test]
fn hundreds_of_empty_sizes_stay_bounded_and_drop_the_oldest() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();

    for step in 0..EMPTY_SIZE_STEPS {
        assert_eq!(
            cache.get_or_insert(&mut backend, empty_key_at(step), |_| None),
            GlyphLookup::Missing
        );
        assert!(cache.negative.len <= NEGATIVE_CACHE_CAP);
    }
    assert_eq!(cache.negative.len, NEGATIVE_CACHE_CAP);

    assert_eq!(
        cache.get_or_insert(
            &mut backend,
            empty_key_at(EMPTY_SIZE_STEPS - 1),
            |_| panic!("BUG: the newest empty key must still be known")
        ),
        GlyphLookup::Missing
    );

    let mut rasterized = false;
    assert_eq!(
        cache.get_or_insert(&mut backend, empty_key_at(0), |_| {
            rasterized = true;
            None
        }),
        GlyphLookup::Missing
    );
    assert!(rasterized, "the oldest empty key must have aged out");
    assert_eq!(cache.negative.len, NEGATIVE_CACHE_CAP);
    assert_eq!(backend.pages_created, 0);
}

/// The whole structure is inline arrays, so its footprint is its size —
/// nothing behind it can grow, and the spec's 40 bytes per entry holds.
#[test]
fn the_negative_cache_owns_no_heap_storage() {
    assert!(
        size_of::<NegativeCache>() <= NEGATIVE_CACHE_CAP * 40,
        "negative cache footprint {} exceeds its budget",
        size_of::<NegativeCache>()
    );
    assert!(
        size_of::<NegativeCache>() >= NEGATIVE_CACHE_CAP * size_of::<Option<GlyphKey>>(),
        "negative cache footprint {} is below inline storage requirement",
        size_of::<NegativeCache>()
    );
}
