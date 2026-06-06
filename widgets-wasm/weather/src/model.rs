// Copyright (C) 2026  Braiins Systems s.r.o.

#[cfg(target_arch = "wasm32")]
use bmc_wasm_sdk::JsonDoc;
use units::availability::Availability;
use units::units::{Degree, DegreeCelsius, KilometerPerHour, Quantity};

pub struct Location {
    pub display_name: String,
    pub timezone: String,
}

pub struct Current {
    pub temperature: DegreeCelsius,
    pub weather_code: i64,
    pub wind_speed: Availability<KilometerPerHour>,
    pub wind_direction: Availability<Degree>,
    pub is_day: bool,
}

pub struct HourEntry {
    pub time_rfc3339: String,
    pub temperature: DegreeCelsius,
    pub weather_code: i64,
    pub is_day: bool,
}

pub struct Hourly {
    pub entries: Vec<HourEntry>,
    /// Index of the first entry at or after the current time — the strips
    /// render from here, not from the start-of-day entry at index 0.
    pub start_index: usize,
}

/// First hourly entry at or after `current_time_rfc3339`, else 0. Mirrors
/// deckfeeder's `getCurrentHourIndex`. Compares parsed instants, not strings:
/// a forecast array can change UTC offset mid-run across a DST transition, and
/// a lexicographic compare would then order those hours wrong.
#[must_use]
pub fn hourly_start_index(entries: &[HourEntry], current_time_rfc3339: &str) -> usize {
    let Some(now) = rfc3339_to_unix(current_time_rfc3339) else {
        return 0;
    };
    entries
        .iter()
        .position(|e| rfc3339_to_unix(&e.time_rfc3339).is_some_and(|t| t >= now))
        .unwrap_or(0)
}

pub struct DayForecast {
    pub time_rfc3339: String,
    pub weather_code: i64,
    pub min: DegreeCelsius,
    pub max: DegreeCelsius,
    pub sunrise: String,
    pub sunset: String,
}

pub struct Daily {
    pub days: Vec<DayForecast>,
    pub today_index: usize,
    pub today_sunrise: String,
    pub today_sunset: String,
}

pub struct Weather {
    pub location: Location,
    pub current: Option<Current>,
    pub hourly: Option<Hourly>,
    pub daily: Option<Daily>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherParseError {
    InvalidDocument,
    MissingRequiredField(&'static str),
}

/// Parse an RFC3339 timestamp (`YYYY-MM-DDTHH:MM:SS` followed by `Z` or
/// `±HH:MM`) to a UTC unix timestamp in seconds. Pure and panic-free: it
/// reads fixed byte positions and returns `None` for anything malformed, so a
/// garbage API value can never panic the render path.
#[must_use]
pub fn rfc3339_to_unix(rfc3339: &str) -> Option<i64> {
    let b = rfc3339.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let year = parse_digits(&b[0..4])?;
    let month = parse_digits(&b[5..7])?;
    let day = parse_digits(&b[8..10])?;
    let hour = parse_digits(&b[11..13])?;
    let minute = parse_digits(&b[14..16])?;
    let second = parse_digits(&b[17..19])?;
    let days = days_from_civil(year, month, day)?;
    let local = days * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(local - tz_offset_seconds(b))
}

/// English weekday name for the calendar date in `rfc3339`. Reads only the
/// `YYYY-MM-DD` head, so the label is the timestamp's own local day and never
/// rolls to an adjacent day under a system- or UTC-timezone shift.
#[must_use]
pub fn weekday_name(rfc3339: &str) -> Option<&'static str> {
    const WEEKDAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    let b = rfc3339.as_bytes();
    if b.len() < 10 {
        return None;
    }
    let year = parse_digits(&b[0..4])?;
    let month = parse_digits(&b[5..7])?;
    let day = parse_digits(&b[8..10])?;
    let days = days_from_civil(year, month, day)?;
    // Day 0 (1970-01-01) is a Thursday; index 0 is Sunday.
    let index = usize::try_from((days + 4).rem_euclid(7)).ok()?;
    Some(WEEKDAYS[index])
}

/// Parse a run of ASCII digits to an `i64`; `None` if any byte is not a digit.
fn parse_digits(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() {
        return None;
    }
    let mut value: i64 = 0;
    for &c in bytes {
        if !c.is_ascii_digit() {
            return None;
        }
        value = value * 10 + i64::from(c - b'0');
    }
    Some(value)
}

/// Days from 1970-01-01 to a proleptic-Gregorian date (Howard Hinnant's
/// algorithm). `None` for an out-of-range month or day.
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// UTC offset in seconds from an RFC3339 byte tail: `Z` → 0, `±HH:MM` → signed
/// seconds. Returns 0 for anything unrecognised (e.g. a naive timestamp).
fn tz_offset_seconds(b: &[u8]) -> i64 {
    if b.last() == Some(&b'Z') {
        return 0;
    }
    if b.len() < 6 {
        return 0;
    }
    let tail = &b[b.len() - 6..];
    let sign: i64 = match tail[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return 0,
    };
    if tail[3] != b':' {
        return 0;
    }
    let hh = parse_digits(&tail[1..3]).unwrap_or(0);
    let mm = parse_digits(&tail[4..6]).unwrap_or(0);
    sign * (hh * 3_600 + mm * 60)
}

#[must_use]
pub fn current_is_day(hourly: Option<&Hourly>, current_time_rfc3339: &str) -> bool {
    let Some(hourly) = hourly else { return true };
    let pick = rfc3339_to_unix(current_time_rfc3339)
        .and_then(|now| {
            hourly
                .entries
                .iter()
                .find(|e| rfc3339_to_unix(&e.time_rfc3339).is_some_and(|t| t >= now))
        })
        .or_else(|| hourly.entries.first());
    pick.is_none_or(|e| e.is_day)
}

pub struct ForecastRange {
    pub min: DegreeCelsius,
    pub max: DegreeCelsius,
}

impl ForecastRange {
    #[must_use]
    pub fn of(days: &[DayForecast]) -> ForecastRange {
        let Some(first) = days.first() else {
            return ForecastRange {
                min: DegreeCelsius(0.0),
                max: DegreeCelsius(0.0),
            };
        };
        let mut min = first.min.raw();
        let mut max = first.max.raw();
        for day in days.iter().skip(1) {
            if day.min.raw() < min {
                min = day.min.raw();
            }
            if day.max.raw() > max {
                max = day.max.raw();
            }
        }
        ForecastRange {
            min: DegreeCelsius(min),
            max: DegreeCelsius(max),
        }
    }

    #[must_use]
    pub fn fraction(&self, value: DegreeCelsius) -> f64 {
        let span = self.max.raw() - self.min.raw();
        if span <= 0.0 {
            return 0.0;
        }
        ((value.raw() - self.min.raw()) / span).clamp(0.0, 1.0)
    }
}

#[cfg(target_arch = "wasm32")]
impl TryFrom<&JsonDoc> for Weather {
    type Error = WeatherParseError;

    fn try_from(doc: &JsonDoc) -> Result<Self, Self::Error> {
        if !doc.is_valid() {
            return Err(WeatherParseError::InvalidDocument);
        }
        let display_name = doc.str("/data/location/display_name").ok_or(
            WeatherParseError::MissingRequiredField("/data/location/display_name"),
        )?;
        let timezone =
            doc.str("/data/location/timezone")
                .ok_or(WeatherParseError::MissingRequiredField(
                    "/data/location/timezone",
                ))?;
        let location = Location {
            display_name,
            timezone,
        };

        let current_time = doc.str("/data/current/time").unwrap_or_default();
        let mut hourly = parse_hourly(doc);
        if let Some(h) = hourly.as_mut() {
            h.start_index = hourly_start_index(&h.entries, &current_time);
        }
        let current = parse_current(doc, hourly.as_ref(), &current_time);
        let daily = parse_daily(doc);

        Ok(Weather {
            location,
            current,
            hourly,
            daily,
        })
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_current(doc: &JsonDoc, hourly: Option<&Hourly>, current_time: &str) -> Option<Current> {
    let temperature = DegreeCelsius(doc.f64("/data/current/temperature")?);
    let weather_code = doc.i64("/data/current/weather_code")?;
    Some(Current {
        temperature,
        weather_code,
        wind_speed: doc
            .f64("/data/current/wind_speed")
            .map(KilometerPerHour)
            .into(),
        wind_direction: doc
            .f64("/data/current/wind_direction_degrees")
            .map(Degree)
            .into(),
        is_day: current_is_day(hourly, current_time),
    })
}

#[cfg(target_arch = "wasm32")]
fn parse_hourly(doc: &JsonDoc) -> Option<Hourly> {
    use bmc_wasm_sdk::ufmt;
    let mut entries = Vec::new();
    for i in 0..256_usize {
        let Some(time_rfc3339) = doc.str(&bmc_wasm_sdk::fmt!("/data/hourly/time/{}", i)) else {
            break;
        };
        let temperature_c = doc.f64(&bmc_wasm_sdk::fmt!("/data/hourly/temperature/{}", i));
        let weather_code = doc.i64(&bmc_wasm_sdk::fmt!("/data/hourly/weather_code/{}", i));
        let is_day = doc.bool(&bmc_wasm_sdk::fmt!("/data/hourly/is_day/{}", i));
        let (Some(temperature_c), Some(weather_code), Some(is_day)) =
            (temperature_c, weather_code, is_day)
        else {
            bmc_wasm_sdk::log_warn!(
                "weather: hourly entry {} incomplete, truncating strip at {} entries",
                i,
                entries.len()
            );
            break;
        };
        entries.push(HourEntry {
            time_rfc3339,
            temperature: DegreeCelsius(temperature_c),
            weather_code,
            is_day,
        });
    }
    if entries.is_empty() {
        None
    } else {
        Some(Hourly {
            entries,
            start_index: 0,
        })
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_daily(doc: &JsonDoc) -> Option<Daily> {
    use bmc_wasm_sdk::ufmt;
    let mut days = Vec::new();
    for i in 0..256_usize {
        let Some(time_rfc3339) = doc.str(&bmc_wasm_sdk::fmt!("/data/daily/time/{}", i)) else {
            break;
        };
        let weather_code = doc.i64(&bmc_wasm_sdk::fmt!("/data/daily/weather_code/{}", i));
        let min_c = doc.f64(&bmc_wasm_sdk::fmt!("/data/daily/temperature_min/{}", i));
        let max_c = doc.f64(&bmc_wasm_sdk::fmt!("/data/daily/temperature_max/{}", i));
        let sunrise = doc.str(&bmc_wasm_sdk::fmt!("/data/daily/sunrise/{}", i));
        let sunset = doc.str(&bmc_wasm_sdk::fmt!("/data/daily/sunset/{}", i));
        let (Some(weather_code), Some(min_c), Some(max_c), Some(sunrise), Some(sunset)) =
            (weather_code, min_c, max_c, sunrise, sunset)
        else {
            bmc_wasm_sdk::log_warn!(
                "weather: daily entry {} incomplete, truncating forecast at {} days",
                i,
                days.len()
            );
            break;
        };
        days.push(DayForecast {
            time_rfc3339,
            weather_code,
            min: DegreeCelsius(min_c),
            max: DegreeCelsius(max_c),
            sunrise,
            sunset,
        });
    }
    if days.is_empty() {
        return None;
    }
    let today_index = doc
        .i64("/data/daily/today/index")
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(0)
        .min(days.len().saturating_sub(1));
    let today_sunrise = doc.str("/data/daily/today/sunrise").unwrap_or_default();
    let today_sunset = doc.str("/data/daily/today/sunset").unwrap_or_default();
    Some(Daily {
        days,
        today_index,
        today_sunrise,
        today_sunset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_to_unix_matches_known_epochs() {
        assert_eq!(rfc3339_to_unix("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_to_unix("2000-01-01T00:00:00Z"), Some(946_684_800));
    }

    #[test]
    fn rfc3339_to_unix_applies_the_offset() {
        // Same wall time, different zone: the more-positive offset is the
        // earlier instant, so the Z reading is later by exactly the offset.
        let plus2 = rfc3339_to_unix("2026-06-03T12:00:00+02:00")
            .expect("BUG: test timestamp with +02:00 offset is valid RFC3339");
        let utc = rfc3339_to_unix("2026-06-03T12:00:00Z")
            .expect("BUG: test timestamp with UTC offset is valid RFC3339");
        let minus0530 = rfc3339_to_unix("2026-06-03T12:00:00-05:30")
            .expect("BUG: test timestamp with -05:30 offset is valid RFC3339");
        assert_eq!(utc - plus2, 7_200);
        assert_eq!(minus0530 - utc, 19_800);
    }

    #[test]
    fn rfc3339_to_unix_is_none_on_garbage_and_never_panics() {
        assert_eq!(rfc3339_to_unix("garbage"), None);
        assert_eq!(rfc3339_to_unix(""), None);
        assert_eq!(rfc3339_to_unix("2026-13-03T00:00:00Z"), None);
        // A multibyte tail must not panic a byte-index slice; just no offset.
        assert!(rfc3339_to_unix("2026-06-03T00:00:00€€").is_some());
    }

    #[test]
    fn weekday_name_uses_local_date_not_the_utc_instant() {
        // 2026-06-03 is a Wednesday. A local-midnight stamp at a negative
        // offset is a previous-day instant in UTC; the label must still read
        // the date's own day, never rolling back to Tuesday.
        assert_eq!(weekday_name("2026-06-03T00:00:00-02:00"), Some("Wednesday"));
        assert_eq!(weekday_name("2026-06-03T00:00:00Z"), Some("Wednesday"));
        assert_eq!(weekday_name("2026-06-03T00:00:00+14:00"), Some("Wednesday"));
        // Day 0 of the epoch is a Thursday.
        assert_eq!(weekday_name("1970-01-01T00:00:00Z"), Some("Thursday"));
        assert_eq!(weekday_name("nope"), None);
    }

    fn hour(time: &str, is_day: bool) -> HourEntry {
        HourEntry {
            time_rfc3339: time.to_string(),
            temperature: DegreeCelsius(10.0),
            weather_code: 1,
            is_day,
        }
    }

    #[test]
    fn current_is_day_picks_first_hour_at_or_after_now() {
        let h = Hourly {
            entries: vec![
                hour("2026-06-03T18:00:00+02:00", true),
                hour("2026-06-03T21:00:00+02:00", false),
            ],
            start_index: 0,
        };
        assert!(!current_is_day(Some(&h), "2026-06-03T19:30:00+02:00"));
        assert!(current_is_day(Some(&h), "2026-06-03T17:00:00+02:00"));
    }

    #[test]
    fn hourly_start_index_finds_first_hour_at_or_after_now() {
        let entries = vec![
            hour("2026-06-03T00:00:00+02:00", true),
            hour("2026-06-03T18:00:00+02:00", true),
            hour("2026-06-03T19:00:00+02:00", false),
        ];
        // 18:30 -> first entry >= it is the 19:00 one at index 2.
        assert_eq!(hourly_start_index(&entries, "2026-06-03T18:30:00+02:00"), 2);
        // Exact match returns that index, not the next.
        assert_eq!(hourly_start_index(&entries, "2026-06-03T18:00:00+02:00"), 1);
        // Past the last entry -> falls back to 0.
        assert_eq!(hourly_start_index(&entries, "2026-06-04T00:00:00+02:00"), 0);
    }

    #[test]
    fn hourly_start_index_orders_by_instant_across_dst() {
        // Autumn fall-back: the offset drops +02:00 -> +01:00 mid-array, so
        // the +01:00 entries are the chronologically later ones even though
        // "+01:00" sorts before "+02:00" lexically.
        let entries = vec![
            hour("2026-10-25T02:00:00+02:00", true),  // 00:00Z
            hour("2026-10-25T02:00:00+01:00", true),  // 01:00Z
            hour("2026-10-25T03:00:00+01:00", false), // 02:00Z
        ];
        // now = 00:30Z. By instant the first entry at/after is the 01:00Z one
        // at index 1; a string compare would wrongly pick index 2.
        assert_eq!(hourly_start_index(&entries, "2026-10-25T02:30:00+02:00"), 1);
    }

    #[test]
    fn current_is_day_defaults_true_without_hourly() {
        assert!(current_is_day(None, "2026-06-03T19:30:00+02:00"));
    }

    fn day(min_c: f64, max_c: f64) -> DayForecast {
        DayForecast {
            time_rfc3339: "2026-06-03T00:00:00+02:00".to_string(),
            weather_code: 3,
            min: DegreeCelsius(min_c),
            max: DegreeCelsius(max_c),
            sunrise: "2026-06-03T04:56:21+02:00".to_string(),
            sunset: "2026-06-03T21:04:41+02:00".to_string(),
        }
    }

    fn loc() -> Location {
        Location {
            display_name: "Prague, Czech Republic".to_string(),
            timezone: "Europe/Prague".to_string(),
        }
    }

    #[test]
    fn forecast_range_spans_min_and_max_across_days() {
        let days = vec![day(16.2, 21.0), day(10.9, 25.8)];
        let range = ForecastRange::of(&days);
        assert!((range.min.raw() - 10.9).abs() < 1e-9);
        assert!((range.max.raw() - 25.8).abs() < 1e-9);
    }

    #[test]
    fn day_fraction_positions_a_value_within_the_global_range() {
        let range = ForecastRange {
            min: DegreeCelsius(10.0),
            max: DegreeCelsius(30.0),
        };
        assert!((range.fraction(DegreeCelsius(20.0)) - 0.5).abs() < 1e-9);
        assert!((range.fraction(DegreeCelsius(10.0)) - 0.0).abs() < 1e-9);
        assert!((range.fraction(DegreeCelsius(30.0)) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn missing_current_does_not_imply_missing_daily() {
        let daily = Daily {
            days: vec![day(10.9, 25.8)],
            today_index: 0,
            today_sunrise: String::new(),
            today_sunset: String::new(),
        };
        let w = Weather {
            current: None,
            daily: Some(daily),
            hourly: None,
            location: loc(),
        };
        assert!(w.current.is_none());
        assert!(w.daily.is_some());
    }
}
