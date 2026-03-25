// Copyright (C) 2026  Braiins Systems s.r.o.

//! /proc parsing for memory observability under the `profiling` feature.
//!
//! Linux-only readers; non-Linux platforms return `None`. Call sites should
//! treat `None` as "no sample available, skip the log line", not as an error.

use std::fs;

#[derive(Debug, Clone, Copy, Default)]
pub struct RssSample {
    pub vm_size_kb: u64,
    pub vm_rss_kb: u64,
    pub rss_anon_kb: u64,
    pub rss_file_kb: u64,
    pub rss_shmem_kb: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MemInfo {
    pub mem_total_kb: u64,
    pub mem_free_kb: u64,
    pub mem_available_kb: u64,
    pub shmem_kb: u64,
    pub cma_total_kb: u64,
    pub cma_free_kb: u64,
}

#[must_use]
pub fn read_self_rss() -> Option<RssSample> {
    if cfg!(target_os = "linux") {
        Some(parse_status(&fs::read_to_string("/proc/self/status").ok()?))
    } else {
        None
    }
}

pub(crate) fn read_meminfo() -> Option<MemInfo> {
    if cfg!(target_os = "linux") {
        Some(parse_meminfo(&fs::read_to_string("/proc/meminfo").ok()?))
    } else {
        None
    }
}

fn parse_status(raw: &str) -> RssSample {
    let mut s = RssSample::default();
    for line in raw.lines() {
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let Some(kb) = parse_kb(val) else { continue };
        match key.trim() {
            "VmSize" => s.vm_size_kb = kb,
            "VmRSS" => s.vm_rss_kb = kb,
            "RssAnon" => s.rss_anon_kb = kb,
            "RssFile" => s.rss_file_kb = kb,
            "RssShmem" => s.rss_shmem_kb = kb,
            _ => {}
        }
    }
    s
}

fn parse_meminfo(raw: &str) -> MemInfo {
    let mut m = MemInfo::default();
    for line in raw.lines() {
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let Some(kb) = parse_kb(val) else { continue };
        match key.trim() {
            "MemTotal" => m.mem_total_kb = kb,
            "MemFree" => m.mem_free_kb = kb,
            "MemAvailable" => m.mem_available_kb = kb,
            "Shmem" => m.shmem_kb = kb,
            "CmaTotal" => m.cma_total_kb = kb,
            "CmaFree" => m.cma_free_kb = kb,
            _ => {}
        }
    }
    m
}

fn parse_kb(val: &str) -> Option<u64> {
    val.split_whitespace().next()?.parse().ok()
}

/// Signed delta between two unsigned KB samples, saturating on overflow.
///
/// Returns `0` when either side is missing — the caller has lost the
/// before-or-after sample, so a synthetic delta would be misleading.
pub(crate) fn delta_kb(before: Option<u64>, after: Option<u64>) -> i64 {
    let (Some(b), Some(a)) = (before, after) else {
        return 0;
    };
    if a >= b {
        i64::try_from(a - b).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(b - a).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_meminfo, parse_status};

    #[test]
    fn parses_status_memory_fields() {
        let raw = "\
Name:\tbmc-widget-wasm
Pid:\t3246
VmSize:\t  108504 kB
VmRSS:\t   22980 kB
RssAnon:\t  9920 kB
RssFile:\t     8 kB
RssShmem:\t13052 kB
Threads:\t9
";
        let s = parse_status(raw);
        assert_eq!(s.vm_size_kb, 108_504);
        assert_eq!(s.vm_rss_kb, 22_980);
        assert_eq!(s.rss_anon_kb, 9_920);
        assert_eq!(s.rss_file_kb, 8);
        assert_eq!(s.rss_shmem_kb, 13_052);
    }

    #[test]
    fn parse_status_skips_unparseable_lines() {
        let raw = "VmRSS:\tbogus\nVmRSS:\t  100 kB\n";
        let s = parse_status(raw);
        // The second well-formed line wins.
        assert_eq!(s.vm_rss_kb, 100);
    }

    #[test]
    fn parses_meminfo_fields() {
        let raw = "\
MemTotal:        262144 kB
MemFree:          12500 kB
MemAvailable:     16000 kB
Shmem:            70000 kB
CmaTotal:        131072 kB
CmaFree:           4096 kB
";
        let m = parse_meminfo(raw);
        assert_eq!(m.mem_total_kb, 262_144);
        assert_eq!(m.mem_free_kb, 12_500);
        assert_eq!(m.mem_available_kb, 16_000);
        assert_eq!(m.shmem_kb, 70_000);
        assert_eq!(m.cma_total_kb, 131_072);
        assert_eq!(m.cma_free_kb, 4_096);
    }

    #[test]
    fn missing_fields_default_to_zero() {
        let m = parse_meminfo("");
        assert_eq!(m.mem_total_kb, 0);
        assert_eq!(m.cma_free_kb, 0);
    }

    #[test]
    fn delta_kb_handles_growth_and_shrink() {
        use super::delta_kb;
        assert_eq!(delta_kb(Some(100), Some(150)), 50);
        assert_eq!(delta_kb(Some(150), Some(100)), -50);
        assert_eq!(delta_kb(Some(100), Some(100)), 0);
    }

    #[test]
    fn delta_kb_returns_zero_when_either_side_missing() {
        use super::delta_kb;
        assert_eq!(delta_kb(None, Some(100)), 0);
        assert_eq!(delta_kb(Some(100), None), 0);
        assert_eq!(delta_kb(None, None), 0);
    }
}
