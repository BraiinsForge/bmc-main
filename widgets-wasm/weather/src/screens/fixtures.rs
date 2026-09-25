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

//! Forecasts, states and viewports to stage the layouts at, off-device.

use bmc_wasm_sdk::{SystemTime, ViewportShape, WidgetViewport};
use units::availability::Availability;
use units::units::{Degree, DegreeCelsius, KilometerPerHour};

use crate::manifest_params::{Params, TimeZone};
use crate::model::{
    Current, Daily, DayForecast, HourEntry, Hourly, Location, SizeBucket, State, Weather,
};
use crate::screens::ViewData;

/// One state, drawn into whichever viewport it is handed.
pub type StateFixture = fn(WidgetViewport, Params) -> ViewData;

/// 8 June 2026, 14:00 in Prague — the current hour of the forecast the capture fixtures recorded.
const RECORDED_AT: i64 = 1_780_920_000;
const HOUR_SECS: i64 = 3_600;
const SUNRISE: i64 = 1_780_887_060;
const SUNSET: i64 = 1_780_945_680;

const HOURLY_TEMPERATURES: [f64; 12] = [
    21.4, 22.1, 22.6, 22.0, 20.8, 18.9, 16.7, 15.2, 14.3, 13.6, 13.1, 12.7,
];
const HOURLY_CODES: [i64; 12] = [2, 2, 3, 3, 1, 1, 0, 0, 1, 2, 2, 3];
/// 21:00, where the recorded hours turn to night.
const FIRST_NIGHT_HOUR: usize = 7;

const DAYS: [&str; 8] = [
    "2026-06-08T00:00:00+02:00",
    "2026-06-09T00:00:00+02:00",
    "2026-06-10T00:00:00+02:00",
    "2026-06-11T00:00:00+02:00",
    "2026-06-12T00:00:00+02:00",
    "2026-06-13T00:00:00+02:00",
    "2026-06-14T00:00:00+02:00",
    "2026-06-15T00:00:00+02:00",
];
const DAILY_CODES: [i64; 8] = [2, 3, 61, 63, 80, 1, 0, 2];
const DAILY_MIN: [f64; 8] = [12.5, 13.0, 11.8, 10.2, 11.5, 12.9, 14.1, 13.6];
const DAILY_MAX: [f64; 8] = [24.1, 22.4, 19.6, 18.0, 20.3, 25.0, 27.2, 24.8];

/// Every figure a two-digit negative, the highs included,
/// in either unit: the warmest figure is -27.8 °C, -18 °F.
const COLD_OFFSET_C: f64 = -55.0;
/// Every high three digits in Fahrenheit: the coolest is 38 °C, 100 °F.
const HOT_OFFSET_C: f64 = 20.0;

/// The gallery clock starts at zero, so this reads as 20 minutes elapsed.
const STALE_SINCE: SystemTime = SystemTime {
    unix_secs: -20 * 60,
};

/// The recorded Nexus forecast for Prague under `display_name`,
/// every temperature shifted by `offset_c`.
fn recorded(display_name: &str, offset_c: f64) -> Weather {
    let celsius = |value: f64| DegreeCelsius(value + offset_c);
    let hours = HOURLY_TEMPERATURES
        .into_iter()
        .zip(HOURLY_CODES)
        .enumerate()
        .map(|(index, (temperature, weather_code))| {
            let offset = i64::try_from(index).expect("BUG: twelve hours fit an i64");
            HourEntry {
                at: Some(SystemTime {
                    unix_secs: RECORDED_AT + offset * HOUR_SECS,
                }),
                temperature: celsius(temperature),
                weather_code,
                is_day: index < FIRST_NIGHT_HOUR,
            }
        })
        .collect();
    let days = DAYS
        .iter()
        .zip(DAILY_CODES)
        .zip(DAILY_MIN.into_iter().zip(DAILY_MAX))
        .map(|((time, weather_code), (min, max))| DayForecast {
            time_rfc3339: (*time).to_owned(),
            weather_code,
            min: celsius(min),
            max: celsius(max),
        })
        .collect();
    Weather {
        location: Location {
            display_name: display_name.to_owned(),
            timezone: "Europe/Prague".to_owned(),
        },
        current: Some(Current {
            temperature: celsius(HOURLY_TEMPERATURES[0]),
            weather_code: HOURLY_CODES[0],
            wind_speed: Availability::Available(KilometerPerHour(12.5)),
            wind_direction: Availability::Available(Degree(235.0)),
            is_day: true,
        }),
        hourly: Some(Hourly {
            entries: hours,
            start_index: 0,
        }),
        daily: Some(Daily {
            days,
            today_index: 0,
            today_sunrise: Some(SystemTime { unix_secs: SUNRISE }),
            today_sunset: Some(SystemTime { unix_secs: SUNSET }),
        }),
    }
}

fn prague() -> Weather {
    recorded("Prague, Czech Republic", 0.0)
}

/// The manifest defaults: Prague, timed in its own zone.
#[must_use]
pub fn default_params() -> Params {
    Params {
        location: "Prague".to_owned(),
        time_zone: TimeZone::Location,
    }
}

#[must_use]
pub fn rectangular(width: u32, height: u32) -> WidgetViewport {
    WidgetViewport {
        width,
        height,
        shape: ViewportShape::Rectangular,
    }
}

#[must_use]
pub fn at_bucket(bucket: SizeBucket) -> WidgetViewport {
    let (width, height) = bucket.design_size();
    rectangular(width, height)
}

fn view(viewport: WidgetViewport, params: Params, state: State) -> ViewData {
    ViewData {
        viewport,
        params,
        state,
        stale_since: None,
    }
}

#[must_use]
pub fn healthy(viewport: WidgetViewport, params: Params) -> ViewData {
    view(viewport, params, State::Loaded(prague()))
}

#[must_use]
pub fn loading(viewport: WidgetViewport, params: Params) -> ViewData {
    view(viewport, params, State::Loading)
}

#[must_use]
pub fn failed(viewport: WidgetViewport, params: Params) -> ViewData {
    view(viewport, params, State::Error)
}

#[must_use]
pub fn stale(viewport: WidgetViewport, params: Params) -> ViewData {
    ViewData {
        stale_since: Some(STALE_SINCE),
        ..healthy(viewport, params)
    }
}

#[must_use]
pub fn bad_location(viewport: WidgetViewport, params: Params) -> ViewData {
    view(viewport, params, State::BadLocation)
}

#[must_use]
pub fn no_location(viewport: WidgetViewport, mut params: Params) -> ViewData {
    params.location.clear();
    view(viewport, params, State::Loading)
}

/// The widest negatives the layouts draw.
#[must_use]
pub fn cold(viewport: WidgetViewport, params: Params) -> ViewData {
    let weather = recorded("Prague, Czech Republic", COLD_OFFSET_C);
    view(viewport, params, State::Loaded(weather))
}

/// The widest positives the layouts draw, once in Fahrenheit.
#[must_use]
pub fn hot(viewport: WidgetViewport, params: Params) -> ViewData {
    let weather = recorded("Prague, Czech Republic", HOT_OFFSET_C);
    view(viewport, params, State::Loaded(weather))
}

/// A display name wider than any layout's location line.
pub const LONG_LOCATION: &str =
    "Llanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogoch, Wales, United Kingdom";

#[must_use]
pub fn long_location(viewport: WidgetViewport, params: Params) -> ViewData {
    let weather = recorded(LONG_LOCATION, 0.0);
    view(viewport, params, State::Loaded(weather))
}
