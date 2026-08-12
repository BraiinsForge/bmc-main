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

//! Formatting and calendar/time helpers for the WASM runtime.

use bmc_wasm_protocol::system::NumberFormat;
use bmc_wasm_protocol::time::{CalendarDate, LocalDateTime};
use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};

pub(super) fn format_number_with_prefs(nf: NumberFormat, value: f64, decimals: u32) -> String {
    bmc_shared_utils::number_format::NumberFormat::from(nf).format_number(value, decimals as usize)
}

/// Expand an RRULE string into concrete UTC timestamps.
///
/// Input is a binary-packed buffer (see `sdk/src/calendar.rs` for wire format):
/// ```text
/// window_start: i64 LE, window_end: i64 LE, max_count: u16 LE,
/// tzid_len: u16 LE, tzid: [u8], dtstart_len: u16 LE, dtstart: [u8],
/// rrule_len: u16 LE, rrule: [u8]
/// ```
pub(super) fn expand_rrule_impl(input: &[u8]) -> Vec<i64> {
    use rrule::RRuleSet;
    use std::fmt::Write;

    if input.len() < 18 {
        tracing::warn!("expand_rrule: input too short ({} bytes)", input.len());
        return Vec::new();
    }

    let window_start = i64::from_le_bytes(input[0..8].try_into().unwrap_or_default());
    let window_end = i64::from_le_bytes(input[8..16].try_into().unwrap_or_default());
    let max_count = u16::from_le_bytes(input[16..18].try_into().unwrap_or_default());

    let mut pos = 18;
    let read_str = |pos: &mut usize| -> Option<&str> {
        if *pos + 2 > input.len() {
            return None;
        }
        let len = u16::from_le_bytes(input[*pos..*pos + 2].try_into().ok()?) as usize;
        *pos += 2;
        if *pos + len > input.len() {
            return None;
        }
        let s = core::str::from_utf8(&input[*pos..*pos + len]).ok()?;
        *pos += len;
        Some(s)
    };

    let Some(tzid_raw) = read_str(&mut pos) else {
        tracing::warn!("expand_rrule: failed to read tzid");
        return Vec::new();
    };
    let tzid = if tzid_raw.is_empty() {
        None
    } else {
        Some(tzid_raw)
    };
    let Some(dtstart_str) = read_str(&mut pos) else {
        tracing::warn!("expand_rrule: failed to read dtstart");
        return Vec::new();
    };
    let Some(rrule_str) = read_str(&mut pos) else {
        tracing::warn!("expand_rrule: failed to read rrule");
        return Vec::new();
    };

    let mut rrule_input = String::with_capacity(256);

    if let Some(tz) = tzid {
        let _ = writeln!(rrule_input, "DTSTART;TZID={tz}:{dtstart_str}");
    } else if dtstart_str.ends_with('Z') {
        let _ = writeln!(rrule_input, "DTSTART:{dtstart_str}");
    } else {
        let _ = writeln!(rrule_input, "DTSTART:{dtstart_str}Z");
    }

    let _ = write!(rrule_input, "RRULE:{rrule_str}");

    let rrule_set: RRuleSet = match rrule_input.parse() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("expand_rrule: failed to parse RRULE: {e}");
            return Vec::new();
        }
    };

    let Some(after) = DateTime::from_timestamp(window_start, 0) else {
        return Vec::new();
    };
    let Some(before) = DateTime::from_timestamp(window_end, 0) else {
        return Vec::new();
    };
    let after = after.with_timezone(&rrule::Tz::UTC);
    let before = before.with_timezone(&rrule::Tz::UTC);

    let result = rrule_set.after(after).before(before).all(max_count);

    result.dates.into_iter().map(|dt| dt.timestamp()).collect()
}

/// Convert a UTC unix timestamp to wall-clock time in a named IANA
/// timezone, or `None` where the zone is unknown.
pub(super) fn tz_convert_impl(unix_secs: i64, tz_name: &str) -> Option<LocalDateTime> {
    use chrono::TimeZone;

    let dt_utc = DateTime::from_timestamp(unix_secs, 0)?;

    let (year, month, day, hour, minute, second, weekday) = if tz_name == "Local" {
        let local = dt_utc.with_timezone(&Local);
        (
            local.year(),
            local.month(),
            local.day(),
            local.hour(),
            local.minute(),
            local.second(),
            local.weekday().num_days_from_monday(),
        )
    } else {
        let tz: chrono_tz::Tz = tz_name.parse().ok()?;
        let local = tz.from_utc_datetime(&dt_utc.naive_utc());
        (
            local.year(),
            local.month(),
            local.day(),
            local.hour(),
            local.minute(),
            local.second(),
            local.weekday().num_days_from_monday(),
        )
    };

    Some(LocalDateTime {
        year: u16::try_from(year).ok()?,
        month: u8::try_from(month).ok()?,
        day: u8::try_from(day).ok()?,
        hour: u8::try_from(hour).ok()?,
        minute: u8::try_from(minute).ok()?,
        second: u8::try_from(second).ok()?,
        weekday: u8::try_from(weekday).ok()?,
    })
}

/// The 20-byte answer widgets built against SDK 0.5 decode.
///
/// Frozen: those binaries read the fields at `[12..19]` without validating
/// anything, so the layout has to outlive them.
pub(super) fn tz_convert_legacy_wire(unix_secs: i64, tz_name: &str) -> Option<[u8; 20]> {
    let local = tz_convert_impl(unix_secs, tz_name)?;
    let utc_offset = utc_offset_secs(unix_secs, tz_name)?;
    let mut buf = [0_u8; 20];
    buf[0..8].copy_from_slice(&unix_secs.to_le_bytes());
    buf[8..12].copy_from_slice(&utc_offset.to_le_bytes());
    buf[12..14].copy_from_slice(&local.year.to_le_bytes());
    buf[14] = local.month;
    buf[15] = local.day;
    buf[16] = local.hour;
    buf[17] = local.minute;
    buf[18] = local.second;
    buf[19] = local.weekday;
    Some(buf)
}

/// The zone's UTC offset at the given instant, in seconds.
fn utc_offset_secs(unix_secs: i64, tz_name: &str) -> Option<i32> {
    use chrono::Offset;
    use chrono::TimeZone;

    let dt_utc = DateTime::from_timestamp(unix_secs, 0)?;
    Some(if tz_name == "Local" {
        dt_utc
            .with_timezone(&Local)
            .offset()
            .fix()
            .local_minus_utc()
    } else {
        let tz: chrono_tz::Tz = tz_name.parse().ok()?;
        tz.from_utc_datetime(&dt_utc.naive_utc())
            .offset()
            .fix()
            .local_minus_utc()
    })
}

/// Read a `YYYY-MM-DD` date, which names a day rather than an instant.
///
/// Kept apart from the timestamp parser rather than folded into it as a
/// fallback: turning a date into an instant would mean inventing a time,
/// and converting that into a zone behind UTC lands on the day before.
pub(super) fn parse_calendar_date_impl(s: &str) -> Option<CalendarDate> {
    let date = s.parse::<NaiveDate>().ok()?;
    Some(CalendarDate {
        year: u16::try_from(date.year()).ok()?,
        month: u8::try_from(date.month()).ok()?,
        day: u8::try_from(date.day()).ok()?,
        weekday: u8::try_from(date.weekday().num_days_from_monday()).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use bmc_wasm_protocol::system::NumberFormat;

    use super::{format_number_with_prefs, parse_calendar_date_impl, tz_convert_impl};

    /// Friday the 21st of August 2026, weekday 4 counting Monday as 0.
    #[test]
    fn a_calendar_date_carries_the_weekday_it_was_resolved_with() {
        let date = parse_calendar_date_impl("2026-08-21").expect("BUG: a well-formed date");
        assert_eq!(
            (date.year, date.month, date.day, date.weekday),
            (2026, 8, 21, 4)
        );
    }

    /// The two parsers divide the shapes between them: whatever names an
    /// instant belongs to the other one, and neither guesses.
    #[test]
    fn a_calendar_date_refuses_anything_carrying_a_time() {
        for text in [
            "2026-08-21T10:30:00Z",
            "2026-08-21 10:30:00",
            "",
            "not a date at all",
        ] {
            assert!(
                parse_calendar_date_impl(text).is_none(),
                "`{text}` read as a date",
            );
        }
    }

    #[test]
    fn format_number_with_prefs_uses_requested_separators() {
        assert_eq!(
            format_number_with_prefs(NumberFormat::CommaGroupDotDecimal, 12_345.5, 1),
            "12,345.5"
        );
        assert_eq!(
            format_number_with_prefs(NumberFormat::DotGroupCommaDecimal, 12_345.5, 1),
            "12.345,5"
        );
    }

    #[test]
    fn tz_convert_impl_rejects_unknown_timezones() {
        assert!(tz_convert_impl(0, "Not/AZone").is_none());
    }
}
