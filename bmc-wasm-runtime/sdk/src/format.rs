// Copyright (C) 2026  Braiins Systems s.r.o.

//! Formatting utilities for WASM widgets.
//!
//! Mirrors the JS SDK's `sdk.format.*` API from deckfeeder.
//!
//! Preference-aware formatters delegate to host functions that use `formato`
//! and the user's `FormatPreferences` (number format, unit system, temperature).
//! Use the macros [`format_number!`], [`format_speed!`], [`format_temperature!`].

unsafe extern "C" {
    fn host_format_number(value: f64, decimals: u32, out_ptr: *mut u8, out_len: u32) -> i32;
    fn host_format_speed(value: f64, decimals: u32, out_ptr: *mut u8, out_len: u32) -> i32;
    fn host_format_temperature(value: f64, decimals: u32, out_ptr: *mut u8, out_len: u32) -> i32;
}

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
pub fn _host_format_number(value: f64, decimals: u32) -> String {
    let mut buf = [0_u8; 64];
    let len = unsafe { host_format_number(value, decimals, buf.as_mut_ptr(), buf.len() as u32) };
    read_host_buf(&buf, len)
}

/// Format a speed using host-side preferences. Called by [`format_speed!`].
#[doc(hidden)]
pub fn _host_format_speed(value: f64, decimals: u32) -> String {
    let mut buf = [0_u8; 64];
    let len = unsafe { host_format_speed(value, decimals, buf.as_mut_ptr(), buf.len() as u32) };
    read_host_buf(&buf, len)
}

/// Format a temperature using host-side preferences. Called by [`format_temperature!`].
#[doc(hidden)]
pub fn _host_format_temperature(value: f64, decimals: u32) -> String {
    let mut buf = [0_u8; 64];
    let len =
        unsafe { host_format_temperature(value, decimals, buf.as_mut_ptr(), buf.len() as u32) };
    read_host_buf(&buf, len)
}

/// Format a number with user-preferred grouping and decimal separators.
///
/// # Example
/// ```ignore
/// let s = format_number!(27_565.0, 0); // "27 565" (SpaceComma default)
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
/// Zero-pads hours, minutes, and seconds to 2 digits. Days are not padded.
///
/// # Examples
///
/// ```
/// # use bmc_wasm_sdk::format::format_duration;
/// assert_eq!(format_duration(2_598_840, false), "30d 02h 14m");
/// assert_eq!(format_duration(2_598_845, true), "30d 02h 14m 05s");
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
