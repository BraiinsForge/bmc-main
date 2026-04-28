// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared profiling primitives for the `profiling` feature.
//!
//! The widget process and the host crate emit `mesh::profile`-tagged
//! `tracing::info!` lines. A single `RUST_LOG=mesh::profile=info` toggle
//! enables them. Compiled out entirely when the feature is off.

use std::time::Instant;

use crate::proc_mem::{self, MemInfo, RssSample};

/// `tracing` target shared by every profiling log line in this crate.
pub(crate) const TARGET: &str = "mesh::profile";

/// One-shot memory + wall-clock probe.
///
/// Captures `/proc/self/status` and `/proc/meminfo` plus a monotonic
/// timestamp at construction; calling [`MemProbe::snapshot`] reads them
/// again and returns the deltas + an absolute "free memory" view of the
/// system, which is what the OOM diagnosis hinges on.
pub(crate) struct MemProbe {
    rss_before: Option<RssSample>,
    mem_before: Option<MemInfo>,
    started_at: Instant,
}

impl MemProbe {
    pub(crate) fn start() -> Self {
        Self {
            rss_before: proc_mem::read_self_rss(),
            mem_before: proc_mem::read_meminfo(),
            started_at: Instant::now(),
        }
    }

    pub(crate) fn snapshot(&self) -> MemSnapshot {
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
            rss_shmem_delta_kb: proc_mem::delta_kb(
                self.rss_before.map(|s| s.rss_shmem_kb),
                rss_after.map(|s| s.rss_shmem_kb),
            ),
            cma_free_delta_kb: proc_mem::delta_kb(
                self.mem_before.map(|m| m.cma_free_kb),
                mem_after.map(|m| m.cma_free_kb),
            ),
            cma_free_kb: mem_after.map_or(0, |m| m.cma_free_kb),
            mem_free_kb: mem_after.map_or(0, |m| m.mem_free_kb),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MemSnapshot {
    pub elapsed_us: u64,
    pub vmrss_delta_kb: i64,
    pub rss_anon_delta_kb: i64,
    pub rss_shmem_delta_kb: i64,
    pub cma_free_delta_kb: i64,
    pub cma_free_kb: u64,
    pub mem_free_kb: u64,
}
