// Copyright (C) 2026  Braiins Systems s.r.o.

#[cfg(target_arch = "wasm32")]
use bmc_wasm_sdk::JsonDoc;

pub struct Location {
    pub display_name: String,
    pub timezone: String,
}

pub struct Current {
    pub temperature_c: f64,
    pub weather_code: i64,
    pub wind_speed_kmh: Option<f64>,
    pub wind_dir_deg: Option<f64>,
}

pub struct HourEntry {
    pub time_rfc3339: String,
    pub temperature_c: f64,
    pub weather_code: i64,
    pub is_day: bool,
}

pub struct Hourly {
    pub entries: Vec<HourEntry>,
}

pub struct DayForecast {
    pub time_rfc3339: String,
    pub weather_code: i64,
    pub min_c: f64,
    pub max_c: f64,
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

pub struct ForecastRange {
    pub min_c: f64,
    pub max_c: f64,
}

impl ForecastRange {
    #[must_use]
    pub fn of(days: &[DayForecast]) -> ForecastRange {
        let Some(first) = days.first() else {
            return ForecastRange {
                min_c: 0.0,
                max_c: 0.0,
            };
        };
        let mut min_c = first.min_c;
        let mut max_c = first.max_c;
        for day in days.iter().skip(1) {
            if day.min_c < min_c {
                min_c = day.min_c;
            }
            if day.max_c > max_c {
                max_c = day.max_c;
            }
        }
        ForecastRange { min_c, max_c }
    }

    #[must_use]
    pub fn fraction(&self, value_c: f64) -> f64 {
        let span = self.max_c - self.min_c;
        if span <= 0.0 {
            return 0.0;
        }
        ((value_c - self.min_c) / span).clamp(0.0, 1.0)
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

        let current = parse_current(doc);
        let hourly = parse_hourly(doc);
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
fn parse_current(doc: &JsonDoc) -> Option<Current> {
    let temperature_c = doc.f64("/data/current/temperature")?;
    let weather_code = doc.i64("/data/current/weather_code")?;
    Some(Current {
        temperature_c,
        weather_code,
        wind_speed_kmh: doc.f64("/data/current/wind_speed"),
        wind_dir_deg: doc.f64("/data/current/wind_direction_degrees"),
    })
}

#[cfg(target_arch = "wasm32")]
fn parse_hourly(doc: &JsonDoc) -> Option<Hourly> {
    use bmc_wasm_sdk::ufmt;
    let mut entries = Vec::new();
    for i in 0..256_usize {
        let time = doc.str(&bmc_wasm_sdk::fmt!("/data/hourly/time/{}", i));
        let temperature_c = doc.f64(&bmc_wasm_sdk::fmt!("/data/hourly/temperature/{}", i));
        let weather_code = doc.i64(&bmc_wasm_sdk::fmt!("/data/hourly/weather_code/{}", i));
        let is_day = doc.bool(&bmc_wasm_sdk::fmt!("/data/hourly/is_day/{}", i));
        match (time, temperature_c, weather_code, is_day) {
            (Some(time_rfc3339), Some(temperature_c), Some(weather_code), Some(is_day)) => {
                entries.push(HourEntry {
                    time_rfc3339,
                    temperature_c,
                    weather_code,
                    is_day,
                });
            }
            _ => break,
        }
    }
    if entries.is_empty() {
        None
    } else {
        Some(Hourly { entries })
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_daily(doc: &JsonDoc) -> Option<Daily> {
    use bmc_wasm_sdk::ufmt;
    let mut days = Vec::new();
    for i in 0..256_usize {
        let time = doc.str(&bmc_wasm_sdk::fmt!("/data/daily/time/{}", i));
        let weather_code = doc.i64(&bmc_wasm_sdk::fmt!("/data/daily/weather_code/{}", i));
        let min_c = doc.f64(&bmc_wasm_sdk::fmt!("/data/daily/temperature_min/{}", i));
        let max_c = doc.f64(&bmc_wasm_sdk::fmt!("/data/daily/temperature_max/{}", i));
        let sunrise = doc.str(&bmc_wasm_sdk::fmt!("/data/daily/sunrise/{}", i));
        let sunset = doc.str(&bmc_wasm_sdk::fmt!("/data/daily/sunset/{}", i));
        match (time, weather_code, min_c, max_c, sunrise, sunset) {
            (
                Some(time_rfc3339),
                Some(weather_code),
                Some(min_c),
                Some(max_c),
                Some(sunrise),
                Some(sunset),
            ) => {
                days.push(DayForecast {
                    time_rfc3339,
                    weather_code,
                    min_c,
                    max_c,
                    sunrise,
                    sunset,
                });
            }
            _ => break,
        }
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

    fn day(min_c: f64, max_c: f64) -> DayForecast {
        DayForecast {
            time_rfc3339: "2026-06-03T00:00:00+02:00".to_string(),
            weather_code: 3,
            min_c,
            max_c,
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
        assert!((range.min_c - 10.9).abs() < 1e-9);
        assert!((range.max_c - 25.8).abs() < 1e-9);
    }

    #[test]
    fn day_fraction_positions_a_value_within_the_global_range() {
        let range = ForecastRange {
            min_c: 10.0,
            max_c: 30.0,
        };
        assert!((range.fraction(20.0) - 0.5).abs() < 1e-9);
        assert!((range.fraction(10.0) - 0.0).abs() < 1e-9);
        assert!((range.fraction(30.0) - 1.0).abs() < 1e-9);
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
