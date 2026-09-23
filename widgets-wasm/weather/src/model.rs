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

//! Which layout a viewport gets, and the forecast the layouts draw.

use bmc_wasm_sdk::{SizeVariant, SystemTime, WidgetSize, WidgetViewport};
use units::availability::Availability;
use units::units::{Degree, DegreeCelsius, KilometerPerHour, Quantity};

/// The frames a layout is picked for: the four BMC100 slots and BMM101's 480×320.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeBucket {
    Full,
    Large,
    Medium,
    Small,
    Bmm101,
}

impl SizeBucket {
    #[must_use]
    pub const fn design_size(self) -> (u32, u32) {
        match self {
            Self::Full => (1_280, 480),
            Self::Large => (638, 480),
            Self::Medium => (638, 238),
            Self::Small => (317, 238),
            Self::Bmm101 => (480, 320),
        }
    }
}

/// A rectangular viewport classified once for every layout:
/// its pixels with their closest BMC100 variant, and the bucket that picks the layout.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub size: WidgetSize,
    pub bucket: SizeBucket,
}

impl Frame {
    #[must_use]
    pub fn of(viewport: WidgetViewport) -> Self {
        Self {
            size: WidgetSize::from_dimensions(viewport.width, viewport.height),
            bucket: size_bucket(viewport.width, viewport.height),
        }
    }
}

/// The two BMM frames the narrow bucket has to tell apart.
const BMM100_HEIGHT: u32 = 240;
const BMM101_HEIGHT: u32 = 320;
/// Split at their midpoint, so either frame keeps its bucket a few pixels either way.
const BMM101_MIN_HEIGHT: u32 = u32::midpoint(BMM100_HEIGHT, BMM101_HEIGHT);
const BMM101_MAX_WIDTH: u32 = 480;

/// A landscape frame no wider than BMM101 and at least as tall as the BMM split
/// is BMM101; everything else takes the closest BMC100 variant, as the SDK does.
#[must_use]
pub fn size_bucket(width: u32, height: u32) -> SizeBucket {
    if width <= BMM101_MAX_WIDTH && height < width && height >= BMM101_MIN_HEIGHT {
        return SizeBucket::Bmm101;
    }
    match SizeVariant::closest(width, height) {
        SizeVariant::Full => SizeBucket::Full,
        SizeVariant::Large => SizeBucket::Large,
        SizeVariant::Medium => SizeBucket::Medium,
        SizeVariant::Small => SizeBucket::Small,
    }
}

/// What the widget holds for its location.
#[derive(Clone, Debug)]
pub enum State {
    Loading,
    Loaded(Weather),
    BadLocation,
    Error,
}

#[derive(Clone, Debug)]
pub struct Location {
    pub display_name: String,
    pub timezone: String,
}

#[derive(Clone, Debug)]
pub struct Current {
    pub temperature: DegreeCelsius,
    pub weather_code: i64,
    pub wind_speed: Availability<KilometerPerHour>,
    pub wind_direction: Availability<Degree>,
    pub is_day: bool,
}

#[derive(Clone, Debug)]
pub struct HourEntry {
    /// `None` when the payload's time does not parse.
    pub at: Option<SystemTime>,
    pub temperature: DegreeCelsius,
    pub weather_code: i64,
    pub is_day: bool,
}

#[derive(Clone, Debug)]
pub struct Hourly {
    pub entries: Vec<HourEntry>,
    /// Index of the first entry at or after the current time — the strips
    /// render from here, not from the start-of-day entry at index 0.
    pub start_index: usize,
}

/// First hourly entry at or after `now`, else 0. Mirrors deckfeeder's `getCurrentHourIndex`.
#[must_use]
pub fn hourly_start_index(entries: &[HourEntry], now: Option<SystemTime>) -> usize {
    let Some(now) = now else {
        return 0;
    };
    entries
        .iter()
        .position(|e| e.at.is_some_and(|at| at.unix_secs >= now.unix_secs))
        .unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct DayForecast {
    pub time_rfc3339: String,
    pub weather_code: i64,
    pub min: DegreeCelsius,
    pub max: DegreeCelsius,
}

#[derive(Clone, Debug)]
pub struct Daily {
    pub days: Vec<DayForecast>,
    pub today_index: usize,
    pub today_sunrise: Option<SystemTime>,
    pub today_sunset: Option<SystemTime>,
}

impl Daily {
    /// The forecast slice starting at today, capped at `max` days, so the
    /// shown rows always begin at today regardless of leading past entries.
    #[must_use]
    pub fn forecast_window(&self, max: usize) -> &[DayForecast] {
        let from = &self.days[self.today_index..];
        &from[..from.len().min(max)]
    }
}

#[derive(Clone, Debug)]
pub struct Weather {
    pub location: Location,
    pub current: Option<Current>,
    pub hourly: Option<Hourly>,
    pub daily: Option<Daily>,
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

/// Whether the hour the strip starts at is daylight; day when there is no hour to go by.
#[must_use]
pub fn current_is_day(hourly: Option<&Hourly>, now: Option<SystemTime>) -> bool {
    hourly.is_none_or(|h| {
        h.entries
            .get(hourly_start_index(&h.entries, now))
            .is_none_or(|e| e.is_day)
    })
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 3 June 2026, 18:00 UTC; the hours below are staged around it.
    const EVENING: i64 = 1_780_509_600;
    const HOUR: i64 = 3_600;

    fn at(unix_secs: i64) -> Option<SystemTime> {
        Some(SystemTime { unix_secs })
    }

    fn hour(unix_secs: i64, is_day: bool) -> HourEntry {
        HourEntry {
            at: at(unix_secs),
            temperature: DegreeCelsius(10.0),
            weather_code: 1,
            is_day,
        }
    }

    #[test]
    fn hourly_start_index_finds_first_hour_at_or_after_now() {
        let entries = vec![
            hour(EVENING - 18 * HOUR, true),
            hour(EVENING, true),
            hour(EVENING + HOUR, false),
        ];
        assert_eq!(hourly_start_index(&entries, at(EVENING + HOUR / 2)), 2);
        assert_eq!(
            hourly_start_index(&entries, at(EVENING)),
            1,
            "an exact match starts at that hour"
        );
        assert_eq!(
            hourly_start_index(&entries, at(EVENING + 6 * HOUR)),
            0,
            "past the last hour falls back to the first"
        );
    }

    #[test]
    fn hourly_start_index_passes_over_an_hour_that_did_not_parse() {
        let unparsed = HourEntry {
            at: None,
            ..hour(EVENING, true)
        };
        let entries = vec![unparsed, hour(EVENING + HOUR, true)];
        assert_eq!(hourly_start_index(&entries, at(EVENING)), 1);
    }

    #[test]
    fn hourly_start_index_starts_at_the_first_hour_without_a_current_time() {
        let entries = vec![hour(EVENING, true), hour(EVENING + HOUR, false)];
        assert_eq!(hourly_start_index(&entries, None), 0);
    }

    #[test]
    fn current_is_day_reads_the_hour_the_strip_starts_at() {
        let h = Hourly {
            entries: vec![hour(EVENING, true), hour(EVENING + 3 * HOUR, false)],
            start_index: 0,
        };
        assert!(!current_is_day(Some(&h), at(EVENING + HOUR)));
        assert!(current_is_day(Some(&h), at(EVENING - HOUR)));
    }

    #[test]
    fn current_is_day_defaults_true_without_hourly() {
        assert!(current_is_day(None, at(EVENING)));
    }

    fn day(min_c: f64, max_c: f64) -> DayForecast {
        DayForecast {
            time_rfc3339: "2026-06-03T00:00:00+02:00".to_string(),
            weather_code: 3,
            min: DegreeCelsius(min_c),
            max: DegreeCelsius(max_c),
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
    fn forecast_window_starts_at_today_and_caps_length() {
        let daily = Daily {
            days: vec![day(1.0, 2.0), day(3.0, 4.0), day(5.0, 6.0)],
            today_index: 1,
            today_sunrise: None,
            today_sunset: None,
        };
        // The window begins at today (index 1), never the leading past day,
        // so the Today label and current-temperature marker land on row 0.
        let window = daily.forecast_window(4);
        assert_eq!(window.len(), 2);
        assert!((window[0].min.raw() - 3.0).abs() < 1e-9);
        // A cap shorter than the remaining days truncates from today forward.
        assert_eq!(daily.forecast_window(1).len(), 1);
    }

    #[test]
    fn missing_current_does_not_imply_missing_daily() {
        let daily = Daily {
            days: vec![day(10.9, 25.8)],
            today_index: 0,
            today_sunrise: None,
            today_sunset: None,
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

    #[test]
    fn every_design_size_lands_in_its_own_bucket() {
        for bucket in [
            SizeBucket::Full,
            SizeBucket::Large,
            SizeBucket::Medium,
            SizeBucket::Small,
            SizeBucket::Bmm101,
        ] {
            let (width, height) = bucket.design_size();
            assert_eq!(size_bucket(width, height), bucket, "{width}x{height}");
        }
    }

    /// A frame that misses the BMM101 rule by a pixel falls to the SDK's closest variant.
    #[test]
    fn the_bmm101_bucket_ends_at_the_midpoint_of_the_bmm_heights_and_at_its_width() {
        assert_eq!(size_bucket(320, 240), SizeBucket::Small, "BMM100");
        assert_eq!(size_bucket(480, 280), SizeBucket::Bmm101);
        assert_eq!(size_bucket(480, 279), SizeBucket::Medium);
        assert_eq!(size_bucket(481, 320), SizeBucket::Large);
    }
}
