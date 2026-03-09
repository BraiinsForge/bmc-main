// Copyright (C) 2026  Braiins Systems s.r.o.

//! Calendar host function wrappers — RRULE expansion and timezone conversion.
//!
//! These functions delegate heavy computation to the host to avoid pulling
//! `rrule` + `chrono-tz` (~1.8 MB) into the WASM binary.
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::calendar;
//!
//! // Expand a weekly recurring event
//! let occurrences = calendar::expand_rrule(
//!     "FREQ=WEEKLY;BYDAY=MO,WE,FR",
//!     "20260310T090000",
//!     Some("Europe/Prague"),
//!     1_741_564_800,
//!     1_742_774_400,
//!     200,
//! );
//!
//! // Convert UTC timestamp to wall-clock in a timezone
//! if let Some(t) = calendar::tz_convert(1_741_564_800, "America/New_York") {
//!     log_info!("{}:{:02}", t.hour, t.minute);
//! }
//! ```

use crate::host::SystemTime;

unsafe extern "C" {
    fn host_expand_rrule(
        input_ptr: *const u8,
        input_len: u32,
        out_ptr: *mut u8,
        out_cap: u32,
    ) -> i32;

    fn host_tz_convert(unix_secs: i64, tz_ptr: *const u8, tz_len: u32, out_ptr: *mut u8) -> i32;
}

/// Expand an RRULE into concrete occurrence timestamps (UTC).
///
/// Returns a vector of unix timestamps for each occurrence within the
/// `[window_start, window_end]` range, capped at `max_count`.
///
/// Returns an empty vec if the RRULE is invalid or no occurrences fall
/// within the window.
#[must_use]
pub fn expand_rrule(
    rrule: &str,
    dtstart: &str,
    tzid: Option<&str>,
    window_start: i64,
    window_end: i64,
    max_count: u32,
) -> Vec<i64> {
    // Build JSON input manually to avoid pulling serde into WASM
    let mut input = String::with_capacity(256);
    input.push_str("{\"rrule\":\"");
    push_json_escaped(&mut input, rrule);
    input.push_str("\",\"dtstart\":\"");
    push_json_escaped(&mut input, dtstart);
    input.push('"');
    if let Some(tz) = tzid {
        input.push_str(",\"tzid\":\"");
        push_json_escaped(&mut input, tz);
        input.push('"');
    }
    input.push_str(",\"window_start\":");
    push_i64(&mut input, window_start);
    input.push_str(",\"window_end\":");
    push_i64(&mut input, window_end);
    input.push_str(",\"max_count\":");
    push_u32(&mut input, max_count);
    input.push('}');

    // Two-call pattern: first get required size
    let needed =
        unsafe { host_expand_rrule(input.as_ptr(), input.len() as u32, core::ptr::null_mut(), 0) };
    if needed <= 0 {
        return Vec::new();
    }

    // Allocate and fill
    let mut buf = vec![0u8; needed as usize];
    let written = unsafe {
        host_expand_rrule(
            input.as_ptr(),
            input.len() as u32,
            buf.as_mut_ptr(),
            needed as u32,
        )
    };
    if written <= 0 {
        return Vec::new();
    }

    // Decode packed i64[] little-endian
    let count = written as usize / 8;
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * 8;
        if offset + 8 <= buf.len() {
            let ts = i64::from_le_bytes([
                buf[offset],
                buf[offset + 1],
                buf[offset + 2],
                buf[offset + 3],
                buf[offset + 4],
                buf[offset + 5],
                buf[offset + 6],
                buf[offset + 7],
            ]);
            result.push(ts);
        }
    }
    result
}

/// Convert a UTC unix timestamp to wall-clock time in a named IANA timezone.
///
/// Returns `None` if the timezone name is unknown.
#[must_use]
pub fn tz_convert(unix_secs: i64, timezone: &str) -> Option<SystemTime> {
    let mut buf = [0u8; 20];
    let rc = unsafe {
        host_tz_convert(
            unix_secs,
            timezone.as_ptr(),
            timezone.len() as u32,
            buf.as_mut_ptr(),
        )
    };
    if rc < 0 {
        return None;
    }
    Some(SystemTime {
        unix_secs: i64::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
        ]),
        utc_offset_secs: i32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
        year: u16::from_le_bytes([buf[12], buf[13]]),
        month: buf[14],
        day: buf[15],
        hour: buf[16],
        minute: buf[17],
        second: buf[18],
        weekday: buf[19],
    })
}

// ── JSON helpers (no serde needed) ──────────────────────────────────

fn push_json_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
}

fn push_i64(out: &mut String, n: i64) {
    use core::fmt::Write;
    let _ = write!(out, "{n}");
}

fn push_u32(out: &mut String, n: u32) {
    use core::fmt::Write;
    let _ = write!(out, "{n}");
}
