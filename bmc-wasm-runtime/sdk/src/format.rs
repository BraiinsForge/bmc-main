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

//! Formatting utilities for WASM widgets.
//!
//! Mirrors the JS SDK's `sdk.format.*` API from deckfeeder.
//!
//! Preference-aware formatters keyed on the deck-wide `SystemSnapshot`
//! (`number_format`, `unit_system`, `temperature_unit`). wasm goes through the
//! host; native through the same `bmc_shared_utils` core, matching the device.
//!
//! Use the macros `format_number!`, `format_speed!`, `format_temperature!`.

#![expect(
    clippy::used_underscore_items,
    reason = "the module's own host-boundary `_host_format_*` helpers call each other"
)]

// The host runs the same core, so native output matches the device.
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use bmc_shared_utils::number_format::NumberFormat;
    use bmc_shared_utils::temperature::TemperatureUnit;
    use bmc_shared_utils::unit_system::UnitSystem;

    pub(super) fn number_format() -> NumberFormat {
        crate::system::current()
            .number_format()
            .map(NumberFormat::from)
            .unwrap_or_default()
    }

    pub(super) fn temperature_unit() -> TemperatureUnit {
        crate::system::current()
            .temperature_unit()
            .map(TemperatureUnit::from)
            .unwrap_or_default()
    }

    pub(super) fn unit_system() -> UnitSystem {
        crate::system::current()
            .unit_system()
            .map(UnitSystem::from)
            .unwrap_or_default()
    }
}

#[cfg(target_arch = "wasm32")]
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
#[cfg(target_arch = "wasm32")]
const TZ_UNKNOWN: i32 = i32::MIN;

/// Read a host formatting result from a 64-byte stack buffer.
#[cfg(target_arch = "wasm32")]
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
    #[cfg(target_arch = "wasm32")]
    {
        let mut buf = [0_u8; 64];
        let len =
            unsafe { host_format_number(value, decimals, buf.as_mut_ptr(), buf.len() as u32) };
        read_host_buf(&buf, len)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        native::number_format().format_number(value, decimals as usize)
    }
}

/// SI value and unit split, D3 `.3s`-style: `("13.2", "kW")`. `unit_prefix`
/// picks the scale; the mantissa renders through the host at `sig_figs`
/// significant digits. Split so value and unit can be sized apart.
#[doc(hidden)]
#[must_use]
pub fn _host_format_si_parts(value: f64, sig_figs: u32, base_unit: &str) -> (String, String) {
    let (mantissa, prefix) = match unit_prefix::NumberPrefix::decimal(value) {
        unit_prefix::NumberPrefix::Standalone(m) => (m, ""),
        unit_prefix::NumberPrefix::Prefixed(p, m) => (m, p.symbol()),
    };
    let value_str = _host_format_number(mantissa, si_decimals(mantissa, sig_figs));
    let mut unit = String::with_capacity(prefix.len() + base_unit.len());
    unit.push_str(prefix);
    unit.push_str(base_unit);
    (value_str, unit)
}

/// [`_host_format_si_parts`] joined into `"value unit"`, e.g. `"13.2 kW"`.
#[doc(hidden)]
#[must_use]
pub fn _host_format_si(value: f64, sig_figs: u32, base_unit: &str) -> String {
    let (mut s, unit) = _host_format_si_parts(value, sig_figs, base_unit);
    s.push(' ');
    s.push_str(&unit);
    s
}

/// Decimals to show `sig_figs` significant digits of a mantissa in `[1, 1000)`.
/// Integer digits counted by range — the no-std wasm target lacks `log10`.
fn si_decimals(mantissa: f64, sig_figs: u32) -> u32 {
    let m = mantissa.abs();
    let int_digits = if m < 10.0 {
        1
    } else if m < 100.0 {
        2
    } else {
        3
    };
    sig_figs.saturating_sub(int_digits)
}

/// Format a speed using host-side preferences. Called by [`format_speed!`].
#[doc(hidden)]
#[must_use]
pub fn _host_format_speed(value: f64, decimals: u32, metric_unit: u32) -> String {
    #[cfg(target_arch = "wasm32")]
    {
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
    #[cfg(not(target_arch = "wasm32"))]
    {
        use bmc_shared_utils::unit_system::MetricSpeedUnit;
        let metric_unit = if metric_unit == 1 {
            MetricSpeedUnit::Ms
        } else {
            MetricSpeedUnit::KmH
        };
        native::unit_system().format_speed(
            native::number_format(),
            value,
            decimals as usize,
            metric_unit,
        )
    }
}

/// Format a distance using host-side preferences.
/// Called by [`format_distance!`].
#[doc(hidden)]
#[must_use]
pub fn _host_format_distance(value: f64, decimals: u32) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let mut buf = [0_u8; 64];
        let len =
            unsafe { host_format_distance(value, decimals, buf.as_mut_ptr(), buf.len() as u32) };
        read_host_buf(&buf, len)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        native::unit_system().format_distance(native::number_format(), value, decimals as usize)
    }
}

/// Format a temperature using host-side preferences. Called by [`format_temperature!`].
#[doc(hidden)]
#[must_use]
pub fn _host_format_temperature(value: f64, decimals: u32, show_unit: u32) -> String {
    #[cfg(target_arch = "wasm32")]
    {
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
    #[cfg(not(target_arch = "wasm32"))]
    {
        native::temperature_unit().format(
            native::number_format(),
            value,
            decimals as usize,
            show_unit != 0,
        )
    }
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
#[cfg(target_arch = "wasm32")]
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

#[cfg(target_arch = "wasm32")]
use crate::system::{self, DateFormat, TimeFormat};
#[cfg(target_arch = "wasm32")]
use crate::tz::Tz;

/// Overrides for [`format_time`]. Any `Some`-valued field replaces the
/// corresponding `system::current()` preference for this call only.
#[derive(Clone, Debug, Default)]
#[cfg(target_arch = "wasm32")]
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
#[cfg(target_arch = "wasm32")]
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
#[cfg(target_arch = "wasm32")]
pub fn format_time(now: crate::host::SystemTime, opts: FormatTimeOpts) -> String {
    let format = opts
        .format
        .or_else(|| system::current().time_format())
        .unwrap_or_default();
    let pattern = match (format, opts.with_seconds) {
        (TimeFormat::Hour24, false) => "%H:%M",
        (TimeFormat::Hour24, true) => "%H:%M:%S",
        (TimeFormat::Hour12, false) => "%I:%M",
        (TimeFormat::Hour12, true) => "%I:%M:%S",
    };
    strftime(
        local_unix_secs_or_system(&now, opts.timezone.as_ref()),
        pattern,
    )
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
#[cfg(target_arch = "wasm32")]
pub fn format_date(now: crate::host::SystemTime, opts: FormatDateOpts) -> String {
    let format = opts
        .format
        .or_else(|| system::current().date_format())
        .unwrap_or_default();
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
    strftime(
        local_unix_secs_or_system(&now, opts.timezone.as_ref()),
        pattern,
    )
}

/// Hour-only label for dense strips: `"20"` in 24-hour mode, `"8PM"` in
/// 12-hour mode. The meridiem is baked in so a bare hour is never ambiguous
/// between morning and evening. `tz` overrides the render timezone like
/// [`FormatTimeOpts::timezone`]; `None` uses the system timezone.
///
/// # Example
/// ```ignore
/// let s = format_hour(now, None); // "20" or "8PM"
/// ```
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn format_hour(now: crate::host::SystemTime, tz: Option<&Tz>) -> String {
    let pattern = match system::current().time_format().unwrap_or_default() {
        TimeFormat::Hour24 => "%H",
        TimeFormat::Hour12 => "%-I%p",
    };
    strftime(local_unix_secs_or_system(&now, tz), pattern)
}

/// The AM/PM marker for `now` under the user's settings, or `None` in 24-hour
/// mode. [`format_time`] deliberately omits it; render this beside the time as
/// a separate element when a 12-hour reading would otherwise be ambiguous.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn meridiem(now: crate::host::SystemTime, tz: Option<&Tz>) -> Option<String> {
    match system::current().time_format().unwrap_or_default() {
        TimeFormat::Hour24 => None,
        TimeFormat::Hour12 => Some(strftime(local_unix_secs_or_system(&now, tz), "%p")),
    }
}

/// `local_unix_secs` with a fallback chain: requested tz → system tz → raw UTC.
/// Used by the string-returning format helpers and by widgets that need to
/// shift `now.unix_secs` into wall-clock seconds before handing to
/// [`strftime`].
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn local_unix_secs_or_system(now: &crate::host::SystemTime, tz: Option<&Tz>) -> i64 {
    if let Some(t) = tz
        && let Some(secs) = local_unix_secs(now, t)
    {
        return secs;
    }
    if let Some(name) = system::current().timezone() {
        let system_tz = Tz::from_runtime(name);
        if let Some(secs) = local_unix_secs(now, &system_tz) {
            return secs;
        }
    }
    now.unix_secs
}

/// Shift `now.unix_secs` by `tz`'s UTC offset for a downstream `strftime`.
/// Returns `None` when the host doesn't recognise the tz name.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn local_unix_secs(now: &crate::host::SystemTime, tz: &Tz) -> Option<i64> {
    let offset_secs = resolve_tz_offset(tz, now.unix_secs)?;
    Some(now.unix_secs + i64::from(offset_secs))
}

/// Resolve the UTC offset (in seconds) for an IANA-name timezone at a
/// moment. Returns `None` when the host doesn't recognise the name.
#[must_use]
#[cfg(target_arch = "wasm32")]
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

/// Append a UTC offset as `+H` for whole hours or `+H:MM` when it has minutes.
/// Sign is always emitted; hours are unpadded, minutes zero-padded.
#[cfg(target_arch = "wasm32")]
pub fn push_utc_offset(s: &mut String, offset_secs: i32) {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.unsigned_abs();
    s.push(sign);
    push_int(s, i64::from(abs / 3_600));
    let mins = i64::from((abs % 3_600) / 60);
    if mins != 0 {
        s.push(':');
        push_pad2(s, mins);
    }
}

/// Outcome of resolving a timezone for a caption.
#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub enum TzLabel {
    /// Resolved cleanly: `city` is the display name, `offset_secs` its UTC offset.
    Resolved { city: String, offset_secs: i32 },
    /// The requested zone didn't resolve; `system_offset_secs` carries the
    /// system-timezone fallback (`0` when that also failed) so time projection
    /// still has an offset. Callers typically flag this state visually.
    Unknown {
        city: String,
        system_offset_secs: i32,
    },
}

/// Resolve `override_tz` → system timezone → UTC into a [`TzLabel`] at
/// `now_secs` (the offset is date-dependent through DST). Pass `None` to
/// caption the system timezone directly.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn resolve_tz_for_label(override_tz: Option<&Tz>, now_secs: i64) -> TzLabel {
    if let Some(t) = override_tz {
        if let Some(offset_secs) = resolve_tz_offset(t, now_secs) {
            return TzLabel::Resolved {
                city: t.city(),
                offset_secs,
            };
        }
        let system_offset_secs = system::current()
            .timezone()
            .and_then(|name| resolve_tz_offset(&Tz::from_runtime(name), now_secs))
            .unwrap_or(0);
        return TzLabel::Unknown {
            city: t.city(),
            system_offset_secs,
        };
    }
    if let Some(name) = system::current().timezone() {
        let tz = Tz::from_runtime(name);
        let city = tz.city();
        return match resolve_tz_offset(&tz, now_secs) {
            Some(offset_secs) => TzLabel::Resolved { city, offset_secs },
            None => TzLabel::Unknown {
                city,
                system_offset_secs: 0,
            },
        };
    }
    TzLabel::Unknown {
        city: "UTC".to_string(),
        system_offset_secs: 0,
    }
}

/// Append a [`TzLabel`]'s caption: `City (±H)` / `City (±H:MM)` when resolved
/// (see [`push_utc_offset`]), `City (unknown)` otherwise.
#[cfg(target_arch = "wasm32")]
pub fn push_tz_caption(s: &mut String, label: &TzLabel) {
    match label {
        TzLabel::Resolved { city, offset_secs } => {
            s.push_str(city);
            s.push_str(" (");
            push_utc_offset(s, *offset_secs);
            s.push(')');
        }
        TzLabel::Unknown { city, .. } => {
            s.push_str(city);
            s.push_str(" (unknown)");
        }
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
/// Input is always km/h; the host converts to mph if imperial (both arms).
/// Use the `ms` arm to request m/s when the system is metric.
///
/// # Example
/// ```ignore
/// let s = format_speed!(27_565.0, 0);       // "27 565 km/h" or "17 126 mph"
/// let s = format_speed!(12.6, 1, ms);       // "3,5 m/s" or "7,8 mph"
/// ```
#[macro_export]
macro_rules! format_speed {
    ($value:expr, $decimals:expr) => {
        $crate::format::_host_format_speed($value as f64, $decimals, 0)
    };
    ($value:expr, $decimals:expr, ms) => {
        $crate::format::_host_format_speed($value as f64, $decimals, 1)
    };
}

/// Format a distance value with user-preferred units and number formatting.
///
/// Input is always km; the host converts to miles if imperial.
///
/// # Example
/// ```ignore
/// let s = format_distance!(420.0, 0); // "420 km" or "261 mi"
/// ```
#[macro_export]
macro_rules! format_distance {
    ($value:expr, $decimals:expr) => {
        $crate::format::_host_format_distance($value as f64, $decimals)
    };
}

/// Format a temperature with user-preferred units and number formatting.
///
/// Input is always °C; the host converts to °F if preferred. Use the `bare`
/// arm for the degree-only form ("26°", no scale letter) used in dense
/// hourly/daily strips.
///
/// # Example
/// ```ignore
/// let s = format_temperature!(20.5, 1);       // "20,5 °C" or "68,9 °F"
/// let s = format_temperature!(20.0, 0, bare); // "20°" or "68°"
/// ```
#[macro_export]
macro_rules! format_temperature {
    ($value:expr, $decimals:expr) => {
        $crate::format::_host_format_temperature($value as f64, $decimals, 1)
    };
    ($value:expr, $decimals:expr, bare) => {
        $crate::format::_host_format_temperature($value as f64, $decimals, 0)
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
#[cfg(target_arch = "wasm32")]
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

/// Append `n`'s decimal digits to `s` (smallest representation, no padding).
#[cfg(target_arch = "wasm32")]
pub fn push_int(s: &mut String, n: i64) {
    if n >= 10 {
        push_int(s, n / 10);
    }
    s.push((b'0' + (n % 10) as u8) as char);
}

/// Append `n`'s decimal digits to `s`, zero-padded to two characters.
#[cfg(target_arch = "wasm32")]
pub fn push_pad2(s: &mut String, n: i64) {
    if n < 10 {
        s.push('0');
    }
    push_int(s, n);
}

/// Push a non-negative integer left-padded with `0` to `width` digits.
#[cfg(target_arch = "wasm32")]
fn push_padded(s: &mut String, n: i64, width: usize) {
    let digits = digit_count(n);
    for _ in digits..width {
        s.push('0');
    }
    push_int(s, n);
}

/// Decimal digit count of a non-negative `i64`, with `0` counted as one digit.
#[cfg(target_arch = "wasm32")]
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
#[cfg(target_arch = "wasm32")]
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

// These exercise the wasm-only pure helpers (duration / f64 / host-buffer),
// so the module is gated to wasm alongside them.
#[cfg(all(test, target_arch = "wasm32"))]
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

    fn caption(label: &TzLabel) -> String {
        let mut s = String::new();
        push_tz_caption(&mut s, label);
        s
    }

    #[test]
    fn tz_caption_resolved_whole_and_half_hour() {
        let whole = TzLabel::Resolved {
            city: "Prague".to_owned(),
            offset_secs: 7_200,
        };
        assert_eq!(caption(&whole), "Prague (+2)");
        let half = TzLabel::Resolved {
            city: "Kolkata".to_owned(),
            offset_secs: 19_800,
        };
        assert_eq!(caption(&half), "Kolkata (+5:30)");
    }

    #[test]
    fn tz_caption_resolved_negative_offset() {
        let label = TzLabel::Resolved {
            city: "New York".to_owned(),
            offset_secs: -18_000,
        };
        assert_eq!(caption(&label), "New York (-5)");
    }

    #[test]
    fn tz_caption_unknown_reads_unknown() {
        let label = TzLabel::Unknown {
            city: "Prague".to_owned(),
            system_offset_secs: 3_600,
        };
        assert_eq!(caption(&label), "Prague (unknown)");
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

#[cfg(test)]
mod si_decimals_tests {
    use super::si_decimals;

    #[test]
    fn targets_the_requested_significant_figures() {
        assert_eq!(si_decimals(13.2, 3), 1); // 13.2
        assert_eq!(si_decimals(154.0, 3), 0); // 154
        assert_eq!(si_decimals(9.11, 3), 2); // 9.11
        assert_eq!(si_decimals(312.5, 3), 0); // 313
        assert_eq!(si_decimals(0.0, 3), 2); // 0.00
    }
}
