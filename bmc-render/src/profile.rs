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

//! Shared profiling primitives for the `profiling` feature.
//!
//! The widget process and the host crate emit `mesh::profile`-tagged
//! `tracing::info!` lines. A single `RUST_LOG=mesh::profile=info` toggle
//! enables them. Production builds compile them out when the feature is off.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::proc_mem::{self, MemInfo, RssSample};

/// `tracing` target shared by every profiling log line in this crate.
pub const TARGET: &str = "mesh::profile";

static DEALLOCATION_TRACKER_INSTALLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static DEALLOCATED_BYTES: Cell<Option<u64>> = const { Cell::new(None) };
}

/// System allocator wrapper that enables scoped deallocation measurements.
#[derive(Debug)]
pub struct DeallocationTrackingAllocator;

// SAFETY: every allocation operation is forwarded to `System` with the caller's
// original pointer and layout; the wrapper only observes deallocation sizes.
unsafe impl GlobalAlloc for DeallocationTrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        DEALLOCATION_TRACKER_INSTALLED.store(true, Ordering::Relaxed);
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s layout contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        DEALLOCATION_TRACKER_INSTALLED.store(true, Ordering::Relaxed);
        // SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s layout contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCATION_TRACKER_INSTALLED.store(true, Ordering::Relaxed);
        record_deallocation(layout.size());
        // SAFETY: the caller supplies the pointer and layout returned by this
        // wrapper's delegated `System` allocation.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        DEALLOCATION_TRACKER_INSTALLED.store(true, Ordering::Relaxed);
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s pointer, layout,
        // and size contract, which this wrapper forwards unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn record_deallocation(bytes: usize) {
    let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
    let _ = DEALLOCATED_BYTES.try_with(|counter| {
        if let Some(current) = counter.get() {
            counter.set(Some(current.saturating_add(bytes)));
        }
    });
}

/// Measures allocation bytes deallocated on the current thread.
/// Reallocations are not counted, so the result may under-report released bytes.
#[derive(Debug)]
pub struct DeallocationProbe {
    active: bool,
}

impl DeallocationProbe {
    /// Starts a probe when the process installed [`DeallocationTrackingAllocator`].
    #[must_use]
    pub fn start() -> Option<Self> {
        if !DEALLOCATION_TRACKER_INSTALLED.load(Ordering::Relaxed) {
            return None;
        }
        DEALLOCATED_BYTES.with(|counter| {
            assert!(
                counter.get().is_none(),
                "BUG: deallocation probes must not be nested on one thread"
            );
            counter.set(Some(0));
        });
        Some(Self { active: true })
    }

    /// Finishes the probe and returns bytes passed to explicit `dealloc` calls.
    ///
    /// Memory released through `realloc` is not included.
    #[must_use]
    pub fn finish(mut self) -> u64 {
        let bytes = DEALLOCATED_BYTES.with(|counter| {
            counter
                .take()
                .expect("BUG: active deallocation probe must own its counter")
        });
        self.active = false;
        bytes
    }
}

impl Drop for DeallocationProbe {
    fn drop(&mut self) {
        if self.active {
            let _ = DEALLOCATED_BYTES.try_with(|counter| counter.set(None));
        }
    }
}

/// One-shot memory + wall-clock probe.
///
/// Captures `/proc/self/status` and `/proc/meminfo` plus a monotonic
/// timestamp at construction; calling [`MemProbe::snapshot`] reads them
/// again and returns the deltas + an absolute "free memory" view of the
/// system, which is what the OOM diagnosis hinges on.
#[derive(Debug)]
pub struct MemProbe {
    rss_before: Option<RssSample>,
    mem_before: Option<MemInfo>,
    started_at: Instant,
}

impl MemProbe {
    #[must_use]
    pub fn start() -> Self {
        Self {
            rss_before: proc_mem::read_self_rss(),
            mem_before: proc_mem::read_meminfo(),
            started_at: Instant::now(),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> MemSnapshot {
        let rss_after = proc_mem::read_self_rss();
        let mem_after = proc_mem::read_meminfo();
        MemSnapshot {
            elapsed_us: u64::try_from(self.started_at.elapsed().as_micros()).unwrap_or(u64::MAX),
            vmrss_delta_kb: proc_mem::delta_kb(
                self.rss_before.map(|s| s.vm_rss_kb),
                rss_after.map(|s| s.vm_rss_kb),
            ),
            rss_anon_delta_kb: proc_mem::delta_kb(
                self.rss_before.map(|s| s.rss_anon_kb),
                rss_after.map(|s| s.rss_anon_kb),
            ),
            rss_file_delta_kb: proc_mem::delta_kb(
                self.rss_before.map(|s| s.rss_file_kb),
                rss_after.map(|s| s.rss_file_kb),
            ),
            rss_shmem_delta_kb: proc_mem::delta_kb(
                self.rss_before.map(|s| s.rss_shmem_kb),
                rss_after.map(|s| s.rss_shmem_kb),
            ),
            mem_free_delta_kb: proc_mem::delta_kb(
                self.mem_before.map(|m| m.mem_free_kb),
                mem_after.map(|m| m.mem_free_kb),
            ),
            mem_available_delta_kb: proc_mem::delta_kb(
                self.mem_before.map(|m| m.mem_available_kb),
                mem_after.map(|m| m.mem_available_kb),
            ),
            cma_free_delta_kb: proc_mem::delta_kb(
                self.mem_before.map(|m| m.cma_free_kb),
                mem_after.map(|m| m.cma_free_kb),
            ),
            cma_free_kb: mem_after.map_or(0, |m| m.cma_free_kb),
            mem_free_kb: mem_after.map_or(0, |m| m.mem_free_kb),
            mem_available_kb: mem_after.map_or(0, |m| m.mem_available_kb),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MemSnapshot {
    pub elapsed_us: u64,
    pub vmrss_delta_kb: i64,
    pub rss_anon_delta_kb: i64,
    pub rss_file_delta_kb: i64,
    pub rss_shmem_delta_kb: i64,
    pub mem_free_delta_kb: i64,
    pub mem_available_delta_kb: i64,
    pub cma_free_delta_kb: i64,
    pub cma_free_kb: u64,
    pub mem_free_kb: u64,
    pub mem_available_kb: u64,
}

#[cfg(test)]
mod tests {
    use super::{DeallocationProbe, DeallocationTrackingAllocator};

    #[global_allocator]
    static TEST_ALLOCATOR: DeallocationTrackingAllocator = DeallocationTrackingAllocator;

    #[test]
    fn deallocation_probe_counts_freed_allocation_bytes() {
        let allocation = std::hint::black_box(Vec::<u8>::with_capacity(4_096));
        let probe = DeallocationProbe::start()
            .expect("BUG: the test binary must install the tracking allocator");

        drop(allocation);

        assert_eq!(probe.finish(), 4_096);
    }
}
