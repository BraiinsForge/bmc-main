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

/// A fixed shuffle rather than the natural order:
/// which bin arrives first must not decide what the cache stores.
const SHUFFLED_BINS: [(SubpixelBin, SubpixelBin); 16] = [
    (SubpixelBin::Two, SubpixelBin::One),
    (SubpixelBin::Zero, SubpixelBin::Three),
    (SubpixelBin::Three, SubpixelBin::Three),
    (SubpixelBin::One, SubpixelBin::Zero),
    (SubpixelBin::Zero, SubpixelBin::One),
    (SubpixelBin::Three, SubpixelBin::Zero),
    (SubpixelBin::Two, SubpixelBin::Three),
    (SubpixelBin::One, SubpixelBin::Two),
    (SubpixelBin::Zero, SubpixelBin::Zero),
    (SubpixelBin::Three, SubpixelBin::One),
    (SubpixelBin::One, SubpixelBin::Three),
    (SubpixelBin::Two, SubpixelBin::Zero),
    (SubpixelBin::Zero, SubpixelBin::Two),
    (SubpixelBin::Three, SubpixelBin::Two),
    (SubpixelBin::One, SubpixelBin::One),
    (SubpixelBin::Two, SubpixelBin::Two),
];

fn keyed_with_bins(x_bin: SubpixelBin, y_bin: SubpixelBin) -> cosmic_text::CacheKey {
    cosmic_text::CacheKey {
        x_bin,
        y_bin,
        ..test_cache_key(1)
    }
}

#[test]
fn sixteen_bin_lookups_rasterize_once_with_zero_bins() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    let mut rasterized = Vec::new();
    let mut quads = Vec::new();

    for (x_bin, y_bin) in SHUFFLED_BINS {
        quads.push(resident(cache.get_or_insert(
            &mut backend,
            keyed_with_bins(x_bin, y_bin),
            |normalized| {
                rasterized.push(normalized);
                Some(ramp_glyph(3, 5))
            },
        )));
    }

    assert_eq!(rasterized.len(), 1);
    assert_eq!(rasterized[0].0.x_bin, SubpixelBin::Zero);
    assert_eq!(rasterized[0].0.y_bin, SubpixelBin::Zero);
    assert_eq!(backend.uploads.len(), 1);
    assert!(quads.iter().all(|quad| *quad == quads[0]));
    assert_eq!(cache.counters().misses, 1);
    assert_eq!(cache.counters().hits, 15);
}

/// The page bytes a fresh cache holds after one insert with these bins.
fn coverage_of_first_insert(x_bin: SubpixelBin, y_bin: SubpixelBin) -> Vec<u8> {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();
    let _ =
        resident(
            cache.get_or_insert(&mut backend, keyed_with_bins(x_bin, y_bin), |normalized| {
                assert_eq!(normalized.0.x_bin, SubpixelBin::Zero);
                assert_eq!(normalized.0.y_bin, SubpixelBin::Zero);
                Some(ramp_glyph(3, 5))
            }),
        );
    backend.pages.remove(0).pixels
}

/// The shuffled run above only exercises whichever bin leads it.
/// A leak of the raw bins into rasterization or placement stays invisible
/// for the other fifteen first arrivals.
#[test]
fn every_bin_first_inserter_stores_identical_coverage() {
    let reference = coverage_of_first_insert(SubpixelBin::Zero, SubpixelBin::Zero);
    for (x_bin, y_bin) in SHUFFLED_BINS {
        assert!(
            coverage_of_first_insert(x_bin, y_bin) == reference,
            "first inserter ({x_bin:?}, {y_bin:?}) stored different coverage"
        );
    }
}
