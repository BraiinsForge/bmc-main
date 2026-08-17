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

//! The glyph cache must reach a steady state:
//! once every page, slab slot and map bucket exists,
//! running the same workload again must not grow the heap.
//!
//! `required-features` cannot express a target,
//! so the whole body is cfg'd on Linux too —
//! the harness needs the headless GL context, which is Linux-only.
//!
//! This binary holds exactly **one** `#[test]`. A second one would run
//! concurrently against the same process-wide allocator counters and both would
//! measure each other's allocations. The counting wrapper's own per-operation
//! tests live in `alloc_gate_support::counting_allocator`, in the library
//! target, which is why the gate is run as a bare
//! `cargo test -p bmc-render --features glyph-alloc-gate`: a `--test` filter
//! would silently skip them.

#![cfg(all(feature = "glyph-alloc-gate", target_os = "linux"))]

use bmc_render::gpu::alloc_gate_support::counting_allocator::CountingAllocator;
use bmc_render::gpu::alloc_gate_support::{AllocGateHarness, MAX_RESIDENT_ENTRIES};

#[global_allocator]
static ALLOCATOR: CountingAllocator<std::alloc::System> =
    CountingAllocator::new(std::alloc::System);

/// The spec's metadata ceiling,
/// applied to what a steady-state pass allocates and frees within itself.
/// This is deliberately a whole-trace gate:
/// every shaping, batching and rasterization temporary the workload makes
/// is charged to the cache's budget rather than excused from it.
const TRANSIENT_PEAK_CEILING_BYTES: usize = 3 * 1024 * 1024;

#[test]
fn a_steady_state_pass_retains_nothing_new() {
    let mut harness = AllocGateHarness::new();

    // Pass 1 builds every page and pre-grows femtovg's retained vertex buffers.
    harness.run_pass();

    let counters = *harness.counters();
    assert!(
        counters.evictions > 0,
        "BUG: the workload never evicted, so it measured a cache still filling up: {counters:?}",
    );
    assert!(
        counters.scratch_uses > 0 && counters.glyphs_dropped > 0,
        "BUG: the workload never exhausted the pages, so the fallback paths went unmeasured: \
         {counters:?}",
    );
    assert_eq!(
        harness.resident_entries(),
        MAX_RESIDENT_ENTRIES,
        "BUG: the workload never filled the entry cap",
    );

    // Pass 2 absorbs the one hashbrown table doubling the spec permits, which
    // is a resize of what pass 1 built rather than a second population of it.
    // Everything measured after this has to come out of the same allocations.
    harness.run_pass();

    let baseline_live = ALLOCATOR.live_bytes();
    ALLOCATOR.reset_high_water();

    harness.run_pass();

    assert_eq!(
        ALLOCATOR.live_bytes(),
        baseline_live,
        "BUG: a steady-state pass changed what the process holds, so the workload \
         still grows something every time it runs",
    );
    assert!(
        ALLOCATOR.high_water() <= baseline_live + TRANSIENT_PEAK_CEILING_BYTES,
        "BUG: a steady-state pass peaked at {} B over the {baseline_live} B it started \
         and ended with",
        ALLOCATOR.high_water() - baseline_live,
    );
}
