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

#[cfg(any(target_arch = "wasm32", test))]
#[expect(clippy::wildcard_imports, reason = "widget code uses many SDK exports")]
use bmc_wasm_sdk::*;
use units::units::{DegreeCelsius, KilometerPerHour};

pub const NOT_AVAILABLE: &str = "--";
pub const ENTER_LOCATION: &str = "Enter location";
pub const LOADING: &str = "Loading…";
pub const CANNOT_LOAD: &str = "Cannot load data";

#[must_use]
pub fn wind_line(direction: &str, speed: &str) -> String {
    let mut s = String::with_capacity("Wind From The ".len() + direction.len() + 1 + speed.len());
    s.push_str("Wind From The ");
    s.push_str(direction);
    s.push(' ');
    s.push_str(speed);
    s
}

#[must_use]
pub fn temperature_or_placeholder(
    value: Option<DegreeCelsius>,
    fmt: impl Fn(DegreeCelsius) -> String,
) -> String {
    value.map_or_else(|| NOT_AVAILABLE.to_string(), fmt)
}

#[must_use]
pub fn temperature(value: DegreeCelsius) -> String {
    units::format::temperature(value, 0)
}

/// Degree-only temperature ("26°") for the dense hourly and daily strips,
/// where the scale letter would crowd the layout.
#[must_use]
pub fn temperature_bare(value: DegreeCelsius) -> String {
    units::format::temperature_bare(value, 0)
}

#[must_use]
pub fn wind_speed(value: KilometerPerHour) -> String {
    units::format::speed(value, 1)
}

#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn select_tz(mode: crate::manifest_params::TimeZone, location_tz: &str) -> Option<Tz> {
    use crate::manifest_params::TimeZone;
    match mode {
        TimeZone::Location => Some(Tz::from_runtime(location_tz)),
        TimeZone::System => None,
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[must_use]
pub fn select_tz(_mode: crate::manifest_params::TimeZone, _location_tz: &str) -> Option<Tz> {
    None
}

#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn hour_label(rfc3339: &str, tz: Option<Tz>) -> String {
    let Some(unix_secs) = parse_datetime(rfc3339) else {
        return NOT_AVAILABLE.to_string();
    };
    format_time(
        SystemTime { unix_secs },
        FormatTimeOpts {
            timezone: tz,
            ..FormatTimeOpts::default()
        },
    )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[must_use]
pub fn hour_label(rfc3339: &str, _tz: Option<Tz>) -> String {
    rfc3339
        .get(11..16)
        .map_or_else(|| NOT_AVAILABLE.to_string(), ToString::to_string)
}

/// Hour-only label ("20", "8PM") for the dense hourly strip, whose entries
/// always fall on the hour. Delegates to the SDK so the 12-hour form carries a
/// meridiem. Sunrise and sunset keep their minutes via [`hour_label`].
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn forecast_hour_label(rfc3339: &str, tz: Option<&Tz>) -> String {
    let Some(unix_secs) = parse_datetime(rfc3339) else {
        return NOT_AVAILABLE.to_string();
    };
    format::format_hour(SystemTime { unix_secs }, tz)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[must_use]
pub fn forecast_hour_label(rfc3339: &str, _tz: Option<&Tz>) -> String {
    rfc3339
        .get(11..13)
        .map_or_else(|| NOT_AVAILABLE.to_string(), ToString::to_string)
}

/// The AM/PM marker for a sunrise/sunset time, or `None` in 24-hour mode.
/// Rendered as a separate element beside [`hour_label`] so a 12-hour reading
/// is unambiguous.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn clock_meridiem(rfc3339: &str, tz: Option<&Tz>) -> Option<String> {
    let unix_secs = parse_datetime(rfc3339)?;
    format::meridiem(SystemTime { unix_secs }, tz)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[must_use]
pub fn clock_meridiem(_rfc3339: &str, _tz: Option<&Tz>) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wind_line_reads_direction_then_speed() {
        assert_eq!(wind_line("South", "3 m/s"), "Wind From The South 3 m/s");
    }

    #[test]
    fn placeholder_used_when_value_absent() {
        assert_eq!(temperature_or_placeholder(None, |_| unreachable!()), "--");
    }

    #[test]
    fn value_present_runs_the_formatter() {
        assert_eq!(
            temperature_or_placeholder(Some(DegreeCelsius(20.0)), |_| "20".to_string()),
            "20"
        );
    }
}
