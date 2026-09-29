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

//! Nexus SpaceX next-launch profile — a cloud API reached through testbed URL rewriting,
//! never LAN discovery. Serves the one envelope the SpaceX launch widget reads.
//!
//! Every read carries one launch, by default the NROL-179 the widget's capture fixtures recorded.
//! Its `net` is pinned to the scenario's start rather than re-derived per read,
//! so the countdown keeps falling across polls and can run on past T-0.

use std::sync::OnceLock;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::blueprint::{EndpointSpec, RequestCtx, ResourceSpec, Response, ResponseSpec};
use crate::http_status::HttpStatus;

const PATH: &str = "/api/v1/data/spacex/next-launch";
const TTL_SECS: u64 = 300;

/// Scenario controls for the simulated Nexus next-launch endpoint.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(rename = "SpacexLaunchParams")]
pub struct Params {
    /// HTTP status returned after startup, or after `fail_after_secs`.
    pub status: HttpStatus,
    /// Answer 503 until this many seconds of scenario time have elapsed.
    pub warmup_secs: u32,
    /// Answer 200 before this point, then switch to `status`.
    pub fail_after_secs: Option<u32>,
    /// Seconds from scenario start to the launch's `net`.
    pub launch_in_secs: i64,
    /// Which launch every read serves.
    pub launch: Launch,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            status: HttpStatus::OK,
            warmup_secs: 0,
            fail_after_secs: None,
            // How far ahead the launch stood when the capture fixtures were recorded.
            launch_in_secs: 73_719,
            launch: Launch::default(),
        }
    }
}

/// The launches a scenario can serve, each staging rows the others leave alone.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(rename = "SpacexLaunchKind")]
pub enum Launch {
    /// NROL-179 from Vandenberg, as the widget's capture fixtures recorded it.
    #[default]
    #[serde(rename = "nrol-179")]
    Nrol179,
    /// A Starlink batch on Starship from Starbase: a long mission name, the pad, a ship
    /// and an ocean landing.
    Starship,
    /// A crewed Dragon from Kennedy: a named capsule and a human exploration payload.
    CrewDragon,
    /// A rideshare whose mission name needs two lines, with its landing not yet confirmed.
    Rideshare,
}

impl Launch {
    /// The launch's `data` block, all but its `net`.
    fn data(self) -> Json {
        match self {
            Launch::Nrol179 => json!({
                "name": "Falcon 9 Block 5 | NROL-179",
                "status": { "name": "Go for Launch" },
                "mission": { "name": "NROL-179", "type": "Government/Top Secret" },
                "rocket": {
                    "configuration": { "full_name": "Falcon 9 Block 5", "name": "Falcon 9" },
                    "launcher_stage": [{
                        "landing": { "attempt": true, "type": { "abbrev": "RTLS" } },
                        "launcher_flight_number": 3,
                    }],
                    "spacecraft_stage": [],
                },
                "pad": {
                    "name": "Space Launch Complex 4E",
                    "location": { "name": "Vandenberg SFB, CA, USA" },
                },
            }),
            Launch::Starship => json!({
                "name": "Starship | Starlink Group 31-1 (Starship Flight 14)",
                "status": { "name": "Go for Launch" },
                "mission": {
                    "name": "Starlink Group 31-1 (Starship Flight 14)",
                    "type": "Communications",
                },
                "rocket": {
                    "configuration": { "full_name": "Starship V3", "name": "Starship" },
                    "launcher_stage": [{
                        "landing": { "attempt": true, "type": { "abbrev": "Ocean" } },
                        "launcher_flight_number": 1,
                    }],
                    "spacecraft_stage": [{ "spacecraft": { "name": "Ship 41" } }],
                },
                "pad": {
                    "name": "Orbital Launch Pad 2",
                    "location": { "name": "SpaceX Starbase, TX, USA" },
                },
            }),
            Launch::CrewDragon => json!({
                "name": "Falcon 9 Block 5 | Crew-12",
                "status": { "name": "Go for Launch" },
                "mission": { "name": "Crew-12", "type": "Human Exploration" },
                "rocket": {
                    "configuration": { "full_name": "Falcon 9 Block 5", "name": "Falcon 9" },
                    "launcher_stage": [{
                        "landing": { "attempt": true, "type": { "abbrev": "ASDS" } },
                        "launcher_flight_number": 7,
                    }],
                    "spacecraft_stage": [{ "spacecraft": { "name": "Crew Dragon Endeavour" } }],
                },
                "pad": {
                    "name": "Launch Complex 39A",
                    "location": { "name": "Kennedy Space Center, FL, USA" },
                },
            }),
            Launch::Rideshare => json!({
                "name": "Falcon 9 Block 5 | Transporter 17 (Dedicated SSO Rideshare)",
                "status": { "name": "To Be Confirmed" },
                "mission": {
                    "name": "Transporter 17 (Dedicated SSO Rideshare)",
                    "type": "Dedicated Rideshare",
                },
                "rocket": {
                    "configuration": { "full_name": "Falcon 9 Block 5", "name": "Falcon 9" },
                    "launcher_stage": [{ "landing": null, "launcher_flight_number": 21 }],
                    "spacecraft_stage": [],
                },
                "pad": {
                    "name": "Space Launch Complex 4E",
                    "location": { "name": "Vandenberg SFB, CA, USA" },
                },
            }),
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        let params = self.clone();
        // Taken once: each read's wall clock and scenario clock disagree by a few
        // milliseconds, enough to tip a whole-second `net` across a boundary.
        let start = OnceLock::new();
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: None,
            endpoints: vec![EndpointSpec {
                method: "GET".to_owned(),
                path: PATH.to_owned(),
                response: ResponseSpec::computed(move |ctx| {
                    let start = *start.get_or_init(|| scenario_start(Utc::now(), ctx.t_s));
                    Response::new(params.status_at(ctx), launch(&params, start))
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
}

/// When the scenario began, as read `elapsed_secs` into it at wall-clock `now`.
fn scenario_start(now: DateTime<Utc>, elapsed_secs: f64) -> DateTime<Utc> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "scenario time in whole milliseconds fits i64"
    )]
    let elapsed = Duration::milliseconds((elapsed_secs * 1_000.0) as i64);
    now - elapsed
}

/// The envelope of a scenario that began at `start`.
fn launch(params: &Params, start: DateTime<Utc>) -> Json {
    let net = start + Duration::seconds(params.launch_in_secs);
    let mut data = params.launch.data();
    data["net"] = json!(net.to_rfc3339_opts(SecondsFormat::Secs, true));
    json!({
        "resource": "spacex/next-launch",
        "data": data,
        "cache_age_secs": 0,
        "ttl_secs": TTL_SECS,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::blueprint::ResponseData;

    fn ctx(t_s: f64) -> RequestCtx {
        RequestCtx {
            query: BTreeMap::new(),
            t_s,
            seed: 1,
            host: None,
            cache: Arc::new(crate::cache::Cache::new::<Vec<_>>(Vec::new())),
        }
    }

    /// When the capture fixtures were recorded.
    fn start() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-06-18T12:11:21Z")
            .expect("BUG: a fixed RFC3339 instant")
            .with_timezone(&Utc)
    }

    #[test]
    fn resource_exposes_only_the_next_launch_route() {
        let resource = Params::default().resource("spacex", 20_600);
        let paths: Vec<_> = resource
            .endpoints
            .iter()
            .map(|endpoint| endpoint.path.as_str())
            .collect();
        assert_eq!(paths, [PATH]);
    }

    #[test]
    fn the_default_envelope_replays_the_recorded_launch() {
        let response = launch(&Params::default(), start());
        assert_eq!(response["resource"], "spacex/next-launch");
        assert_eq!(response["ttl_secs"], TTL_SECS);
        assert_eq!(response["data"]["net"], "2026-06-19T08:40:00Z");
        assert_eq!(response["data"]["mission"]["name"], "NROL-179");
        assert_eq!(
            response["data"]["rocket"]["launcher_stage"][0]["launcher_flight_number"],
            3
        );
    }

    #[test]
    fn every_launch_carries_its_mission_at_the_pinned_net() {
        for kind in [
            Launch::Nrol179,
            Launch::Starship,
            Launch::CrewDragon,
            Launch::Rideshare,
        ] {
            let params = Params {
                launch: kind,
                ..Params::default()
            };
            let response = launch(&params, start());
            assert_eq!(response["data"]["net"], "2026-06-19T08:40:00Z", "{kind:?}");
            assert!(response["data"]["mission"]["name"].is_string(), "{kind:?}");
        }
    }

    /// Re-deriving the instant per read would shift the widget's countdown between polls.
    #[test]
    fn the_served_net_holds_across_reads() {
        let resource = Params::default().resource("spacex", 20_600);
        let [endpoint] = resource.endpoints.as_slice() else {
            panic!("BUG: the profile serves one route");
        };
        let ResponseSpec::Computed(respond) = &endpoint.response else {
            panic!("BUG: the next-launch route is computed per read");
        };
        let net = |t_s| {
            let ResponseData::Json(body) = respond(&ctx(t_s)).data else {
                panic!("BUG: Nexus answers JSON");
            };
            body["data"]["net"].clone()
        };
        assert_eq!(net(0.0), net(1_800.5));
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
