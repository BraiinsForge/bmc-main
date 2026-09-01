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

//! `public-api.braiins.com` — the Bitcoin-network figures
//! the miner widgets read alongside a miner's own telemetry.
//!
//! A cloud API on its port, never announced,
//! so a scenario reaches it with `--rewrite-url`.
//!
//! Shape follows what `widgets-wasm/lib/miner-info/src/public.rs` parses;
//! no public spec URL to link yet.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::blueprint::{EndpointSpec, RequestCtx, ResourceSpec, Response, ResponseSpec};
use crate::http_status::HttpStatus;

const PRICE_STATS_PATH: &str = "/v1/price-stats";
const BLOCKS_PATH: &str = "/v2/blocks";
const DIFFICULTY_PATH: &str = "/v1/difficulty-stats";
const HASHRATE_PATH: &str = "/v2/hashrate-stats";
const PRICE_HISTORY_PATH: &str = "/v1/price-history";

/// Points the 1d sparkline is drawn from.
/// The widget reads until the first gap and caps itself
/// well above this, so the series length is ours to choose.
const HISTORY_POINTS: usize = 64;

// Scenario controls and values returned by the simulated public API.
// Strict, unlike persisted state: a blueprint is hand-authored, so a mistyped
// key is a fault that would silently never fire rather than a forward-compatible
// field to skip over.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(rename = "BraiinsPublicApiParams")]
pub struct Params {
    /// HTTP status returned after startup, or after `fail_after_secs`.
    pub status: HttpStatus,
    /// Answer 200 before this many seconds of scenario time,
    /// then switch to `status`.
    /// Drives the stale case: data lands, then stops refreshing.
    pub fail_after_secs: Option<u32>,
    /// Bitcoin price. The `currency` query is not inspected,
    /// so this one value answers whichever currency is asked for.
    pub price: f64,
    /// Price move over 24h, as a percent; negative reads red.
    pub percent_change_24h: f64,
    /// Latest block height.
    pub block_height: u64,
    /// Realised difficulty adjustment, as a fraction; `0.01` means 1%.
    pub previous_adjustment: f64,
    /// Projected difficulty adjustment, as a fraction.
    pub estimated_adjustment: f64,
    /// Blocks mined in the current 2016-block epoch.
    pub block_epoch: f64,
    /// Network hashrate, in EH/s.
    pub current_hashrate: f64,
    /// Average transaction fees per block, in BTC.
    pub avg_fees_per_block: f64,
    /// Fees as a percent of the block reward.
    pub fees_percent: f64,
    /// Hashprice, in the queried currency per TH/day.
    pub hash_price_currency: f64,
    /// Hashvalue, in BTC per TH/day.
    pub hash_value: f64,
    /// Price at the start of the 1d history series, which ends at `price`.
    /// Above `price` the sparkline falls, below it climbs.
    pub history_open: f64,
    /// Points in the history series. One point draws a widget with a series
    /// too short to have a shape, which the chart still has to survive.
    pub history_points: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            status: HttpStatus::OK,
            fail_after_secs: None,
            price: 101_754.0,
            percent_change_24h: 6.25,
            block_height: 880_123,
            previous_adjustment: -0.021,
            estimated_adjustment: -0.045,
            block_epoch: 1_754.0,
            current_hashrate: 650.0,
            avg_fees_per_block: 0.055,
            fees_percent: 12.1,
            hash_price_currency: 0.052,
            hash_value: 0.000_000_7,
            history_open: 99_800.0,
            history_points: HISTORY_POINTS,
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: None,
            endpoints: vec![
                self.endpoint(PRICE_STATS_PATH, Self::price_stats),
                self.endpoint(BLOCKS_PATH, Self::blocks),
                self.endpoint(DIFFICULTY_PATH, Self::difficulty_stats),
                self.endpoint(HASHRATE_PATH, Self::hashrate_stats),
                self.endpoint(PRICE_HISTORY_PATH, Self::price_history),
            ],
            sampler: None,
        }
    }

    fn endpoint(&self, path: &str, body: fn(&Self) -> Json) -> EndpointSpec {
        let params = self.clone();
        EndpointSpec {
            method: "GET".to_owned(),
            path: path.to_owned(),
            response: ResponseSpec::computed(move |ctx: &RequestCtx| {
                Response::new(params.status_at(ctx), body(&params))
            }),
        }
    }

    /// `status` throughout, unless `fail_after_secs` holds the endpoint healthy
    /// for an opening stretch first.
    fn status_at(&self, ctx: &RequestCtx) -> HttpStatus {
        if self
            .fail_after_secs
            .is_some_and(|after| ctx.t_s < f64::from(after))
        {
            HttpStatus::OK
        } else {
            self.status
        }
    }

    fn price_stats(&self) -> Json {
        json!({
            "price": self.price,
            "percent_change_24h": self.percent_change_24h,
        })
    }

    /// The widget asks for `limit=1` and reads the head, so the array carries
    /// exactly the one block it looks at.
    fn blocks(&self) -> Json {
        json!([{ "height": self.block_height }])
    }

    fn difficulty_stats(&self) -> Json {
        json!({
            "previous_adjustment": self.previous_adjustment,
            "estimated_adjustment": self.estimated_adjustment,
            "block_epoch": self.block_epoch,
        })
    }

    fn hashrate_stats(&self) -> Json {
        json!({
            "current_hashrate": self.current_hashrate,
            "avg_fees_per_block": self.avg_fees_per_block,
            "fees_percent": self.fees_percent,
            "hash_price_currency": self.hash_price_currency,
            "hash_value": self.hash_value,
        })
    }

    /// A straight line from `history_open` to `price`,
    /// which is enough for a sparkline:
    /// the widget normalises the series and only its shape shows.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a sample index over a series this short is exact in f64"
    )]
    fn price_history(&self) -> Json {
        // A one-point series has no span to divide by, and still has to render.
        let span = self.history_points.saturating_sub(1).max(1);
        let points: Vec<Json> = (0..self.history_points)
            .map(|index| {
                let progress = index as f64 / span as f64;
                json!({ "y": self.history_open + (self.price - self.history_open) * progress })
            })
            .collect();
        json!({ "price": points })
    }
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

    fn body(params: &Params, path: &str) -> Json {
        let resource = params.resource("public-api", 20_400);
        let endpoint = resource
            .endpoints
            .iter()
            .find(|endpoint| endpoint.path == path)
            .expect("BUG: the profile must serve the path under test");
        let ResponseSpec::Computed(responder) = &endpoint.response else {
            panic!("BUG: {path} must answer on the clock");
        };
        match responder(&ctx(0.0)).data {
            ResponseData::Json(json) => json,
            ResponseData::Bytes { .. } => panic!("BUG: {path} answered with bytes, not JSON"),
        }
    }

    #[test]
    fn serves_every_path_the_miner_widgets_read() {
        let resource = Params::default().resource("public-api", 20_400);
        let paths: Vec<_> = resource
            .endpoints
            .iter()
            .map(|endpoint| endpoint.path.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                PRICE_STATS_PATH,
                BLOCKS_PATH,
                DIFFICULTY_PATH,
                HASHRATE_PATH,
                PRICE_HISTORY_PATH,
            ]
        );
    }

    #[test]
    fn the_block_height_sits_at_the_head_of_the_array() {
        let params = Params {
            block_height: 880_123,
            ..Params::default()
        };
        let blocks = body(&params, BLOCKS_PATH);
        assert_eq!(
            blocks[0]["height"],
            json!(880_123),
            "the widget asks for limit=1 and reads the head"
        );
    }

    #[test]
    fn the_history_series_runs_from_its_open_to_the_current_price() {
        let params = Params {
            history_open: 100.0,
            price: 200.0,
            history_points: 3,
            ..Params::default()
        };
        let history = body(&params, PRICE_HISTORY_PATH);
        assert_eq!(history["price"][0]["y"], json!(100.0));
        assert_eq!(history["price"][1]["y"], json!(150.0));
        assert_eq!(history["price"][2]["y"], json!(200.0));
    }

    #[test]
    fn a_single_point_history_still_renders_one_sample() {
        let params = Params {
            history_open: 100.0,
            price: 200.0,
            history_points: 1,
            ..Params::default()
        };
        let history = body(&params, PRICE_HISTORY_PATH);
        assert_eq!(history["price"][0]["y"], json!(100.0));
        assert_eq!(history["price"][1], Json::Null, "the series stops at one");
    }

    #[test]
    fn endpoints_hold_up_until_fail_after_secs_then_take_the_fault_status() {
        let failing = Params {
            status: HttpStatus::SERVICE_UNAVAILABLE,
            fail_after_secs: Some(20),
            ..Params::default()
        };
        assert_eq!(failing.status_at(&ctx(19.0)), HttpStatus::OK);
        assert_eq!(
            failing.status_at(&ctx(20.0)),
            HttpStatus::SERVICE_UNAVAILABLE
        );
    }
}
