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

//! Nexus halving prediction profile — a cloud API reached through testbed URL rewriting,
//! never LAN discovery. Serves the one envelope the halving countdown widget reads.
//!
//! The prediction is pinned to the scenario's start rather than re-derived per read,
//! so a countdown keeps falling across polls, and the tip gains a block every ten minutes.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::blueprint::{EndpointSpec, RequestCtx, ResourceSpec, Response, ResponseSpec};
use crate::http_status::HttpStatus;

const PATH: &str = "/api/v1/data/bitcoin/halving-prediction";
const TTL_SECS: u64 = 60;
const HALVING_INTERVAL: u64 = 210_000;
const BLOCK_SECS: i64 = 600;

/// Scenario controls and the prediction returned by the simulated Nexus endpoint.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(rename = "HalvingCountdownParams")]
pub struct Params {
    /// HTTP status returned after startup, or after `fail_after_secs`.
    pub status: HttpStatus,
    /// Answer 503 until this many seconds of scenario time have elapsed.
    pub warmup_secs: u32,
    /// Answer 200 before this point, then switch to `status`.
    pub fail_after_secs: Option<u32>,
    /// Cache age advertised in the Nexus response envelope.
    pub cache_age_secs: u64,
    /// Bitcoin block height at scenario start; the halving is the next multiple of 210 000.
    pub block_height: u64,
    /// Seconds from scenario start to the predicted halving.
    pub halving_in_secs: i64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            status: HttpStatus::OK,
            warmup_secs: 0,
            fail_after_secs: None,
            cache_age_secs: 0,
            block_height: 959_131,
            halving_in_secs: 54_521_099,
        }
    }
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
                    Response::new(
                        params.status_at(ctx),
                        prediction(&params, Utc::now(), ctx.t_s),
                    )
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

fn point(height: u64, at: DateTime<Utc>, now: DateTime<Utc>) -> Json {
    json!({
        "block_height": height,
        "delta": (at - now).num_seconds(),
        "timestamp": at.to_rfc3339_opts(SecondsFormat::Secs, true),
    })
}

/// The envelope as read `elapsed_secs` into the scenario, at wall-clock `now`.
fn prediction(params: &Params, now: DateTime<Utc>, elapsed_secs: f64) -> Json {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "scenario time in whole milliseconds fits i64"
    )]
    let elapsed = Duration::milliseconds((elapsed_secs * 1_000.0) as i64);
    let mined = u64::try_from(elapsed.num_seconds().div_euclid(BLOCK_SECS)).unwrap_or(0);
    let height = params.block_height + mined;
    let last_height = params.block_height.div_euclid(HALVING_INTERVAL) * HALVING_INTERVAL;
    let next_height = last_height + HALVING_INTERVAL;
    let blocks_since_last = i64::try_from(height - last_height).unwrap_or(i64::MAX);
    json!({
        "resource": "bitcoin/halving-prediction",
        "data": {
            "current": point(height, now, now),
            "last": point(last_height, now - Duration::seconds(blocks_since_last * BLOCK_SECS), now),
            "next": point(next_height, now - elapsed + Duration::seconds(params.halving_in_secs), now),
        },
        "cache_age_secs": params.cache_age_secs,
        "ttl_secs": TTL_SECS,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;

    fn ctx(t_s: f64) -> RequestCtx {
        RequestCtx {
            query: BTreeMap::new(),
            t_s,
            seed: 1,
            host: None,
            cache: Arc::new(crate::cache::Cache::new::<Vec<_>>(Vec::new())),
        }
    }

    fn start() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-22T13:09:35Z")
            .expect("BUG: a fixed RFC3339 instant")
            .with_timezone(&Utc)
    }

    #[test]
    fn resource_exposes_only_the_prediction_route() {
        let resource = Params::default().resource("halving", 20_400);
        let paths: Vec<_> = resource
            .endpoints
            .iter()
            .map(|endpoint| endpoint.path.as_str())
            .collect();
        assert_eq!(paths, [PATH]);
    }

    #[test]
    fn the_envelope_carries_the_nexus_cadence_and_the_next_halving() {
        let params = Params {
            cache_age_secs: 57,
            ..Params::default()
        };
        let response = prediction(&params, start(), 0.0);
        assert_eq!(response["resource"], "bitcoin/halving-prediction");
        assert_eq!(response["ttl_secs"], TTL_SECS);
        assert_eq!(response["cache_age_secs"], 57);
        assert_eq!(response["data"]["current"]["block_height"], 959_131);
        assert_eq!(response["data"]["last"]["block_height"], 840_000);
        assert_eq!(response["data"]["next"]["block_height"], 1_050_000);
        assert_eq!(
            response["data"]["next"]["timestamp"],
            "2028-04-13T13:54:34Z"
        );
        assert_eq!(response["data"]["next"]["delta"], 54_521_099);
    }

    /// Re-deriving the instant per read would reset the widget's countdown every poll.
    #[test]
    fn the_predicted_instant_holds_while_the_tip_advances() {
        let params = Params::default();
        let later = start() + Duration::seconds(1_800);
        let first = prediction(&params, start(), 0.0);
        let second = prediction(&params, later, 1_800.0);
        assert_eq!(
            first["data"]["next"]["timestamp"],
            second["data"]["next"]["timestamp"]
        );
        assert_eq!(second["data"]["next"]["delta"], 54_521_099 - 1_800);
        assert_eq!(second["data"]["current"]["block_height"], 959_131 + 3);
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
