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

/// Reaches the diagnostic's callsite from a thread that has a subscriber
/// installed, then rebuilds the interest cache.
///
/// tracing caches a callsite's interest process-wide on first reach,
/// so a sibling test reaching it first from a subscriber-less thread
/// would pin it disabled for the whole test binary.
fn arm_pressure_callsite() {
    let mut cache = GlyphCache::<usize>::new();
    cache.counters.glyphs_dropped += 1;
    cache.end_frame();
    tracing::callsite::rebuild_interest_cache();
}

#[test]
fn repeated_failing_frames_log_once_per_interval() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();

    let ((), records) = counting_warns(|warns| {
        arm_pressure_callsite();
        warns.reset();
        for frame in 0..3 {
            backend.fail_next_upload = Some(PageFaultKind::Transient);
            assert_eq!(
                cache.get_or_insert(&mut backend, test_cache_key(frame), |_| Some(solid_glyph(
                    3, 5
                ))),
                GlyphLookup::Missing
            );
            cache.end_frame();
        }
    });

    assert_eq!(cache.counters().upload_transient_failures, 3);
    assert_eq!(records, 1);
}

#[test]
fn one_record_carries_every_tally_of_a_churning_frame() {
    let mut backend = test_support::MockBackend::default();
    let mut cache = GlyphCache::new();

    let ((), records) = counting_warns(|warns| {
        arm_pressure_callsite();
        warns.reset();
        fill_cold_pages(&mut cache, &mut backend);
        assert_eq!(warns.count(), 0, "quiet frames must stay silent");

        // A fresh entry on every page stamps the atlas with this frame,
        // so what follows can only go to scratch.
        for index in 0..MAX_NORMAL_PAGES {
            let _ = resident(cache.get_or_insert(
                &mut backend,
                test_cache_key(MAX_NORMAL_PAGES + index),
                |_| Some(page_filling_glyph()),
            ));
        }
        let _ = resident(cache.get_or_insert(
            &mut backend,
            test_cache_key(2 * MAX_NORMAL_PAGES),
            |_| Some(page_filling_glyph()),
        ));
        let _ = cache.get_or_insert(
            &mut backend,
            test_cache_key(2 * MAX_NORMAL_PAGES + 1),
            |_| Some(page_filling_glyph()),
        );
        let _ = cache.get_or_insert(
            &mut backend,
            test_cache_key(2 * MAX_NORMAL_PAGES + 2),
            |_| Some(solid_glyph(600, 20)),
        );
        backend.fail_next_upload = Some(PageFaultKind::Transient);
        let _ = cache.get_or_insert(
            &mut backend,
            test_cache_key(2 * MAX_NORMAL_PAGES + 3),
            |_| Some(solid_glyph(3, 5)),
        );
        cache.end_frame();
    });

    assert_eq!(
        cache.counters().evictions,
        u64::try_from(MAX_NORMAL_PAGES).expect("BUG: page cap outside the counter range")
    );
    assert_eq!(cache.counters().scratch_uses, 1);
    assert_eq!(cache.counters().glyphs_dropped, 1);
    assert_eq!(cache.counters().glyphs_oversized, 1);
    assert_eq!(cache.counters().upload_transient_failures, 1);
    assert_eq!(records, 1);
}
