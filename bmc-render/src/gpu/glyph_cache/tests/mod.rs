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

use std::cell::Cell;

use super::*;
use crate::test_tracing::counting_warns;
use cosmic_text::SubpixelBin;

mod atlas;
mod key_normalization;
mod metadata_bound;
mod negative_cache;
mod pressure_log;
mod quarantine;
mod scratch;
mod slab;
mod subpixel_bins;

/// Cache and slab tests care only that keys stay distinct, so the key is
/// synthesized rather than shaped — shaping a font system per entry would
/// dominate the eight-thousand-entry runs.
fn test_cache_key(index: usize) -> cosmic_text::CacheKey {
    cosmic_text::CacheKey {
        font_id: cosmic_text::fontdb::ID::dummy(),
        glyph_id: u16::try_from(index).expect("BUG: test index outside glyph id range"),
        font_size_bits: 17.0_f32.to_bits(),
        x_bin: SubpixelBin::Zero,
        y_bin: SubpixelBin::Zero,
        font_weight: cosmic_text::fontdb::Weight::NORMAL,
        flags: cosmic_text::CacheKeyFlags::empty(),
    }
}

fn test_key(index: usize) -> GlyphKey {
    GlyphKey(test_cache_key(index))
}

fn hot_to_cold(slab: &EntrySlab, lru: &LruQueue<SlotId>) -> Vec<SlotId> {
    let mut order = Vec::new();
    let mut cursor = lru.hottest();
    while let Some(slot) = cursor {
        order.push(slot);
        cursor = slab.get(slot).next;
    }
    order
}

/// A reverse walk catches link corruption that the forward walk alone hides.
fn cold_to_hot(slab: &EntrySlab, lru: &LruQueue<SlotId>) -> Vec<SlotId> {
    let mut order = Vec::new();
    let mut cursor = lru.coldest();
    while let Some(slot) = cursor {
        order.push(slot);
        cursor = slab.get(slot).prev;
    }
    order
}

fn solid_glyph(width: usize, height: usize) -> RasterGlyph {
    RasterGlyph {
        width,
        height,
        left: 1,
        top: -2,
        coverage: vec![0xFF; width * height],
    }
}

/// Coverage that differs per texel, so a blit shifted by a row or a column
/// shows up as a byte difference; a solid glyph would hide it.
fn ramp_glyph(width: usize, height: usize) -> RasterGlyph {
    RasterGlyph {
        width,
        height,
        left: 1,
        top: -2,
        coverage: (0..width * height)
            .map(|texel| u8::try_from(texel % 251).expect("BUG: ramp value outside the byte range"))
            .collect(),
    }
}

fn resident<P: Copy + Eq + core::fmt::Debug>(lookup: GlyphLookup<P>) -> GlyphQuad<P> {
    match lookup {
        GlyphLookup::Resident(quad) => quad,
        GlyphLookup::Missing => panic!("BUG: expected a resident glyph, got Missing"),
        GlyphLookup::Oversized => panic!("BUG: expected a resident glyph, got Oversized"),
        GlyphLookup::Dropped => panic!("BUG: expected a resident glyph, got Dropped"),
    }
}

/// Padded to 402 px, which leaves no shelf a second one could share.
/// Each of these glyphs takes a page to itself, so twenty of them fill
/// the whole normal budget — instead of thousands of small ones.
const PAGE_FILLING_GLYPH_PX: usize = 400;

fn page_filling_glyph() -> RasterGlyph {
    solid_glyph(PAGE_FILLING_GLYPH_PX, PAGE_FILLING_GLYPH_PX)
}

/// Fills every normal page with entries no frame still references:
/// each insert ends its own frame, so no entry carries this generation
/// and every one of them is an eviction candidate.
fn fill_cold_pages(cache: &mut GlyphCache<usize>, backend: &mut test_support::MockBackend) {
    for index in 0..MAX_NORMAL_PAGES {
        let _ = resident(cache.get_or_insert(backend, test_cache_key(index), |_| {
            Some(page_filling_glyph())
        }));
        cache.end_frame();
    }
    assert_eq!(
        backend.pages_created, MAX_NORMAL_PAGES,
        "BUG: a page-filling glyph must open a page of its own"
    );
    assert_eq!(cache.counters().evictions, 0);
}

/// Draws every entry of a full atlas in the current frame,
/// leaving the cache with no eviction candidate at all.
fn touch_every_resident_entry(
    cache: &mut GlyphCache<usize>,
    backend: &mut test_support::MockBackend,
) {
    for index in 0..MAX_NORMAL_PAGES {
        let _ = resident(cache.get_or_insert(backend, test_cache_key(index), |_| {
            panic!("BUG: a resident glyph must not rasterize")
        }));
    }
}

/// Fills the normal pages with 32 px padded rects,
/// stopping once eviction proves they are full.
/// How many rects fit a page is etagere's packing to decide, not ours.
fn fill_with_small_glyphs(
    cache: &mut GlyphCache<usize>,
    backend: &mut test_support::MockBackend,
) -> usize {
    let mut inserted = 0;
    while cache.counters().evictions == 0 {
        assert!(inserted < 20_000, "BUG: the normal pages never filled");
        let _ = resident(cache.get_or_insert(backend, test_cache_key(inserted), |_| {
            Some(solid_glyph(30, 30))
        }));
        cache.end_frame();
        inserted += 1;
    }
    inserted
}

/// Distinct exact-size keys of the kind an animated font size mints,
/// one per frame, for every space character.
/// The step divides exactly, so no two steps can collide in `font_size_bits`.
fn empty_key_at(step: usize) -> cosmic_text::CacheKey {
    let step = f32::from(u16::try_from(step).expect("BUG: test step outside the size range"));
    cosmic_text::CacheKey {
        font_size_bits: (17.0 + step / 16.0).to_bits(),
        ..test_cache_key(1)
    }
}

/// Sizes far past the cap, because the negative cache exists to survive
/// exactly that: an unbounded `None` map would grow the heap forever.
const EMPTY_SIZE_STEPS: usize = 500;
