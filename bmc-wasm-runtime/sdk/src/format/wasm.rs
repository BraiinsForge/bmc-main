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

use super::{FormatTextProvider, FormatZoneOffsetProvider};
use crate::tz::Tz;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_format_number(value: f64, decimals: u32, out_ptr: *mut u8, out_len: u32) -> i32;
    fn host_format_speed(
        value: f64,
        decimals: u32,
        metric_unit: u32,
        out_ptr: *mut u8,
        out_len: u32,
    ) -> i32;
    fn host_format_temperature(
        value: f64,
        decimals: u32,
        show_unit: u32,
        out_ptr: *mut u8,
        out_len: u32,
    ) -> i32;
    fn host_format_distance(value: f64, decimals: u32, out_ptr: *mut u8, out_len: u32) -> i32;
    fn host_format_date(
        timestamp: i64,
        fmt_ptr: *const u8,
        fmt_len: u32,
        out_ptr: *mut u8,
        out_len: u32,
    ) -> i32;
    /// Resolve `(tz_name, unix_secs)` to the UTC offset in seconds.
    /// Returns `i32::MIN` when the name is not in the deck's supported timezone list.
    fn host_resolve_tz(name_ptr: *const u8, name_len: u32, unix_secs: i64) -> i32;
}

/// Sentinel returned by `host_resolve_tz` for unknown IANA names.
/// Real UTC offsets are bounded to ±14 hours, so this value never collides.
const TZ_UNKNOWN: i32 = i32::MIN;

/// Read a host formatting result from a 64-byte stack buffer.
fn read_host_buf(buf: &[u8; 64], len: i32) -> String {
    if len <= 0 {
        return String::new();
    }
    let len = (len as usize).min(buf.len());
    String::from_utf8_lossy(&buf[..len]).into_owned()
}

pub(super) struct Host;

impl FormatTextProvider for Host {
    fn number(value: f64, decimals: u32) -> String {
        let mut buf = [0_u8; 64];
        let len =
            unsafe { host_format_number(value, decimals, buf.as_mut_ptr(), buf.len() as u32) };
        read_host_buf(&buf, len)
    }

    fn speed(value: f64, decimals: u32, metric_unit: u32) -> String {
        let mut buf = [0_u8; 64];
        let len = unsafe {
            host_format_speed(
                value,
                decimals,
                metric_unit,
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        };
        read_host_buf(&buf, len)
    }

    fn distance(value: f64, decimals: u32) -> String {
        let mut buf = [0_u8; 64];
        let len =
            unsafe { host_format_distance(value, decimals, buf.as_mut_ptr(), buf.len() as u32) };
        read_host_buf(&buf, len)
    }

    fn temperature(value: f64, decimals: u32, show_unit: u32) -> String {
        let mut buf = [0_u8; 64];
        let len = unsafe {
            host_format_temperature(
                value,
                decimals,
                show_unit,
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        };
        read_host_buf(&buf, len)
    }

    fn strftime(timestamp: i64, format: &str) -> String {
        let mut buf = [0_u8; 64];
        let len = unsafe {
            host_format_date(
                timestamp,
                format.as_ptr(),
                format.len() as u32,
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        };
        read_host_buf(&buf, len)
    }
}

impl FormatZoneOffsetProvider for Host {
    fn zone_offset(tz: &Tz, unix_secs: i64) -> Option<i32> {
        let name = tz.iana().as_bytes();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "IANA names ship well under u32 bytes; truncation would be a programmer bug"
        )]
        let offset = unsafe { host_resolve_tz(name.as_ptr(), name.len() as u32, unix_secs) };
        if offset == TZ_UNKNOWN {
            None
        } else {
            Some(offset)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::read_host_buf;

    #[test]
    fn read_host_buf_empty() {
        let buf = [0_u8; 64];
        assert_eq!(read_host_buf(&buf, 0), "");
        assert_eq!(read_host_buf(&buf, -1), "");
    }

    #[test]
    fn read_host_buf_valid() {
        let mut buf = [0_u8; 64];
        buf[..5].copy_from_slice(b"hello");
        assert_eq!(read_host_buf(&buf, 5), "hello");
    }

    #[test]
    fn read_host_buf_clamped() {
        let mut buf = [0_u8; 64];
        buf.fill(b'x');
        assert_eq!(read_host_buf(&buf, 100), "x".repeat(64));
    }
}
