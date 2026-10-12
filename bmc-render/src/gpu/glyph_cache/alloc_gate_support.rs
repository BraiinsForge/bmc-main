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

//! Harness for `tests/glyph_cache_alloc_gate.rs`.
//!
//! The gate asserts the cache reaches a steady state:
//! once every page and every metadata slot exists,
//! a further identical pass must not grow the heap.
//! That claim can only be made against the migrated renderer,
//! so the harness owns a real one rather than a mock.

/// Re-exported so the gate's own crate can name what the harness returns:
/// `glyph_cache` itself is crate-private.
pub use super::{Counters, MAX_RESIDENT_ENTRIES};

use crate::gpu::FemtoVgRenderer;
use crate::renderer::Renderer as _;
use crate::test_harness::{GlHarness, create_readback_fbo};
use crate::tree::{SpanData, TextStyle};
use bmc_wasm_protocol::Color;
use bmc_wasm_protocol::text::{ArcAnchor, ArcTextFacing};

pub mod counting_allocator {
    //! A `#[global_allocator]` wrapper
    //! that tracks live bytes and their high-water mark.

    use std::alloc::{GlobalAlloc, Layout};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Wraps any allocator and counts what passes through it.
    ///
    /// `Relaxed` throughout: the gate reads the counters from the one
    /// thread that ran the workload, after it finished, so the only
    /// ordering that matters is each counter's own atomicity.
    #[derive(Debug)]
    pub struct CountingAllocator<A> {
        inner: A,
        live: AtomicUsize,
        high_water: AtomicUsize,
    }

    impl<A> CountingAllocator<A> {
        pub const fn new(inner: A) -> Self {
            Self {
                inner,
                live: AtomicUsize::new(0),
                high_water: AtomicUsize::new(0),
            }
        }

        pub fn live_bytes(&self) -> usize {
            self.live.load(Ordering::Relaxed)
        }

        pub fn high_water(&self) -> usize {
            self.high_water.load(Ordering::Relaxed)
        }

        /// Start a new measurement epoch. The high-water mark is monotonic,
        /// so without this it still carries construction and warm-up peaks
        /// and a later comparison against it measures nothing.
        pub fn reset_high_water(&self) {
            self.high_water
                .store(self.live.load(Ordering::Relaxed), Ordering::Relaxed);
        }

        fn record_alloc(&self, bytes: usize) {
            let live = self.live.fetch_add(bytes, Ordering::Relaxed) + bytes;
            self.high_water.fetch_max(live, Ordering::Relaxed);
        }

        fn record_dealloc(&self, bytes: usize) {
            self.live.fetch_sub(bytes, Ordering::Relaxed);
        }
    }

    // SAFETY: every method forwards to `inner` with the caller's own
    // arguments and returns its pointer unchanged; the counters are
    // bookkeeping beside the allocation, never a substitute for it.
    unsafe impl<A: GlobalAlloc> GlobalAlloc for CountingAllocator<A> {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { self.inner.alloc(layout) };
            if !ptr.is_null() {
                self.record_alloc(layout.size());
            }
            ptr
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { self.inner.alloc_zeroed(layout) };
            if !ptr.is_null() {
                self.record_alloc(layout.size());
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            self.record_dealloc(layout.size());
            unsafe { self.inner.dealloc(ptr, layout) };
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let new_ptr = unsafe { self.inner.realloc(ptr, layout, new_size) };
            if !new_ptr.is_null() {
                // Both directions: a shrink that only ever added would let
                // the cache's own churn read as unbounded growth.
                self.record_dealloc(layout.size());
                self.record_alloc(new_size);
            }
            new_ptr
        }
    }

    #[cfg(test)]
    mod tests {
        use super::CountingAllocator;
        use std::alloc::{GlobalAlloc, Layout};
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// Bump allocator over a static arena.
        /// Hands back real, aligned, distinct addresses
        /// without touching the process allocator,
        /// so the counters under test are the only ones moving.
        struct StubAlloc;

        const ARENA_BYTES: usize = 1 << 16;
        static ARENA: [u8; ARENA_BYTES] = [0; ARENA_BYTES];
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        unsafe impl GlobalAlloc for StubAlloc {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                let base = std::ptr::from_ref(&ARENA).cast::<u8>() as usize;
                let offset = NEXT.fetch_add(layout.size() + layout.align(), Ordering::Relaxed);
                let addr = (base + offset).next_multiple_of(layout.align());
                if offset + layout.size() + layout.align() > ARENA_BYTES {
                    return std::ptr::null_mut();
                }
                addr as *mut u8
            }

            // The default forwards to `alloc` and then zeroes, which would
            // write into the read-only arena.
            unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
                unsafe { self.alloc(layout) }
            }

            unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}

            unsafe fn realloc(&self, _ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
                let grown = Layout::from_size_align(new_size, layout.align())
                    .expect("BUG: test layout is not representable");
                unsafe { self.alloc(grown) }
            }
        }

        fn layout(size: usize) -> Layout {
            Layout::from_size_align(size, 8).expect("BUG: test layout is not representable")
        }

        #[test]
        fn alloc_and_dealloc_balance_to_zero() {
            let counting = CountingAllocator::new(StubAlloc);
            let ptr = unsafe { counting.alloc(layout(128)) };
            assert_eq!(counting.live_bytes(), 128);
            unsafe { counting.dealloc(ptr, layout(128)) };
            assert_eq!(counting.live_bytes(), 0);
        }

        #[test]
        fn zeroed_allocation_counts_like_a_plain_one() {
            let counting = CountingAllocator::new(StubAlloc);
            let _ = unsafe { counting.alloc_zeroed(layout(64)) };
            assert_eq!(counting.live_bytes(), 64);
        }

        #[test]
        fn growing_realloc_counts_only_the_difference() {
            let counting = CountingAllocator::new(StubAlloc);
            let ptr = unsafe { counting.alloc(layout(64)) };
            let _ = unsafe { counting.realloc(ptr, layout(64), 256) };
            assert_eq!(counting.live_bytes(), 256);
        }

        #[test]
        fn shrinking_realloc_releases_the_difference() {
            let counting = CountingAllocator::new(StubAlloc);
            let ptr = unsafe { counting.alloc(layout(256)) };
            let _ = unsafe { counting.realloc(ptr, layout(256), 64) };
            assert_eq!(counting.live_bytes(), 64);
        }

        /// The whole point of the epoch reset: a peak that has already been
        /// released must stop counting against the next measurement.
        #[test]
        fn reset_high_water_drops_released_peaks() {
            let counting = CountingAllocator::new(StubAlloc);
            let ptr = unsafe { counting.alloc(layout(1024)) };
            assert_eq!(counting.high_water(), 1024);
            unsafe { counting.dealloc(ptr, layout(1024)) };
            counting.reset_high_water();
            assert_eq!(counting.high_water(), 0);
        }
    }
}

const W: u32 = 320;
const H: u32 = 320;
/// Below the direct-path cutoff,
/// so the migrated phase really populates the cache
/// instead of delegating everything to femtovg.
const CACHED_SIZE_PX: f32 = 92.0;
/// Small enough that the whole corpus fits in a handful of pages, which is
/// what lets the entry cap — rather than page space — be the binding limit.
const CHURN_SIZES_PX: [f32; 5] = [6.0, 5.0, 4.0, 3.0, 2.0];
/// Latin, Greek, Cyrillic, digits and punctuation: everything the embedded
/// faces cover plus the variant letters that force the Noto fallback.
const MULTI_SCRIPT: &str = "Wij. AV 0123 Привет Γειά ϖϑϰϱϵ";

/// One renderer on one GL context, reused across passes.
///
/// Field order is the drop order:
/// the renderer's canvas must release its GL objects
/// before [`GlHarness`] tears the context down.
pub struct AllocGateHarness {
    renderer: FemtoVgRenderer,
    /// Never read — it exists so the context outlives the renderer above.
    _gl: GlHarness,
}

impl AllocGateHarness {
    #[must_use]
    pub fn new() -> Self {
        let gl = GlHarness::new().expect("BUG: headless GL setup failed");
        let (_, fbo_id) = create_readback_fbo(&gl.gl, W, H);
        let renderer = unsafe { FemtoVgRenderer::new(gl.load_fn(), W, H, fbo_id, 0) }
            .expect("BUG: renderer init failed");
        Self { renderer, _gl: gl }
    }

    /// Drive one full workload: the real draw paths, then enough cache
    /// pressure to force page exhaustion and eviction.
    pub fn run_pass(&mut self) {
        self.migrated_path_frame();
        self.corpus_frame();
        self.churn_frames();
    }

    /// Everything the renderer's own text entry points allocate — shaping,
    /// span grouping, quad batching — none of which the corpus seam sees.
    fn migrated_path_frame(&mut self) {
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let size = CACHED_SIZE_PX as u32;
        let white = Color::from_rgb(255, 255, 255);
        let style = TextStyle {
            size,
            color: white,
            ..TextStyle::default()
        };
        let outlined = TextStyle {
            outline_color: Color::from_rgb(10, 10, 10),
            outline_width: 2.0,
            ..style
        };
        let spans = [SpanData {
            text: MULTI_SCRIPT.to_owned(),
            weight: None,
            color: Some(white),
            size: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }];

        self.renderer.begin_frame(W, H, 1.0);
        self.renderer
            .draw_text(MULTI_SCRIPT, 4.0, 4.0, CACHED_SIZE_PX, white);
        self.renderer
            .draw_paragraph(&style, &spans, 4.0, 100.0, f32::from(u16::MAX));
        self.renderer
            .draw_canvas_text(MULTI_SCRIPT, 4.0, 200.0, &outlined);
        self.renderer.draw_curved_text(
            160.0,
            160.0,
            140.0,
            0.0,
            ArcAnchor::Center,
            ArcTextFacing::Outward,
            MULTI_SCRIPT,
            &style,
        );
        self.renderer.flush();
    }

    /// Every glyph of every face at a size where the pages run out, which is
    /// the only way to reach the scratch and drop paths.
    fn corpus_frame(&mut self) {
        self.renderer.begin_frame(W, H, 1.0);
        self.renderer
            .cache_full_glyph_corpus_for_gate(CACHED_SIZE_PX);
        self.renderer.flush();
    }

    /// One frame per size,
    /// because entries made this generation are eviction-protected:
    /// churning inside a single frame
    /// saturates the caps once and then drops everything without ever evicting.
    fn churn_frames(&mut self) {
        for size in CHURN_SIZES_PX {
            self.renderer.begin_frame(W, H, 1.0);
            self.renderer.cache_full_glyph_corpus_for_gate(size);
            self.renderer.flush();
        }
    }

    /// What the workload actually did to the cache. The gate checks these
    /// before it trusts any allocator number: a pass that never evicted
    /// measured a cache that was still filling up.
    #[must_use]
    pub fn counters(&self) -> &Counters {
        self.renderer.glyph_cache_for_gate().counters()
    }

    #[must_use]
    pub fn resident_entries(&self) -> usize {
        self.renderer.glyph_cache_for_gate().resident_len()
    }
}

impl std::fmt::Debug for AllocGateHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AllocGateHarness")
            .field("resident_entries", &self.resident_entries())
            .finish_non_exhaustive()
    }
}

impl Default for AllocGateHarness {
    fn default() -> Self {
        Self::new()
    }
}
