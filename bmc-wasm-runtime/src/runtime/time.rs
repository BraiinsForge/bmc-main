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
use bmc_wasm_protocol::time::{RruleError, expand_rrule, utc_offset_secs, wall_clock};

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

    let Some(tzid) = read_str(&mut pos) else {
        tracing::warn!("expand_rrule: failed to read tzid");
        return Vec::new();
    };
    let Some(dtstart_str) = read_str(&mut pos) else {
        tracing::warn!("expand_rrule: failed to read dtstart");
        return Vec::new();
    };
    let Some(rrule_str) = read_str(&mut pos) else {
        tracing::warn!("expand_rrule: failed to read rrule");
        return Vec::new();
    };

    match expand_rrule(
        rrule_str,
        dtstart_str,
        (!tzid.is_empty()).then_some(tzid),
        window_start,
        window_end,
        max_count,
    ) {
        Ok(timestamps) => timestamps,
        Err(RruleError::InvalidRule(e)) => {
            tracing::warn!("expand_rrule: failed to parse RRULE: {e}");
            Vec::new()
        }
        Err(RruleError::WindowOutOfRange) => Vec::new(),
    }
}

/// The 20-byte answer widgets built against SDK 0.5 decode.
///
/// Frozen: those binaries read the fields at `[12..19]` without validating
/// anything, so the layout has to outlive them.
pub(super) fn tz_convert_legacy_wire(unix_secs: i64, tz_name: &str) -> Option<[u8; 20]> {
    let local = wall_clock(unix_secs, tz_name)?;
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

#[cfg(test)]
mod tests {
    use bmc_wasm_protocol::system::NumberFormat;

    use super::{expand_rrule_impl, format_number_with_prefs};

    /// The SDK writes a missing zone as empty text.
    #[test]
    fn an_empty_wire_zone_expands_as_utc() {
        const SEPTEMBER_MORNING: i64 = 1_789_381_800;
        const DAY: i64 = 86_400;
        let mut wire = Vec::new();
        wire.extend_from_slice(&(SEPTEMBER_MORNING - 1).to_le_bytes());
        wire.extend_from_slice(&(SEPTEMBER_MORNING + 10 * DAY).to_le_bytes());
        wire.extend_from_slice(&10_u16.to_le_bytes());
        for field in ["", "20260914T103000", "FREQ=DAILY;COUNT=2"] {
            let len = u16::try_from(field.len()).expect("BUG: a test field fits u16");
            wire.extend_from_slice(&len.to_le_bytes());
            wire.extend_from_slice(field.as_bytes());
        }

        assert_eq!(
            expand_rrule_impl(&wire),
            [SEPTEMBER_MORNING, SEPTEMBER_MORNING + DAY]
        );
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
}
