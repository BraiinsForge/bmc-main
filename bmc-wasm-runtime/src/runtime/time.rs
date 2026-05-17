// Copyright (C) 2026  Braiins Systems s.r.o.

//! Formatting and calendar/time helpers for the WASM runtime.

#![expect(clippy::cast_possible_truncation)]

use bmc_wasm_protocol::system::NumberFormat;
use chrono::{DateTime, Datelike, Local, Timelike};
use formato::{FormatOptions, Formato};

/// Format a number using the given number format preference and `formato` crate.
pub(super) fn format_number_with_prefs(nf: NumberFormat, value: f64, decimals: u32) -> String {
    let (group_sep, decimal_sep) = match nf {
        NumberFormat::SpaceGroupCommaDecimal => ("\u{00a0}", ","),
        NumberFormat::CommaGroupDotDecimal => (",", "."),
        NumberFormat::DotGroupCommaDecimal => (".", ","),
        NumberFormat::SpaceGroupDotDecimal => ("\u{00a0}", "."),
    };

    let options = FormatOptions::new()
        .with_thousands(group_sep)
        .with_decimal(decimal_sep);

    let pattern = if decimals == 0 {
        "#,##0".to_owned()
    } else {
        format!("#,##0.{}", "0".repeat(decimals as usize))
    };

    value.formato_ops(&pattern, &options)
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

/// Convert a UTC unix timestamp to wall-clock time in a named IANA timezone.
/// Returns the 20-byte SystemTime wire format, or `None` on error.
pub(super) fn tz_convert_impl(unix_secs: i64, tz_name: &str) -> Option<[u8; 20]> {
    use chrono::Offset;
    use chrono::TimeZone;

    let dt_utc = DateTime::from_timestamp(unix_secs, 0)?;

    let (year, month, day, hour, minute, second, weekday, utc_offset) = if tz_name == "Local" {
        let local = dt_utc.with_timezone(&Local);
        (
            local.year(),
            local.month(),
            local.day(),
            local.hour(),
            local.minute(),
            local.second(),
            local.weekday().num_days_from_monday(),
            local.offset().fix().local_minus_utc(),
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
            local.offset().fix().local_minus_utc(),
        )
    };

    let mut buf = [0_u8; 20];
    buf[0..8].copy_from_slice(&unix_secs.to_le_bytes());
    buf[8..12].copy_from_slice(&utc_offset.to_le_bytes());
    #[expect(clippy::cast_sign_loss)]
    let y = year as u16;
    buf[12..14].copy_from_slice(&y.to_le_bytes());
    buf[14] = month as u8;
    buf[15] = day as u8;
    buf[16] = hour as u8;
    buf[17] = minute as u8;
    buf[18] = second as u8;
    buf[19] = weekday as u8;

    Some(buf)
}

#[cfg(test)]
mod tests {
    use bmc_wasm_protocol::system::NumberFormat;

    use super::{format_number_with_prefs, tz_convert_impl};

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
