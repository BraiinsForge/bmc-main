// Copyright (C) 2026  Braiins Systems s.r.o.

//! Formatting utilities for WASM widgets.
//!
//! Mirrors the JS SDK's `sdk.format.*` API from deckfeeder.
//! Preference-aware formatters (number, date, time, temperature) will be
//! added as the preferences delivery mechanism lands (Phase 3b).

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
