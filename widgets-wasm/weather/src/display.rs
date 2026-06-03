// Copyright (C) 2026  Braiins Systems s.r.o.

#[cfg(target_arch = "wasm32")]
#[expect(clippy::wildcard_imports, reason = "widget code uses many SDK exports")]
use bmc_wasm_sdk::*;

pub const NOT_AVAILABLE: &str = "--";

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
pub fn temperature_or_placeholder(value_c: Option<f64>, fmt: impl Fn(f64) -> String) -> String {
    value_c.map_or_else(|| NOT_AVAILABLE.to_string(), fmt)
}

#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn temperature(value_c: f64) -> String {
    format_temperature!(value_c, 0)
}

#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn wind_speed_ms(value_kmh: f64) -> String {
    format_speed!(value_kmh, 0, ms)
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

#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn hour_label(rfc3339: &str, tz: Option<Tz>) -> String {
    let Some(unix_secs) = parse_date(rfc3339) else {
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

/// Hour-only label ("20", "8PM") for the dense hourly strip, whose entries
/// always fall on the hour. Delegates to the SDK so the 12-hour form carries a
/// meridiem. Sunrise and sunset keep their minutes via [`hour_label`].
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn forecast_hour_label(rfc3339: &str, tz: Option<&Tz>) -> String {
    let Some(unix_secs) = parse_date(rfc3339) else {
        return NOT_AVAILABLE.to_string();
    };
    format::format_hour(SystemTime { unix_secs }, tz)
}

/// The AM/PM marker for a sunrise/sunset time, or `None` in 24-hour mode.
/// Rendered as a separate element beside [`hour_label`] so a 12-hour reading
/// is unambiguous.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn clock_meridiem(rfc3339: &str, tz: Option<&Tz>) -> Option<String> {
    let unix_secs = parse_date(rfc3339)?;
    format::meridiem(SystemTime { unix_secs }, tz)
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
            temperature_or_placeholder(Some(20.0), |_| "20".to_string()),
            "20"
        );
    }
}
