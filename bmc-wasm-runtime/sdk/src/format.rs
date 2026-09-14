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
//! Preference-aware formatters keyed on the deck-wide `SystemSnapshot` (`number_format`,
//! `unit_system`, `temperature_unit`, timezone, time and date formats). wasm goes through
//! the host; native through the same `bmc_shared_utils` and calendar core, matching the device.
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
    let (mut mantissa, mut prefix) = si_split(value);
    let mut decimals = si_decimals(mantissa, sig_figs);
    // Rounding to those decimals can carry the mantissa to a thousand,
    // which belongs to the next prefix — split again from the rounded value.
    let rounded = round_to(mantissa, decimals);
    if rounded.abs() >= 1_000.0 {
        (mantissa, prefix) = si_split(rounded * si_magnitude(prefix));
        decimals = si_decimals(mantissa, sig_figs);
    }
    let value_str = _host_format_number(mantissa, decimals);
    let mut unit = String::with_capacity(prefix.len() + base_unit.len());
    unit.push_str(prefix);
    unit.push_str(base_unit);
    (value_str, unit)
}

fn si_split(value: f64) -> (f64, &'static str) {
    match unit_prefix::NumberPrefix::decimal(value) {
        unit_prefix::NumberPrefix::Standalone(m) => (m, ""),
        unit_prefix::NumberPrefix::Prefixed(p, m) => (m, p.symbol()),
    }
}

fn si_magnitude(prefix: &str) -> f64 {
    match prefix {
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "E" => 1e18,
        "Z" => 1e21,
        "Y" => 1e24,
        "" => 1.0,
        _ => unreachable!("BUG: NumberPrefix::decimal hands out only k..Y"),
    }
}

fn round_to(value: f64, decimals: u32) -> f64 {
    let scale = 10_f64.powi(i32::try_from(decimals).expect("BUG: SI decimals fit i32"));
    (value * scale).round() / scale
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
/// On device the result crosses a 64-byte buffer and is cut there,
/// possibly inside a multibyte character; off-device it is not.
/// Keep patterns well short of it — the longest one shipped,
/// `%a %-d %B %Y`, is under half.
///
/// # Example
/// ```ignore
/// let ts = parse_datetime("2026-03-04T04:19:23+00:00").unwrap();
/// let s = strftime(ts, "%m/%d %H:%M"); // "03/04 04:19"
/// let s = strftime(ts, "%d.%m.%Y %H:%M:%S"); // "04.03.2026 04:19:23"
/// ```
#[must_use]
pub fn strftime(timestamp: i64, format: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    {
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
    #[cfg(not(target_arch = "wasm32"))]
    {
        bmc_wasm_protocol::time::strftime_utc(timestamp, format).unwrap_or_default()
    }
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
    /// `None` uses the system timezone, as does a name the deck does not know
    /// (see [`local_unix_secs_or_system`]).
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

/// Format the time component of a [`SystemTime`](crate::host::SystemTime)
/// per the user's preferences, with per-call overrides.
/// AM/PM is **not** included in the output — render it as a separate element
/// when `system::current().time_format()` is [`TimeFormat::Hour12`].
///
/// # Example
/// ```ignore
/// let now = SystemTime::now();
/// let s = format_time(now, FormatTimeOpts::default());                       // "13:45"
/// let s = format_time(now, FormatTimeOpts { with_seconds: true, ..default }); // "13:45:09"
/// ```
#[must_use]
pub fn format_time(now: crate::host::SystemTime, opts: FormatTimeOpts) -> String {
    let FormatTimeOpts {
        format,
        timezone,
        with_seconds,
    } = opts;
    let format = format
        .or_else(|| system::current().time_format())
        .unwrap_or_default();
    let pattern = match (format, with_seconds) {
        (TimeFormat::Hour24, false) => "%H:%M",
        (TimeFormat::Hour24, true) => "%H:%M:%S",
        (TimeFormat::Hour12, false) => "%I:%M",
        (TimeFormat::Hour12, true) => "%I:%M:%S",
    };
    strftime(local_unix_secs_or_system(&now, timezone.as_ref()), pattern)
}

/// Format the date component of a [`SystemTime`](crate::host::SystemTime)
/// per the user's preferences, with per-call overrides.
/// Output mirrors the operator's configured locale (e.g. `12.03.2026` vs `03/12/2026`).
///
/// # Example
/// ```ignore
/// let now = SystemTime::now();
/// let s = format_date(now, FormatDateOpts::default()); // "12.03.2026"
/// ```
#[must_use]
pub fn format_date(now: crate::host::SystemTime, opts: FormatDateOpts) -> String {
    let FormatDateOpts { format, timezone } = opts;
    let format = format
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
    strftime(local_unix_secs_or_system(&now, timezone.as_ref()), pattern)
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
pub fn local_unix_secs(now: &crate::host::SystemTime, tz: &Tz) -> Option<i64> {
    let offset_secs = resolve_tz_offset(tz, now.unix_secs)?;
    Some(now.unix_secs + i64::from(offset_secs))
}

/// Resolve the UTC offset (in seconds) for an IANA-name timezone at a
/// moment. Returns `None` when the host doesn't recognise the name.
#[must_use]
pub fn resolve_tz_offset(tz: &Tz, unix_secs: i64) -> Option<i32> {
    #[cfg(target_arch = "wasm32")]
    {
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
    #[cfg(not(target_arch = "wasm32"))]
    {
        bmc_wasm_protocol::time::zone_offset_secs(unix_secs, tz.iana())
    }
}

/// Append a UTC offset as `+H` for whole hours or `+H:MM` when it has minutes.
/// Sign is always emitted; hours are unpadded, minutes zero-padded.
pub fn push_utc_offset(s: &mut String, offset_secs: i32) {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.unsigned_abs();
    s.push(sign);
    push_int(s, i64::from(abs.div_euclid(3_600)));
    let mins = i64::from(abs.rem_euclid(3_600).div_euclid(60));
    if mins != 0 {
        s.push(':');
        push_pad2(s, mins);
    }
}

/// Outcome of resolving a timezone for a caption.
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
#[must_use]
pub fn format_duration(remaining_secs: i64, show_seconds: bool) -> String {
    if remaining_secs <= 0 {
        return String::from("T-0");
    }

    let d = remaining_secs.div_euclid(86_400);
    let h = remaining_secs.rem_euclid(86_400).div_euclid(3_600);
    let m = remaining_secs.rem_euclid(3_600).div_euclid(60);
    let s = remaining_secs.rem_euclid(60);

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
pub fn push_int(s: &mut String, n: i64) {
    if n >= 10 {
        push_int(s, n.div_euclid(10));
    }
    let digit = u8::try_from(n.rem_euclid(10)).expect("BUG: rem_euclid(10) is 0..=9");
    s.push((b'0' + digit) as char);
}

/// Append `n`'s decimal digits to `s`, zero-padded to two characters.
pub fn push_pad2(s: &mut String, n: i64) {
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
    let int_part = scaled.abs().div_euclid(factor);
    let frac_part = scaled.abs().rem_euclid(factor);

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
mod tests;
