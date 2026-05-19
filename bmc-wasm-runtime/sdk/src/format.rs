// Copyright (C) 2026  Braiins Systems s.r.o.

//! Formatting utilities for WASM widgets.
//!
//! Mirrors the JS SDK's `sdk.format.*` API from deckfeeder.
//!
//! Preference-aware formatters delegate to host functions that use `formato`
//! and the operator-controlled fields of the deck-wide `SystemSnapshot`
//! (`number_format`, `unit_system`, `temperature_unit`) — see [`crate::system`].
//! Use the macros [`format_number!`], [`format_speed!`], [`format_temperature!`].

unsafe extern "C" {
    fn host_format_number(value: f64, decimals: u32, out_ptr: *mut u8, out_len: u32) -> i32;
    fn host_format_speed(value: f64, decimals: u32, out_ptr: *mut u8, out_len: u32) -> i32;
    fn host_format_temperature(value: f64, decimals: u32, out_ptr: *mut u8, out_len: u32) -> i32;
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
    // SAFETY: host writes valid UTF-8 (formatted numbers + unit suffixes)
    String::from_utf8_lossy(&buf[..len]).into_owned()
}

/// Format a number using host-side preferences. Called by [`format_number!`].
#[doc(hidden)]
#[must_use]
pub fn _host_format_number(value: f64, decimals: u32) -> String {
    let mut buf = [0_u8; 64];
    let len = unsafe { host_format_number(value, decimals, buf.as_mut_ptr(), buf.len() as u32) };
    read_host_buf(&buf, len)
}

/// Format a speed using host-side preferences. Called by [`format_speed!`].
#[doc(hidden)]
#[must_use]
pub fn _host_format_speed(value: f64, decimals: u32) -> String {
    let mut buf = [0_u8; 64];
    let len = unsafe { host_format_speed(value, decimals, buf.as_mut_ptr(), buf.len() as u32) };
    read_host_buf(&buf, len)
}

/// Format a temperature using host-side preferences. Called by [`format_temperature!`].
#[doc(hidden)]
#[must_use]
pub fn _host_format_temperature(value: f64, decimals: u32) -> String {
    let mut buf = [0_u8; 64];
    let len =
        unsafe { host_format_temperature(value, decimals, buf.as_mut_ptr(), buf.len() as u32) };
    read_host_buf(&buf, len)
}

/// Format a unix timestamp using a chrono strftime pattern.
///
/// Low-level escape hatch — most callers should use the enum-level
/// helpers [`format_time`] / [`format_date`] that default to the
/// user's system preferences.
///
/// Uses the host's `chrono` library for proper date/time formatting.
/// See <https://docs.rs/chrono/latest/chrono/format/strftime/> for pattern syntax.
///
/// # Example
/// ```ignore
/// let ts = parse_date("2026-03-04T04:19:23+00:00").unwrap();
/// let s = strftime(ts, "%m/%d %H:%M"); // "03/04 04:19"
/// let s = strftime(ts, "%d.%m.%Y %H:%M:%S"); // "04.03.2026 04:19:23"
/// ```
#[must_use]
pub fn strftime(timestamp: i64, format: &str) -> String {
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

// ── System-bound time / date formatters ───────────────────────────────
//
// Each formatter defaults every dimension to the corresponding
// `system::current()` field; `opts` overrides per call.
//
// This is the intended SDK convention for every formatting helper going forward
// (unit-system / temperature / number-format helpers when those land).
//
// Use-cases the override flag must accommodate:
// rendering an event's time in both the user's configured timezone
// and the event's local timezone, or rendering metric and imperial side-by-side.

use crate::system::{self, DateFormat, TimeFormat};
use crate::tz::Tz;

/// Overrides for [`format_time`]. Any `Some`-valued field replaces the
/// corresponding `system::current()` preference for this call only.
#[derive(Clone, Debug, Default)]
pub struct FormatTimeOpts {
    /// Override the system's [`TimeFormat`].
    /// `None` uses `system::current().time_format()`.
    pub format: Option<TimeFormat>,
    /// Override the timezone the moment is rendered in.
    /// `None` uses the host-applied system timezone already baked
    /// into [`crate::host::SystemTime::utc_offset_secs`].
    /// Unknown names fall back to the system timezone (see `host_resolve_tz`).
    pub timezone: Option<Tz>,
    /// Include seconds in the output (e.g. `12:34` vs `12:34:56`).
    pub with_seconds: bool,
}

/// Overrides for [`format_date`]. Any `Some`-valued field replaces the
/// corresponding `system::current()` preference for this call only.
#[derive(Clone, Debug, Default)]
pub struct FormatDateOpts {
    /// Override the system's [`DateFormat`].
    /// `None` uses `system::current().date_format()`.
    pub format: Option<DateFormat>,
    /// Override the timezone the moment is rendered in.
    /// See [`FormatTimeOpts::timezone`].
    pub timezone: Option<Tz>,
}

/// Format the time component of a [`SystemTime`] per the user's
/// preferences, with per-call overrides. AM/PM is **not** included in
/// the output — render it as a separate element when
/// `system::current().time_format()` is [`TimeFormat::Hour12`].
///
/// # Example
/// ```ignore
/// let now = SystemTime::now();
/// let s = format_time(now, FormatTimeOpts::default());                       // "13:45"
/// let s = format_time(now, FormatTimeOpts { with_seconds: true, ..default }); // "13:45:09"
/// ```
#[must_use]
pub fn format_time(now: crate::host::SystemTime, opts: FormatTimeOpts) -> String {
    let format = opts
        .format
        .unwrap_or_else(|| system::current().time_format());
    let pattern = match (format, opts.with_seconds) {
        (TimeFormat::Hour24, false) => "%H:%M",
        (TimeFormat::Hour24, true) => "%H:%M:%S",
        (TimeFormat::Hour12, false) => "%I:%M",
        (TimeFormat::Hour12, true) => "%I:%M:%S",
    };
    strftime(local_unix_secs(&now, opts.timezone.as_ref()), pattern)
}

/// Format the date component of a [`SystemTime`] per the user's preferences,
/// with per-call overrides. Output mirrors the operator's configured locale
/// (e.g. `12.03.2026` vs `03/12/2026`).
///
/// # Example
/// ```ignore
/// let now = SystemTime::now();
/// let s = format_date(now, FormatDateOpts::default()); // "12.03.2026"
/// ```
#[must_use]
pub fn format_date(now: crate::host::SystemTime, opts: FormatDateOpts) -> String {
    let format = opts
        .format
        .unwrap_or_else(|| system::current().date_format());
    let pattern = match format {
        DateFormat::DdMmYyyyDot => "%d.%m.%Y",
        DateFormat::DdMmYyyySlash => "%d/%m/%Y",
        DateFormat::DMYyyySlash => "%-d/%-m/%Y",
        DateFormat::MDYyyySlash => "%-m/%-d/%Y",
        DateFormat::DdMmYyyyDash => "%d-%m-%Y",
        DateFormat::YyyyMDSlash => "%Y/%-m/%-d",
        DateFormat::YyyyMmDdDot => "%Y.%m.%d",
        DateFormat::YyyyMmDdDash => "%Y-%m-%d",
    };
    strftime(local_unix_secs(&now, opts.timezone.as_ref()), pattern)
}

/// Shift `now.unix_secs` (UTC epoch) by the effective UTC offset
/// so a downstream `strftime` (which formats in UTC) prints local
/// wall-clock digits.
///
/// When `tz` is `None`, the pre-applied
/// [`crate::host::SystemTime::utc_offset_secs`] is used — no host round-trip.
/// Otherwise, `host_resolve_tz` looks up the offset for the override at the moment
/// `unix_secs`; unknown names fall back to the system offset.
///
/// Useful for `strftime` patterns the high-level helpers don't cover
/// (e.g. composed weekday + day + month-name strings).
#[must_use]
pub fn local_unix_secs(now: &crate::host::SystemTime, tz: Option<&Tz>) -> i64 {
    let offset_secs = match tz {
        None => now.utc_offset_secs,
        Some(tz) => resolve_tz_offset(tz, now.unix_secs).unwrap_or(now.utc_offset_secs),
    };
    now.unix_secs + i64::from(offset_secs)
}

/// Resolve the UTC offset (in seconds) for an IANA-name timezone at a
/// moment. Returns `None` when the host doesn't recognise the name.
#[must_use]
pub fn resolve_tz_offset(tz: &Tz, unix_secs: i64) -> Option<i32> {
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

/// Format a number with user-preferred grouping and decimal separators.
///
/// # Example
/// ```ignore
/// let s = format_number!(27_565.0, 0); // "27 565" (SpaceGroupCommaDecimal default)
/// ```
#[macro_export]
macro_rules! format_number {
    ($value:expr, $decimals:expr) => {
        $crate::format::_host_format_number($value as f64, $decimals)
    };
}

/// Format a speed value with user-preferred units and number formatting.
///
/// Input is always km/h; the host converts to mph if imperial.
///
/// # Example
/// ```ignore
/// let s = format_speed!(27_565.0, 0); // "27 565 km/h" or "17 126 mph"
/// ```
#[macro_export]
macro_rules! format_speed {
    ($value:expr, $decimals:expr) => {
        $crate::format::_host_format_speed($value as f64, $decimals)
    };
}

/// Format a temperature with user-preferred units and number formatting.
///
/// Input is always °C; the host converts to °F if preferred.
///
/// # Example
/// ```ignore
/// let s = format_temperature!(20.5, 1); // "20,5 °C" or "68,9 °F"
/// ```
#[macro_export]
macro_rules! format_temperature {
    ($value:expr, $decimals:expr) => {
        $crate::format::_host_format_temperature($value as f64, $decimals)
    };
}

/// Format a duration in seconds as a compact countdown string.
///
/// Zero-pads hours, minutes, and seconds to 2 digits.
/// Days are not padded.
///
/// # Examples
///
/// ```
/// # use bmc_wasm_sdk::format::format_duration;
/// assert_eq!(format_duration(2_598_840, false), "30d 01h 54m");
/// assert_eq!(format_duration(2_598_845, true), "30d 01h 54m 05s");
/// assert_eq!(format_duration(3_661, false), "0d 01h 01m");
/// assert_eq!(format_duration(0, false), "T-0");
/// assert_eq!(format_duration(-100, true), "T-0");
/// ```
#[must_use]
pub fn format_duration(remaining_secs: i64, show_seconds: bool) -> String {
    if remaining_secs <= 0 {
        return String::from("T-0");
    }

    let d = remaining_secs / 86_400;
    let h = (remaining_secs % 86_400) / 3_600;
    let m = (remaining_secs % 3_600) / 60;
    let s = remaining_secs % 60;

    let mut out = String::with_capacity(20);
    push_int(&mut out, d);
    out.push_str("d ");
    push_pad2(&mut out, h);
    out.push_str("h ");
    push_pad2(&mut out, m);
    if show_seconds {
        out.push_str("m ");
        push_pad2(&mut out, s);
        out.push('s');
    } else {
        out.push('m');
    }
    out
}

/// Push a non-negative integer as decimal digits.
fn push_int(s: &mut String, n: i64) {
    if n >= 10 {
        push_int(s, n / 10);
    }
    s.push((b'0' + (n % 10) as u8) as char);
}

/// Push a value 0–99 as exactly two decimal digits.
fn push_pad2(s: &mut String, n: i64) {
    if n < 10 {
        s.push('0');
    }
    push_int(s, n);
}

/// Push a non-negative integer left-padded with `0` to `width` digits.
fn push_padded(s: &mut String, n: i64, width: usize) {
    let digits = digit_count(n);
    for _ in digits..width {
        s.push('0');
    }
    push_int(s, n);
}

/// Decimal digit count of a non-negative `i64`, with `0` counted as one digit.
fn digit_count(n: i64) -> usize {
    if n < 10 {
        return 1;
    }
    let mut count = 0;
    let mut v = n;
    while v > 0 {
        count += 1;
        v /= 10;
    }
    count
}

/// Format an `f64` with a fixed number of decimal places, without pulling in `core::fmt::Display for f64`.
/// Provides float display formatting in widgets without the binary-size cost of `format!()`,
/// and without the orphan-rule pain of implementing `uDisplay` on `f64` directly.
///
/// `decimals` is clamped to `0..=9` so the scaled integer always fits in `i64`.
/// The `params` wayland edge already rejects non-finite values, so callers can rely on `value`
/// being a normal finite f64; NaN / ±infinity fall through to whatever the rounded cast produces
/// (well-defined as saturation in stable Rust) and are not specially formatted.
///
/// # Examples
///
/// ```
/// # use bmc_wasm_sdk::format::format_f64_fixed;
/// assert_eq!(format_f64_fixed(2.5, 2), "2.50");
/// assert_eq!(format_f64_fixed(0.0, 2), "0.00");
/// assert_eq!(format_f64_fixed(-0.05, 2), "-0.05");
/// assert_eq!(format_f64_fixed(123.456, 0), "123");
/// assert_eq!(format_f64_fixed(-1.0, 3), "-1.000");
/// ```
#[must_use]
pub fn format_f64_fixed(value: f64, decimals: u32) -> String {
    let decimals = decimals.min(9) as usize;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "decimals clamped to 0..=9 above, so the pow(u32) result fits trivially"
    )]
    let factor: i64 = 10_i64.pow(decimals as u32);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "scale stays within i64 range for finite f64 inputs at decimals <= 9"
    )]
    let scaled = (value * factor as f64).round() as i64;
    let int_part = scaled.abs() / factor;
    let frac_part = scaled.abs() % factor;

    let mut out = String::with_capacity(20);
    // Preserve a leading "-" for negative values that don't round to zero.
    // `value.is_sign_negative()` is true for `-0.0`, so the `scaled != 0` guard prevents "-0.00"
    // output from a stray sign bit on a true zero.
    if value.is_sign_negative() && scaled != 0 {
        out.push('-');
    }
    push_int(&mut out, int_part);
    if decimals > 0 {
        out.push('.');
        push_padded(&mut out, frac_part, decimals);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_zero_and_negative() {
        assert_eq!(format_duration(0, false), "T-0");
        assert_eq!(format_duration(-1, false), "T-0");
        assert_eq!(format_duration(-100, true), "T-0");
    }

    #[test]
    fn duration_seconds_only() {
        assert_eq!(format_duration(59, false), "0d 00h 00m");
        assert_eq!(format_duration(59, true), "0d 00h 00m 59s");
    }

    #[test]
    fn duration_minutes() {
        assert_eq!(format_duration(60, false), "0d 00h 01m");
        assert_eq!(format_duration(3_599, true), "0d 00h 59m 59s");
    }

    #[test]
    fn duration_hours() {
        assert_eq!(format_duration(3_600, false), "0d 01h 00m");
        assert_eq!(format_duration(3_661, false), "0d 01h 01m");
        assert_eq!(format_duration(3_661, true), "0d 01h 01m 01s");
    }

    #[test]
    fn duration_days() {
        assert_eq!(format_duration(86_400, false), "1d 00h 00m");
        assert_eq!(format_duration(2_598_840, false), "30d 01h 54m");
        assert_eq!(format_duration(2_598_840, true), "30d 01h 54m 00s");
    }

    #[test]
    fn duration_large() {
        // 365 days
        assert_eq!(format_duration(365 * 86_400, false), "365d 00h 00m");
    }

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
        // len > 64 should be clamped
        assert_eq!(read_host_buf(&buf, 100), "x".repeat(64));
    }

    #[test]
    fn f64_fixed_positive_with_decimals() {
        assert_eq!(format_f64_fixed(2.5, 2), "2.50");
        assert_eq!(format_f64_fixed(2.55, 2), "2.55");
        assert_eq!(format_f64_fixed(0.05, 2), "0.05");
        assert_eq!(format_f64_fixed(123.456, 2), "123.46");
        assert_eq!(format_f64_fixed(1.0, 3), "1.000");
    }

    #[test]
    fn f64_fixed_zero_and_signed_zero() {
        assert_eq!(format_f64_fixed(0.0, 2), "0.00");
        assert_eq!(format_f64_fixed(-0.0, 2), "0.00");
    }

    #[test]
    fn f64_fixed_negative() {
        assert_eq!(format_f64_fixed(-1.0, 2), "-1.00");
        assert_eq!(format_f64_fixed(-0.05, 2), "-0.05");
        assert_eq!(format_f64_fixed(-123.456, 2), "-123.46");
    }

    #[test]
    fn f64_fixed_zero_decimals() {
        assert_eq!(format_f64_fixed(123.456, 0), "123");
        assert_eq!(format_f64_fixed(-2.5, 0), "-3");
        assert_eq!(format_f64_fixed(0.0, 0), "0");
    }

    #[test]
    fn f64_fixed_clamps_excessive_decimals() {
        assert_eq!(format_f64_fixed(1.0, 10), "1.000000000");
        assert_eq!(format_f64_fixed(1.0, 9), "1.000000000");
    }

    #[test]
    fn f64_fixed_does_not_emit_negative_zero() {
        assert_eq!(format_f64_fixed(-0.001, 2), "0.00");
    }
}
