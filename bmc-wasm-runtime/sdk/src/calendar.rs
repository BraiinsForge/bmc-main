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

//! Calendar host function wrappers — date parsing, RRULE expansion, and
//! timezone conversion.
//!
//! These functions delegate heavy computation to the host to avoid
//! pulling `rrule` + `chrono-tz` (~1.8 MB) into the WASM binary.
//! Off-device they run the host's own implementations instead.
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

pub use bmc_wasm_protocol::time::{CalendarDate, LocalDateTime, month_short, weekday_short};

/// Convert a UTC unix timestamp to wall-clock time in a named IANA timezone.
///
/// Returns `None` if the timezone name is unknown.
#[must_use]
pub fn tz_convert(unix_secs: i64, timezone: &str) -> Option<LocalDateTime> {
    Platform::wall_clock(unix_secs, timezone)
}

/// Parse a date-time into a unix timestamp, in the relaxed RFC 3339 the
/// host's parser accepts: `2026-02-13T10:15:56Z`, the same with a space
/// separator, or with a `+02:00`-style offset.
///
/// Returns `None` unless the text names an instant. A UTC offset is
/// required, so a bare `2026-02-13 10:15:56` is refused rather than
/// guessed at; for a day without a time use [`parse_calendar_date`].
#[must_use]
pub fn parse_datetime(s: &str) -> Option<i64> {
    Platform::parse_datetime(s)
}

/// Parse a `YYYY-MM-DD` date, which names a day rather than an instant.
///
/// Returns `None` if the text is not such a date.
#[must_use]
pub fn parse_calendar_date(s: &str) -> Option<CalendarDate> {
    Platform::parse_calendar_date(s)
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
    max_count: u16,
) -> Vec<i64> {
    Platform::expand_rrule(rrule, dtstart, tzid, window_start, window_end, max_count)
}

trait CalendarWallClockProvider {
    fn wall_clock(unix_secs: i64, timezone: &str) -> Option<LocalDateTime>;
}

trait CalendarParsingProvider {
    fn parse_datetime(s: &str) -> Option<i64>;
    fn parse_calendar_date(s: &str) -> Option<CalendarDate>;
    fn expand_rrule(
        rrule: &str,
        dtstart: &str,
        tzid: Option<&str>,
        window_start: i64,
        window_end: i64,
        max_count: u16,
    ) -> Vec<i64>;
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{CalendarDate, LocalDateTime};

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_expand_rrule(
            input_ptr: *const u8,
            input_len: u32,
            out_ptr: *mut u8,
            out_cap: u32,
        ) -> i32;

        // Not `host_tz_convert`: that name is frozen on the 20-byte layout
        // SDK 0.5 binaries decode.
        fn host_tz_wall_clock(
            unix_secs: i64,
            tz_ptr: *const u8,
            tz_len: u32,
            out_ptr: *mut u8,
        ) -> i32;

        fn host_parse_datetime(str_ptr: *const u8, str_len: u32) -> i64;

        fn host_parse_calendar_date(str_ptr: *const u8, str_len: u32, out_ptr: *mut u8) -> i32;
    }

    pub(super) struct Host;

    impl super::CalendarWallClockProvider for Host {
        fn wall_clock(unix_secs: i64, timezone: &str) -> Option<LocalDateTime> {
            let mut buf = [0_u8; LocalDateTime::WIRE_LEN];
            let rc = unsafe {
                host_tz_wall_clock(
                    unix_secs,
                    timezone.as_ptr(),
                    crate::ffi_u32(timezone.len()),
                    buf.as_mut_ptr(),
                )
            };
            if rc < 0 {
                return None;
            }
            LocalDateTime::from_wire(buf)
        }
    }

    impl super::CalendarParsingProvider for Host {
        fn parse_datetime(s: &str) -> Option<i64> {
            let val = unsafe { host_parse_datetime(s.as_ptr(), crate::ffi_u32(s.len())) };
            if val == i64::MIN { None } else { Some(val) }
        }

        fn parse_calendar_date(s: &str) -> Option<CalendarDate> {
            let mut buf = [0_u8; CalendarDate::WIRE_LEN];
            let rc = unsafe {
                host_parse_calendar_date(s.as_ptr(), crate::ffi_u32(s.len()), buf.as_mut_ptr())
            };
            if rc < 0 {
                return None;
            }
            CalendarDate::from_wire(buf)
        }

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
        /// Output: packed `i64[]` LE (UTC timestamps).
        fn expand_rrule(
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
            let buf_len = crate::ffi_u32(buf.len());
            let needed =
                unsafe { host_expand_rrule(buf.as_ptr(), buf_len, core::ptr::null_mut(), 0) };
            let Some(needed) = u32::try_from(needed).ok().filter(|&needed| needed > 0) else {
                return Vec::new();
            };

            let mut out = vec![0_u8; needed as usize];
            let written =
                unsafe { host_expand_rrule(buf.as_ptr(), buf_len, out.as_mut_ptr(), needed) };
            let Some(written) = u32::try_from(written).ok().filter(|&written| written > 0) else {
                return Vec::new();
            };

            // Decode packed i64[] little-endian
            out.truncate(written as usize);
            out.chunks_exact(8)
                .map(|c| i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
                .collect()
        }
    }

    /// Push a length-prefixed string into a binary buffer (u16 LE length + bytes).
    fn push_str(buf: &mut Vec<u8>, s: &str) {
        let len = u16::try_from(s.len()).unwrap_or(u16::MAX);
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&s.as_bytes()[..usize::from(len)]);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use bmc_wasm_protocol::time::{self, RruleError};

    use super::{CalendarDate, LocalDateTime};

    pub(super) struct Host;

    impl super::CalendarWallClockProvider for Host {
        fn wall_clock(unix_secs: i64, timezone: &str) -> Option<LocalDateTime> {
            time::wall_clock(unix_secs, timezone)
        }
    }

    impl super::CalendarParsingProvider for Host {
        fn parse_datetime(s: &str) -> Option<i64> {
            time::parse_datetime(s)
        }

        fn parse_calendar_date(s: &str) -> Option<CalendarDate> {
            time::parse_calendar_date(s)
        }

        fn expand_rrule(
            rrule: &str,
            dtstart: &str,
            tzid: Option<&str>,
            window_start: i64,
            window_end: i64,
            max_count: u16,
        ) -> Vec<i64> {
            match time::expand_rrule(rrule, dtstart, tzid, window_start, window_end, max_count) {
                Ok(timestamps) => timestamps,
                // The warning the host logs on the device.
                Err(RruleError::InvalidRule(reason)) => {
                    crate::log_warn!("expand_rrule: failed to parse RRULE: {}", reason.as_str());
                    Vec::new()
                }
                Err(RruleError::WindowOutOfRange) => Vec::new(),
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;

#[cfg(test)]
mod tests {
    use super::{LocalDateTime, tz_convert};
    use crate::wasm_test;

    /// Monday the 14th of September 2026, 10:30 UTC.
    const SEPTEMBER_MORNING: i64 = 1_789_381_800;

    #[wasm_test]
    fn tz_convert_reads_the_wall_clock_in_the_zone() {
        assert_eq!(
            tz_convert(SEPTEMBER_MORNING, "Europe/Prague"),
            Some(LocalDateTime {
                year: 2026,
                month: 9,
                day: 14,
                hour: 12,
                minute: 30,
                second: 0,
                weekday: 0,
            })
        );
    }

    #[wasm_test]
    fn tz_convert_refuses_an_unknown_zone() {
        assert_eq!(tz_convert(SEPTEMBER_MORNING, "Not/AZone"), None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod off_device {
        use super::SEPTEMBER_MORNING;
        use crate::calendar::{expand_rrule, parse_calendar_date, parse_datetime};

        #[test]
        fn the_parsers_read_off_the_device() {
            assert_eq!(
                parse_datetime("2026-09-14T10:30:00Z"),
                Some(SEPTEMBER_MORNING)
            );
            assert_eq!(
                parse_calendar_date("2026-09-14").map(|date| date.weekday),
                Some(0)
            );
            assert_eq!(
                expand_rrule(
                    "FREQ=DAILY;COUNT=1",
                    "20260914T103000Z",
                    None,
                    SEPTEMBER_MORNING - 1,
                    SEPTEMBER_MORNING + 1,
                    10,
                ),
                vec![SEPTEMBER_MORNING]
            );
        }
    }
}
