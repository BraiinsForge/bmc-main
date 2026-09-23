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

//! Nexus's weather endpoint: the URL for a location, what a reply's status means,
//! and the envelope read into a [`crate::model::Weather`].

pub const NEXUS_BASE: &str = "https://nexus.braiinsforge.com/api/v1/data/weather/";

#[must_use]
pub fn weather_url(base: &str, location: &str) -> String {
    let mut out = String::from(base);
    for byte in location.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            other => push_percent_encoded(&mut out, other),
        }
    }
    out
}

fn push_percent_encoded(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(HEX[(byte >> 4) as usize] as char);
    out.push(HEX[(byte & 0x0f) as usize] as char);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherFetchAction {
    BadLocation,
    ReadPayload,
    TransientFailure,
}

#[must_use]
pub fn weather_fetch_action(status: u32) -> WeatherFetchAction {
    match status {
        404 => WeatherFetchAction::BadLocation,
        200..=299 => WeatherFetchAction::ReadPayload,
        _ => WeatherFetchAction::TransientFailure,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchOutcome {
    Store,
    Keep,
    Fail,
    BadLocation,
}

#[must_use]
pub fn fetch_outcome(action: WeatherFetchAction, parsed_ok: bool, has_data: bool) -> FetchOutcome {
    match action {
        WeatherFetchAction::BadLocation => FetchOutcome::BadLocation,
        WeatherFetchAction::ReadPayload if parsed_ok => FetchOutcome::Store,
        WeatherFetchAction::ReadPayload | WeatherFetchAction::TransientFailure => {
            if has_data {
                FetchOutcome::Keep
            } else {
                FetchOutcome::Fail
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use payload::WeatherParseError;

/// Wasm-only: `JsonDoc` and `parse_datetime` are host calls.
#[cfg(target_arch = "wasm32")]
mod payload {
    use bmc_wasm_sdk::{JsonDoc, SystemTime, parse_datetime, ufmt};
    use units::units::{Degree, DegreeCelsius, KilometerPerHour};

    use crate::model::{
        Current, Daily, DayForecast, HourEntry, Hourly, Location, Weather, current_is_day,
        hourly_start_index,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum WeatherParseError {
        InvalidDocument,
        MissingRequiredField(&'static str),
    }

    fn instant(rfc3339: &str) -> Option<SystemTime> {
        parse_datetime(rfc3339).map(|unix_secs| SystemTime { unix_secs })
    }

    impl TryFrom<&JsonDoc> for Weather {
        type Error = WeatherParseError;

        fn try_from(doc: &JsonDoc) -> Result<Self, Self::Error> {
            if !doc.is_valid() {
                return Err(WeatherParseError::InvalidDocument);
            }
            let display_name = doc.str("/data/location/display_name").ok_or(
                WeatherParseError::MissingRequiredField("/data/location/display_name"),
            )?;
            let timezone = doc.str("/data/location/timezone").ok_or(
                WeatherParseError::MissingRequiredField("/data/location/timezone"),
            )?;
            let location = Location {
                display_name,
                timezone,
            };

            let now = doc
                .str("/data/current/time")
                .and_then(|time| instant(&time));
            let mut hourly = parse_hourly(doc);
            if let Some(h) = hourly.as_mut() {
                h.start_index = hourly_start_index(&h.entries, now);
            }
            let current = parse_current(doc, hourly.as_ref(), now);
            let daily = parse_daily(doc);

            Ok(Weather {
                location,
                current,
                hourly,
                daily,
            })
        }
    }

    fn parse_current(
        doc: &JsonDoc,
        hourly: Option<&Hourly>,
        now: Option<SystemTime>,
    ) -> Option<Current> {
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
            is_day: current_is_day(hourly, now),
        })
    }

    fn parse_hourly(doc: &JsonDoc) -> Option<Hourly> {
        let mut entries = Vec::new();
        for i in 0..256_usize {
            let Some(time) = doc.str(&bmc_wasm_sdk::fmt!("/data/hourly/time/{}", i)) else {
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
                at: instant(&time),
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

    fn parse_daily(doc: &JsonDoc) -> Option<Daily> {
        let mut days = Vec::new();
        for i in 0..256_usize {
            let Some(time_rfc3339) = doc.str(&bmc_wasm_sdk::fmt!("/data/daily/time/{}", i)) else {
                break;
            };
            let weather_code = doc.i64(&bmc_wasm_sdk::fmt!("/data/daily/weather_code/{}", i));
            let min_c = doc.f64(&bmc_wasm_sdk::fmt!("/data/daily/temperature_min/{}", i));
            let max_c = doc.f64(&bmc_wasm_sdk::fmt!("/data/daily/temperature_max/{}", i));
            let (Some(weather_code), Some(min_c), Some(max_c)) = (weather_code, min_c, max_c)
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
        Some(Daily {
            days,
            today_index,
            today_sunrise: doc
                .str("/data/daily/today/sunrise")
                .and_then(|at| instant(&at)),
            today_sunset: doc
                .str("/data/daily/today/sunset")
                .and_then(|at| instant(&at)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_become_percent_twenty() {
        let url = weather_url("https://example/api/weather/", "New York");
        assert_eq!(url, "https://example/api/weather/New%20York");
    }

    #[test]
    fn unreserved_ascii_passes_through() {
        assert_eq!(weather_url("b/", "Prague"), "b/Prague");
    }

    #[test]
    fn multibyte_umlaut_is_encoded_per_utf8_byte() {
        assert_eq!(weather_url("b/", "Zürich"), "b/Z%C3%BCrich");
    }

    #[test]
    fn multibyte_and_space_are_both_encoded() {
        assert_eq!(weather_url("b/", "São Paulo"), "b/S%C3%A3o%20Paulo");
    }

    #[test]
    fn location_miss_disables_weather_poll_until_params_change() {
        assert_eq!(weather_fetch_action(404), WeatherFetchAction::BadLocation);
    }

    #[test]
    fn a_404_is_a_bad_location_regardless_of_held_data() {
        assert_eq!(
            fetch_outcome(WeatherFetchAction::BadLocation, false, false),
            FetchOutcome::BadLocation
        );
        assert_eq!(
            fetch_outcome(WeatherFetchAction::BadLocation, false, true),
            FetchOutcome::BadLocation
        );
    }

    #[test]
    fn a_parsed_payload_replaces_the_data() {
        assert_eq!(
            fetch_outcome(WeatherFetchAction::ReadPayload, true, false),
            FetchOutcome::Store
        );
        assert_eq!(
            fetch_outcome(WeatherFetchAction::ReadPayload, true, true),
            FetchOutcome::Store
        );
    }

    #[test]
    fn a_failure_keeps_data_when_present_else_errors() {
        // Held data survives a failed refresh (and goes stale); with nothing
        // loaded yet the same failure is a hard error.
        for action in [
            WeatherFetchAction::ReadPayload,
            WeatherFetchAction::TransientFailure,
        ] {
            assert_eq!(fetch_outcome(action, false, true), FetchOutcome::Keep);
            assert_eq!(fetch_outcome(action, false, false), FetchOutcome::Fail);
        }
    }
}
