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

//! Nexus Bitcoin mining profile — a cloud API reached through testbed URL rewriting,
//! never LAN discovery. Serves the two envelopes consumed by the widget.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::blueprint::{EndpointSpec, RequestCtx, ResourceSpec, Response, ResponseSpec};
use crate::http_status::HttpStatus;

const INFO_PATH: &str = "/api/v1/data/bitcoin/mining-info";
const HISTORY_PATH: &str = "/api/v1/data/bitcoin/mining-history";
const INFO_TTL_SECS: u64 = 60;
const HISTORY_TTL_SECS: u64 = 10 * 60;

/// Scenario controls and values returned by the simulated Nexus endpoints.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default)]
#[schemars(rename = "BitcoinMiningDataParams")]
pub struct Params {
    /// HTTP status returned after startup, or after `fail_after_secs`.
    pub status: HttpStatus,
    /// Answer 503 until this many seconds of scenario time have elapsed.
    pub warmup_secs: u32,
    /// Answer 200 before this point, then switch to `status`.
    pub fail_after_secs: Option<u32>,
    /// Cache age advertised in both Nexus response envelopes.
    pub cache_age_secs: u64,
    /// Bitcoin price, in USD.
    pub bitcoin_price_usd: f64,
    /// Bitcoin network difficulty.
    pub difficulty: f64,
    /// Previous difficulty adjustment as a ratio; `0.01` means 1%.
    pub previous_adjustment: f64,
    /// Estimated difficulty adjustment as a ratio; `0.01` means 1%.
    pub estimated_adjustment: f64,
    /// Blocks mined in the current 2016-block epoch.
    pub blocks_this_epoch: u64,
    /// Current epoch block time, in seconds.
    pub epoch_block_time_secs: u64,
    /// Bitcoin network hashrate, in EH/s.
    pub network_hashrate_ehs: f64,
    /// Average transaction fees per block, in BTC.
    pub avg_fees_per_block_btc: f64,
    /// Transaction fees as a percentage of the block reward.
    pub fees_percent: f64,
    /// Hashprice, in USD per TH/s per day.
    pub hashprice_usd_per_th_day: f64,
    /// Total mining revenue, in USD.
    pub revenue_usd: f64,
    /// Latest Bitcoin block height.
    pub block_height: u64,
    /// Blocks mined in the last 24 hours.
    pub blocks_last_24h: u64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            status: HttpStatus::OK,
            warmup_secs: 0,
            fail_after_secs: None,
            cache_age_secs: 0,
            bitcoin_price_usd: 169_420.0,
            difficulty: 129.7e12,
            previous_adjustment: -0.0281,
            estimated_adjustment: 0.104,
            blocks_this_epoch: 1_293,
            epoch_block_time_secs: 559,
            network_hashrate_ehs: 877.8,
            avg_fees_per_block_btc: 0.021,
            fees_percent: 0.66,
            hashprice_usd_per_th_day: 0.0562,
            revenue_usd: 49.35e6,
            block_height: 914_038,
            blocks_last_24h: 151,
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        let endpoint = |path: &str, body: fn(&Params) -> Json| EndpointSpec {
            method: "GET".to_owned(),
            path: path.to_owned(),
            response: ResponseSpec::computed({
                let params = self.clone();
                move |ctx| Response::new(params.status_at(ctx), body(&params))
            }),
        };
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: None,
            endpoints: vec![
                endpoint(INFO_PATH, mining_info),
                endpoint(HISTORY_PATH, mining_history),
            ],
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

fn mining_info(params: &Params) -> Json {
    let estimate = chrono::Utc::now() + chrono::Duration::days(9);
    json!({
        "resource": "bitcoin/mining-info",
        "data": {
            "block_height": params.block_height,
            "btc_price": params.bitcoin_price_usd,
            "btc_price_change_24h": 5.31,
            "difficulty": params.difficulty,
            "previous_adjustment": params.previous_adjustment,
            "estimated_adjustment": params.estimated_adjustment,
            "estimated_adjustment_date": estimate.to_rfc3339(),
            "blocks_this_epoch": params.blocks_this_epoch,
            "epoch_block_time": params.epoch_block_time_secs,
            "network_hashrate": params.network_hashrate_ehs,
            "hashprice": params.hashprice_usd_per_th_day,
            "avg_fees_per_block": params.avg_fees_per_block_btc,
            "fees_percent": params.fees_percent,
            "total_mining_revenue": params.revenue_usd,
            "blocks_last_24h": params.blocks_last_24h,
        },
        "cache_age_secs": params.cache_age_secs,
        "ttl_secs": INFO_TTL_SECS,
    })
}

fn mining_history(params: &Params) -> Json {
    json!({
        "resource": "bitcoin/mining-history",
        "data": {
            "btc_price": history(48, 30 * 60, params.bitcoin_price_usd, 0.035, 0.002),
            "hashrate": history(48, 30 * 60, params.network_hashrate_ehs, 0.055, 0.0015),
            "difficulty": history(52, 7 * 86_400, params.difficulty, 0.045, 0.001),
        },
        "cache_age_secs": params.cache_age_secs,
        "ttl_secs": HISTORY_TTL_SECS,
    })
}

fn history(count: usize, interval_secs: i64, center: f64, swing: f64, rise: f64) -> Vec<Json> {
    let now = chrono::Utc::now();
    (0..count)
        .map(|index| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "history fixture indices are tiny and exact in f64"
            )]
            let x = index as f64;
            let age = i64::try_from(count - index).unwrap_or(i64::MAX) * interval_secs;
            let wave = (x * 0.73).sin() * swing + (x * 1.91).cos() * swing * 0.35;
            json!({
                "x": (now - chrono::Duration::seconds(age)).to_rfc3339(),
                "y": center * (1.0 + wave + rise * x),
            })
        })
        .collect()
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

    #[test]
    fn resource_exposes_only_the_two_nexus_routes() {
        let resource = Params::default().resource("bitcoin", 20_200);
        let paths: Vec<_> = resource
            .endpoints
            .iter()
            .map(|endpoint| endpoint.path.as_str())
            .collect();
        assert_eq!(paths, [INFO_PATH, HISTORY_PATH]);
    }

    #[test]
    fn info_uses_the_nexus_envelope_and_cadence() {
        let params = Params {
            cache_age_secs: 61,
            ..Params::default()
        };
        let response = mining_info(&params);
        assert_eq!(response["resource"], "bitcoin/mining-info");
        assert_eq!(response["ttl_secs"], INFO_TTL_SECS);
        assert_eq!(response["cache_age_secs"], 61);
        assert_eq!(response["data"]["blocks_last_24h"], 151);
    }

    #[test]
    fn history_contains_all_three_timestamped_series() {
        let response = mining_history(&Params::default());
        assert_eq!(response["resource"], "bitcoin/mining-history");
        assert_eq!(response["ttl_secs"], HISTORY_TTL_SECS);
        for name in ["btc_price", "hashrate", "difficulty"] {
            let samples = response["data"][name]
                .as_array()
                .expect("BUG: Nexus history series is an array");
            assert!(!samples.is_empty());
            assert!(
                samples[0]["x"]
                    .as_str()
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .is_some()
            );
        }
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
