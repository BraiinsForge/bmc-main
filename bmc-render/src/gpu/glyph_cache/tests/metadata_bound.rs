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

const METADATA_CEILING_BYTES: usize = 3 * 1024 * 1024;

const GLYPH_IDS_PER_SIZE: usize = 1 << 16;

/// `test_cache_key` runs out of distinct keys at the 16-bit glyph id,
/// and a churn outrunning the resident cap many times over needs more,
/// so the size moves on whenever the glyph ids wrap.
#[expect(
    clippy::integer_division,
    reason = "the size advances once per exhausted run of glyph ids"
)]
fn churn_key(step: usize) -> cosmic_text::CacheKey {
    let size_step =
        u16::try_from(step / GLYPH_IDS_PER_SIZE).expect("BUG: churn step outside the size range");
    cosmic_text::CacheKey {
        font_size_bits: (17.0 + f32::from(size_step) / 16.0).to_bits(),
        ..test_cache_key(step % GLYPH_IDS_PER_SIZE)
    }
}

/// Every path that could enlarge a container, in one run:
/// tiny glyphs past the resident cap with their predecessors cold,
/// so eviction churns the slab and the key map;
/// the same again with every predecessor drawn this frame,
/// so the scratch page and its map fill and then drop;
/// and empty keys past the negative cache.
/// All 21 page textures exist by the end.
fn churn_past_every_cap(
    cache: &mut GlyphCache<usize>,
    backend: &mut test_support::MockBackend,
    cold_steps: usize,
) {
    fill_cold_pages(cache, backend);

    for step in 0..cold_steps {
        let _ = cache.get_or_insert(backend, churn_key(step), |_| Some(solid_glyph(2, 2)));
        cache.end_frame();
        backend.uploads.clear();
    }

    let hot_steps = MAX_RESIDENT_ENTRIES + SCRATCH_MAP_CAP + 1;
    for step in cold_steps..cold_steps + hot_steps {
        let _ = cache.get_or_insert(backend, churn_key(step), |_| Some(solid_glyph(2, 2)));
    }
    cache.end_frame();
    backend.uploads.clear();

    for step in 0..EMPTY_SIZE_STEPS {
        let _ = cache.get_or_insert(backend, empty_key_at(step), |_| None);
    }
    cache.end_frame();
}

/// Long enough to cross every cap, and short of the eviction count
/// at which hashbrown gives up rehashing the key map in place.
const BOUNDED_CHURN_STEPS: usize = 2 * MAX_RESIDENT_ENTRIES;

/// Nothing a frame does may enlarge a container `new()` sized;
/// the estimates still fit once all resident and scratch pages exist.
#[test]
fn metadata_bound_holds_at_allocated_capacity() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    let when_new = cache.app_owned_metadata_bytes();

    churn_past_every_cap(&mut cache, &mut backend, BOUNDED_CHURN_STEPS);

    assert!(cache.counters().evictions > 0);
    assert!(cache.counters().scratch_uses > 0);
    assert!(cache.counters().glyphs_dropped > 0);
    assert_eq!(backend.pages_created, MAX_NORMAL_PAGES + 1);
    assert_eq!(cache.app_owned_metadata_bytes(), when_new);
    assert!(
        cache.metadata_capacity_bytes() <= METADATA_CEILING_BYTES,
        "metadata {} exceeds the ceiling",
        cache.metadata_capacity_bytes()
    );
    assert_eq!(
        cache.resident_atlas_bytes(),
        (MAX_NORMAL_PAGES + 1) * PAGE_SIZE_PX * PAGE_SIZE_PX
    );
}

/// Past this many evictions hashbrown stops rehashing the key map in place
/// and doubles its table instead: tombstones exhaust the slack of a table
/// held at 8 192 live entries.
const SUSTAINED_CHURN_STEPS: usize = 150_000;

/// So the ceiling covers the table hashbrown ends up with,
/// not the one `new()` reserved: a bound that only held before the resize
/// is a bound the renderer outgrows in service.
#[test]
fn the_ceiling_covers_the_key_maps_grown_table() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    let when_new = cache.app_owned_metadata_bytes();

    churn_past_every_cap(&mut cache, &mut backend, SUSTAINED_CHURN_STEPS);

    assert!(
        cache.app_owned_metadata_bytes() > when_new,
        "the churn no longer reaches the resize, so the ceiling proves nothing here"
    );
    assert!(
        cache.metadata_capacity_bytes() <= METADATA_CEILING_BYTES,
        "metadata {} exceeds the ceiling",
        cache.metadata_capacity_bytes()
    );
}

/// A frame can submit arbitrarily many batches, so the inspection record
/// drops what it cannot hold rather than growing to fit — otherwise one
/// long frame would leave its capacity resident for the renderer's life.
#[cfg(feature = "atlas-inspect")]
#[test]
fn batches_past_the_record_cap_are_counted_not_stored() {
    let mut cache = GlyphCache::<usize>::new();
    let over_cap = 3 * MAX_RECORDED_BATCHES;

    for batch in 0..MAX_RECORDED_BATCHES + over_cap {
        cache.record_drawn_page(batch);
    }
    cache.retire_pages_drawn();

    assert_eq!(cache.pages_drawn_last_frame().len(), MAX_RECORDED_BATCHES);
    assert_eq!(cache.batches_over_cap_last_frame(), over_cap);
    assert_eq!(
        cache.app_owned_metadata_bytes(),
        GlyphCache::<usize>::new().app_owned_metadata_bytes(),
        "the record grew past the capacity it was built with"
    );
}

/// hashbrown's widest control group; a narrower target is over-counted,
/// which is the safe direction for a ceiling.
const HASHBROWN_GROUP_BYTES: usize = 16;

/// The spec's accounting table models hashbrown 0.16.1's table
/// from a capacity: a power-of-two bucket count at a 7/8 load factor,
/// with one control byte per bucket
/// and a trailing group the probe reads past the end.
#[expect(
    clippy::integer_division,
    reason = "hashbrown's own bucket rounding, truncation included"
)]
fn map_bucket_bytes(capacity: usize, kv_size: usize) -> usize {
    let buckets = (capacity * 8 / 7).next_power_of_two();
    buckets * kv_size + buckets + HASHBROWN_GROUP_BYTES
}

/// The ceiling was derived from that model at the two maps' fresh
/// capacities. A bump to the load factor, the rounding or the control bytes
/// would leave the model under the allocation, and the derivation void.
#[test]
fn the_spec_model_covers_hashbrowns_own_allocation() {
    let map = hashbrown::HashMap::<GlyphKey, SlotId>::with_capacity(MAX_RESIDENT_ENTRIES);
    assert_eq!(map.capacity(), 14_336);
    assert!(
        map_bucket_bytes(map.capacity(), size_of::<(GlyphKey, SlotId)>()) >= map.allocation_size()
    );

    let scratch = hashbrown::HashMap::<GlyphKey, ScratchEntry>::with_capacity(SCRATCH_MAP_CAP);
    assert_eq!(scratch.capacity(), 1_792);
    assert!(
        map_bucket_bytes(scratch.capacity(), size_of::<(GlyphKey, ScratchEntry)>())
            >= scratch.allocation_size()
    );
}

/// Why the counter asks hashbrown what it allocated
/// instead of rebuilding the table from `capacity()`:
/// capacity is `len + growth_left`, and removals that leave tombstones
/// drain that slack without returning a single bucket.
/// Modelled from the drained capacity,
/// a grown table prices at half what it holds.
#[test]
fn a_tombstone_drained_table_is_priced_at_its_real_allocation() {
    let mut map = hashbrown::HashMap::<GlyphKey, SlotId>::with_capacity(MAX_RESIDENT_ENTRIES);
    let fresh_allocation = map.allocation_size();
    let mut inserted = 0;
    while map.allocation_size() == fresh_allocation {
        map.insert(test_key(inserted), SlotId::from_index(0));
        inserted += 1;
    }
    while map.len() < map.capacity() {
        map.insert(test_key(inserted), SlotId::from_index(0));
        inserted += 1;
    }
    let grown_allocation = map.allocation_size();

    for step in 0..inserted {
        if map.len() == MAX_RESIDENT_ENTRIES {
            break;
        }
        map.remove(&test_key(step));
    }

    assert_eq!(map.len(), MAX_RESIDENT_ENTRIES);
    assert!(
        map_bucket_bytes(map.capacity(), size_of::<(GlyphKey, u32)>()) < grown_allocation,
        "capacity {} still prices the whole table, so the drain never happened and this \
         proves nothing",
        map.capacity()
    );
    assert_eq!(map.allocation_size(), grown_allocation);
}
