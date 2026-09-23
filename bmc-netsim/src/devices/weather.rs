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

//! Nexus weather profile — a cloud API reached through testbed URL rewriting,
//! never LAN discovery, serving the one envelope the weather widget reads.
//!
//! Every location gets the Prague forecast the widget's capture fixtures recorded,
//! so a testbed run reads as the baselines do.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::blueprint::{EndpointSpec, RequestCtx, ResourceSpec, Response, ResponseSpec};
use crate::http_status::HttpStatus;

/// Any location answers; a scenario plays a location miss with `status: 404`.
const PATH: &str = "/api/v1/data/weather/{location}";

const CURRENT_TEMPERATURE: f64 = 21.4;
const HOURLY_TEMPERATURES: [f64; 12] = [
    21.4, 22.1, 22.6, 22.0, 20.8, 18.9, 16.7, 15.2, 14.3, 13.6, 13.1, 12.7,
];
const DAILY_MIN: [f64; 8] = [12.5, 13.0, 11.8, 10.2, 11.5, 12.9, 14.1, 13.6];
const DAILY_MAX: [f64; 8] = [24.1, 22.4, 19.6, 18.0, 20.3, 25.0, 27.2, 24.8];

/// Scenario controls for the simulated Nexus weather endpoint.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(rename = "WeatherParams")]
pub struct Params {
    /// HTTP status returned after startup, or after `fail_after_secs`; `404` is a location miss.
    pub status: HttpStatus,
    /// Answer 503 until this many seconds of scenario time have elapsed.
    pub warmup_secs: u32,
    /// Answer 200 before this point, then switch to `status`.
    pub fail_after_secs: Option<u32>,
    /// Degrees Celsius added to every temperature in the forecast.
    pub temperature_offset_c: f64,
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        let params = self.clone();
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: None,
            endpoints: vec![EndpointSpec {
                method: "GET".to_owned(),
                path: PATH.to_owned(),
                response: ResponseSpec::computed(move |ctx| {
                    Response::new(params.status_at(ctx), forecast(&params))
                }),
            }],
            sampler: None,
        }
    }

    fn status_at(&self, ctx: &RequestCtx) -> HttpStatus {
        if ctx.t_s < f64::from(self.warmup_secs) {
            return HttpStatus::SERVICE_UNAVAILABLE;
        }
        if self
            .fail_after_secs
            .is_some_and(|after| ctx.t_s < f64::from(after))
        {
            HttpStatus::OK
        } else {
            self.status
        }
    }

    /// Rounded back to the recorded one decimal, dropping the sum's float noise.
    fn shifted(&self, celsius: f64) -> f64 {
        ((celsius + self.temperature_offset_c) * 10.0).round() / 10.0
    }

    fn all_shifted(&self, temperatures: &[f64]) -> Vec<f64> {
        temperatures.iter().map(|&t| self.shifted(t)).collect()
    }
}

fn forecast(params: &Params) -> Json {
    json!({
        "data": {
            "location": {
                "display_name": "Prague, Czech Republic",
                "timezone": "Europe/Prague",
            },
            "current": {
                "time": "2026-06-08T14:00:00+02:00",
                "temperature": params.shifted(CURRENT_TEMPERATURE),
                "weather_code": 2,
                "wind_speed": 12.5,
                "wind_direction_degrees": 235,
            },
            "hourly": {
                "time": [
                    "2026-06-08T14:00:00+02:00",
                    "2026-06-08T15:00:00+02:00",
                    "2026-06-08T16:00:00+02:00",
                    "2026-06-08T17:00:00+02:00",
                    "2026-06-08T18:00:00+02:00",
                    "2026-06-08T19:00:00+02:00",
                    "2026-06-08T20:00:00+02:00",
                    "2026-06-08T21:00:00+02:00",
                    "2026-06-08T22:00:00+02:00",
                    "2026-06-08T23:00:00+02:00",
                    "2026-06-09T00:00:00+02:00",
                    "2026-06-09T01:00:00+02:00",
                ],
                "temperature": params.all_shifted(&HOURLY_TEMPERATURES),
                "weather_code": [2, 2, 3, 3, 1, 1, 0, 0, 1, 2, 2, 3],
                "is_day": [
                    true, true, true, true, true, true, true, false, false, false, false, false,
                ],
            },
            "daily": {
                "time": [
                    "2026-06-08T00:00:00+02:00",
                    "2026-06-09T00:00:00+02:00",
                    "2026-06-10T00:00:00+02:00",
                    "2026-06-11T00:00:00+02:00",
                    "2026-06-12T00:00:00+02:00",
                    "2026-06-13T00:00:00+02:00",
                    "2026-06-14T00:00:00+02:00",
                    "2026-06-15T00:00:00+02:00",
                ],
                "weather_code": [2, 3, 61, 63, 80, 1, 0, 2],
                "temperature_min": params.all_shifted(&DAILY_MIN),
                "temperature_max": params.all_shifted(&DAILY_MAX),
                "today": {
                    "index": 0,
                    "sunrise": "2026-06-08T04:51:00+02:00",
                    "sunset": "2026-06-08T21:08:00+02:00",
                },
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::Instant;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    use super::*;
    use crate::cache::Cache;

    fn ctx(t_s: f64) -> RequestCtx {
        RequestCtx {
            query: BTreeMap::new(),
            t_s,
            seed: 1,
            host: None,
            cache: Arc::new(Cache::new::<Vec<_>>(Vec::new())),
        }
    }

    #[tokio::test]
    async fn the_route_answers_for_any_location() {
        let resource = Params::default().resource("weather", 20_500);
        let cache = Arc::new(Cache::new::<Vec<_>>(Vec::new()));
        let router = crate::respond::build_router(resource.endpoints, 1, Instant::now(), &cache)
            .expect("BUG: the weather router builds");
        let request = Request::get("/api/v1/data/weather/New%20York")
            .body(Body::empty())
            .expect("BUG: a GET with an empty body builds");
        let response = router
            .oneshot(request)
            .await
            .expect("BUG: the router responds");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("BUG: the body reads");
        let json: Json = serde_json::from_slice(&body).expect("BUG: the body is JSON");
        assert_eq!(
            json["data"]["location"]["display_name"],
            "Prague, Czech Republic"
        );
    }

    #[test]
    fn the_default_forecast_is_the_recorded_one() {
        let data = &forecast(&Params::default())["data"];
        assert_eq!(data["current"]["temperature"], 21.4);
        assert_eq!(data["hourly"]["temperature"][3], 22.0);
        assert_eq!(data["daily"]["temperature_min"][2], 11.8);
        assert_eq!(data["daily"]["temperature_max"][6], 27.2);
        assert_eq!(
            data["daily"]["today"]["sunset"],
            "2026-06-08T21:08:00+02:00"
        );
    }

    #[test]
    fn an_offset_shifts_every_temperature() {
        let params = Params {
            temperature_offset_c: -25.0,
            ..Params::default()
        };
        let data = &forecast(&params)["data"];
        assert_eq!(data["current"]["temperature"], -3.6);
        assert_eq!(data["hourly"]["temperature"][11], -12.3);
        assert_eq!(data["daily"]["temperature_min"][3], -14.8);
        assert_eq!(data["daily"]["temperature_max"][6], 2.2);
    }

    #[test]
    fn warmup_and_failure_transitions_are_time_driven() {
        let warming = Params {
            warmup_secs: 60,
            ..Params::default()
        };
        assert_eq!(
            warming.status_at(&ctx(59.0)),
            HttpStatus::SERVICE_UNAVAILABLE
        );
        assert_eq!(warming.status_at(&ctx(60.0)), HttpStatus::OK);

        let failing = Params {
            status: HttpStatus::SERVICE_UNAVAILABLE,
            fail_after_secs: Some(60),
            ..Params::default()
        };
        assert_eq!(failing.status_at(&ctx(59.0)), HttpStatus::OK);
        assert_eq!(
            failing.status_at(&ctx(60.0)),
            HttpStatus::SERVICE_UNAVAILABLE
        );
    }
}
