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
//!     200_u16,
//! );
//!
//! // Convert UTC timestamp to wall-clock in a timezone
//! if let Some(t) = calendar::tz_convert(1_741_564_800, "America/New_York") {
//!     log_info!("{}:{:02}", t.hour, t.minute);
//! }
//! ```

use crate::host::LocalDateTime;

#[link(wasm_import_module = "env")]
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
///
/// ## Wire format (input)
///
/// ```text
/// window_start: i64 LE    [0..8]
/// window_end:   i64 LE    [8..16]
/// max_count:    u16 LE    [16..18]
/// tzid_len:     u16 LE    [18..20]   (0 = no TZID)
/// tzid:         [u8]      [20..20+tzid_len]
/// dtstart_len:  u16 LE    [..]
/// dtstart:      [u8]
/// rrule_len:    u16 LE    [..]
/// rrule:        [u8]
/// ```
///
/// Output: packed `i64[]` LE (UTC timestamps), unchanged from before.
#[must_use]
pub fn expand_rrule(
    rrule: &str,
    dtstart: &str,
    tzid: Option<&str>,
    window_start: i64,
    window_end: i64,
    max_count: u16,
) -> Vec<i64> {
    let tzid = tzid.unwrap_or("");
    let cap = 18 + 2 + tzid.len() + 2 + dtstart.len() + 2 + rrule.len();
    let mut buf = Vec::with_capacity(cap);

    buf.extend_from_slice(&window_start.to_le_bytes());
    buf.extend_from_slice(&window_end.to_le_bytes());
    buf.extend_from_slice(&max_count.to_le_bytes());
    push_str(&mut buf, tzid);
    push_str(&mut buf, dtstart);
    push_str(&mut buf, rrule);

    // Two-call pattern: first get required output size
    let needed =
        unsafe { host_expand_rrule(buf.as_ptr(), buf.len() as u32, core::ptr::null_mut(), 0) };
    if needed <= 0 {
        return Vec::new();
    }

    // Allocate and fill
    let mut out = vec![0u8; needed as usize];
    let written = unsafe {
        host_expand_rrule(
            buf.as_ptr(),
            buf.len() as u32,
            out.as_mut_ptr(),
            needed as u32,
        )
    };
    if written <= 0 {
        return Vec::new();
    }

    // Decode packed i64[] little-endian
    out.truncate(written as usize);
    out.chunks_exact(8)
        .map(|c| i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
        .collect()
}

/// Push a length-prefixed string into a binary buffer (u16 LE length + bytes).
fn push_str(buf: &mut Vec<u8>, s: &str) {
    let len = s.len().min(u16::MAX as usize) as u16;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&s.as_bytes()[..len as usize]);
}

/// Convert a UTC unix timestamp to wall-clock time in a named IANA timezone.
///
/// Returns `None` if the timezone name is unknown.
#[must_use]
pub fn tz_convert(unix_secs: i64, timezone: &str) -> Option<LocalDateTime> {
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
    // Host's tz_convert still emits the legacy 20-byte layout.
    // Pick out year/month/day/hour/minute/second/weekday;
    // drop the redundant unix_secs (the caller passed it in)
    // and utc_offset_secs.
    Some(LocalDateTime {
        year: u16::from_le_bytes([buf[12], buf[13]]),
        month: buf[14],
        day: buf[15],
        hour: buf[16],
        minute: buf[17],
        second: buf[18],
        weekday: buf[19],
    })
}
