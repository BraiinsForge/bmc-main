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

//! BOS+ profile (BMM/BFM). Announces `_http._tcp` with the `_bos` subtype
//! and serves the boser REST paths the widget reads, in BOS units (GH/s, watts).
//!
//! Shape follows the boser REST API ("Braiins OS Public REST API")
//! as consumed by the widget's `families/bos.rs` adapter.
//! Nominal is `sticker_hashrate` (a `GigaHashrate`, i.e. `{gigahash_per_second}`)
//! from `GetMinerDetailsResponse` (`GET /api/v1/miner/details`);
//! no public spec URL to link yet.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::blueprint::{
    AnnounceSpec, EndpointSpec, RequestCtx, ResourceSpec, Response, ResponseSpec,
};
use crate::build::{celsius, drift, leaf, mac, steady};
use crate::http_status::HttpStatus;
use crate::noise::{mix, mix_index, stable01};
use crate::quantity::{Celsius, NonNegative};
use crate::render;

/// Peak per-board temperature spread (°C): each board sits within half this
/// of the miner's baseline.
const BOARD_TEMP_SPREAD_C: f64 = 12.0;

/// Hashboards the simulated miner reports.
const BOARDS: usize = 2;

/// How far the board sensor sits below the hottest chip on the same board.
const BOARD_BELOW_CHIP_C: f64 = 13.0;

/// One board's share of the miner's hashrate, since the miner
/// reports totals and each board reports its own part of them.
#[expect(
    clippy::cast_precision_loss,
    reason = "a board count this small is exact in f64"
)]
const BOARD_SHARE: f64 = 1.0 / BOARDS as f64;

/// A stable per-board temperature offset (°C), keyed on device identity
/// and board index, so the fleet shows a real min/avg/max spread.
fn board_offset(base: u64, index: usize) -> f64 {
    (stable01(mix_index(base, index)) - 0.5) * BOARD_TEMP_SPREAD_C
}

/// Tunables for a simulated BOS+ miner.
// Strict, unlike persisted state: a blueprint is hand-authored,
// so a mistyped key is a fault that would silently never fire
// rather than a forward-compatible field to skip over.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(rename = "BosParams")]
pub struct Params {
    /// Model name reported on `/miner/details`.
    pub model_name: String,
    /// Current hashrate to hover around, in TH/s (low value = not-okay).
    pub hashrate_ths: NonNegative,
    /// Nameplate (`sticker_hashrate`) hashrate, in TH/s.
    pub nominal_ths: NonNegative,
    /// Power draw to hover around, in W.
    pub power_w: NonNegative,
    /// Junction temperature to hover around, in °C.
    pub temp_c: Celsius,
    /// Reported uptime, in seconds.
    pub uptime_s: u64,
    /// HTTP status the telemetry endpoints return (503 = present, unreadable).
    pub status: HttpStatus,
    /// HTTP status the login endpoint returns; 401 = the miner needs credentials
    /// the widget doesn't have, so it never authenticates (the no-creds case).
    pub auth_status: HttpStatus,
    /// Answer 200 before this many seconds of scenario time,
    /// then switch to `status`.
    /// Drives the stale case: data lands, then stops refreshing.
    pub fail_after_secs: Option<u32>,
    /// Fan duty reported on `/cooling/state`, in percent.
    pub fan_percent: NonNegative,
    /// Address reported on `/network/`.
    pub ip_address: String,
    /// Chip model each hashboard reports.
    /// Empty reports no chip identity at all,
    /// which drops the chip header rather than drawing placeholders.
    pub chip_type: String,
    /// Chips per hashboard.
    pub chips_count: usize,
    /// Tuner hashrate target, in TH/s.
    /// Absent leaves the round gauge with no sweep
    /// to anchor, which is the un-tuned miner.
    pub hashrate_target: Option<TargetRange>,
    /// Tuner power target, in W.
    pub power_target: Option<TargetRange>,
}

/// The `{min, default, max}` triple a tuner target reports.
/// All three edges are required — the widget drops
/// the whole target when any one is missing.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "BosTargetRange")]
pub struct TargetRange {
    pub min: f64,
    pub default: f64,
    pub max: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            model_name: "Braiins Mini Miner BMM 101".to_owned(),
            hashrate_ths: NonNegative::from(1.0),
            nominal_ths: NonNegative::from(1.0),
            power_w: NonNegative::from(32.0),
            temp_c: Celsius::from(65.0),
            uptime_s: 187_020,
            status: HttpStatus::OK,
            auth_status: HttpStatus::OK,
            fail_after_secs: None,
            fan_percent: NonNegative::from(72.0),
            ip_address: "192.168.23.1".to_owned(),
            chip_type: "BM1370".to_owned(),
            chips_count: 76,
            hashrate_target: Some(TargetRange {
                min: 0.5,
                default: 1.0,
                max: 1.4,
            }),
            power_target: Some(TargetRange {
                min: 20.0,
                default: 32.0,
                max: 45.0,
            }),
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        let ghs = leaf(drift(self.hashrate_ths.get() * 1_000.0));
        let watt = leaf(drift(self.power_w.get()));
        let base = mix(0, name);
        let endpoints = vec![
            // The widget fingerprints BOS over this unauthenticated endpoint
            // before crediting; always 200, independent of the miner's state.
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/api/v1/version".to_owned(),
                response: ResponseSpec::render(json!({ "major": 1, "minor": 6, "patch": 0 })),
            },
            EndpointSpec {
                method: "POST".to_owned(),
                path: "/api/v1/auth/login".to_owned(),
                response: ResponseSpec::Render {
                    status: self.auth_status,
                    template: json!({ "token": "sim-token" }),
                },
            },
            self.telemetry(
                "/api/v1/miner/stats",
                json!({
                    "miner_stats": { "real_hashrate": { "last_1m": { "gigahash_per_second": ghs } } },
                    "power_stats": { "approximated_consumption": { "watt": watt } },
                }),
            ),
            self.telemetry(
                "/api/v1/miner/hw/hashboards",
                json!({
                    "hashboards": (0..BOARDS).map(|index| self.board(board_offset(base, index))).collect::<Vec<_>>(),
                }),
            ),
            self.telemetry(
                "/api/v1/miner/details",
                json!({
                    "bosminer_uptime_s": self.uptime_s,
                    "platform": 8,
                    "mac_address": mac(name),
                    "miner_identity": { "miner_model": self.model_name.as_str() },
                    "sticker_hashrate": { "gigahash_per_second": leaf(steady(self.nominal_ths.get() * 1_000.0)) },
                }),
            ),
            // The widget reads a fraction here and renders it as a percent.
            self.telemetry(
                "/api/v1/cooling/state",
                json!({
                    "fans": [{ "target_speed_ratio": self.fan_percent.get() / 100.0 }],
                }),
            ),
            self.telemetry(
                "/api/v1/network/",
                json!({ "networks": [{ "address": self.ip_address.as_str() }] }),
            ),
            self.telemetry(
                "/api/v1/configuration/constraints",
                json!({
                    "tuner_constraints": {
                        "hashrate_target": target(self.hashrate_target.as_ref(), "terahash_per_second"),
                        "power_target": target(self.power_target.as_ref(), "watt"),
                    },
                }),
            ),
        ];
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: Some(AnnounceSpec::Mdns {
                service_type: "_http._tcp".to_owned(),
                subtype: Some("_bos".to_owned()),
                txt: BTreeMap::new(),
            }),
            endpoints,
            sampler: None,
        }
    }

    /// An endpoint carrying miner telemetry, so it answers on `status_at`
    /// rather than a status fixed when the scenario was built.
    ///
    /// The template still renders per request, keeping the drift
    /// a `Render` endpoint would give.
    fn telemetry(&self, path: &str, template: Json) -> EndpointSpec {
        let params = self.clone();
        EndpointSpec {
            method: "GET".to_owned(),
            path: path.to_owned(),
            response: ResponseSpec::computed(move |ctx: &RequestCtx| {
                Response::new(
                    params.status_at(ctx),
                    render::render(&template, ctx.t_s, ctx.seed),
                )
            }),
        }
    }

    /// `status` throughout, unless `fail_after_secs` holds
    /// the endpoint healthy for an opening stretch first.
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

    /// A hashboard whose temperatures drift around the baseline plus `offset_c`.
    ///
    /// The board runs cooler than its hottest chip, and carries its own share of
    /// the miner's hashrate: the single-miner widgets read both pairs, pairing
    /// board with chip for one temperature reading and real against nominal for
    /// the mining-mode ratio.
    fn board(&self, offset_c: f64) -> Json {
        let mut board = json!({
            "board_temp": { "degree_c": leaf(celsius(self.temp_c.get() + offset_c - BOARD_BELOW_CHIP_C)) },
            "highest_chip_temp": { "temperature": { "degree_c": leaf(celsius(self.temp_c.get() + offset_c)) } },
            "stats": {
                "real_hashrate": { "last_1m": { "gigahash_per_second": leaf(drift(self.hashrate_ths.get() * BOARD_SHARE * 1_000.0)) } },
                "nominal_hashrate": { "gigahash_per_second": leaf(steady(self.nominal_ths.get() * BOARD_SHARE * 1_000.0)) },
            },
        });
        // An empty chip type stands for firmware that reports no chip identity,
        // which drops the whole chip header rather than showing placeholders.
        if !self.chip_type.is_empty() {
            let map = board
                .as_object_mut()
                .expect("BUG: the board body is a JSON object");
            map.insert("chip_type".to_owned(), json!(self.chip_type.as_str()));
            map.insert("chips_count".to_owned(), json!(self.chips_count));
        }
        board
    }
}

/// A tuner target as the API reports it, keyed on the unit leaf the widget reads.
/// An absent target renders as `{}`, which drops the whole range widget-side.
fn target(range: Option<&TargetRange>, leaf_name: &str) -> Json {
    let Some(range) = range else {
        return json!({});
    };
    json!({
        "min": { leaf_name: range.min },
        "default": { leaf_name: range.default },
        "max": { leaf_name: range.max },
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

    /// The JSON a telemetry route answers with, rendered as of `t_s`.
    fn body(params: &Params, path: &str, t_s: f64) -> Json {
        let resource = params.resource("miner", 20_300);
        let endpoint = resource
            .endpoints
            .iter()
            .find(|endpoint| endpoint.path == path)
            .expect("BUG: the profile must serve the path under test");
        let ResponseSpec::Computed(responder) = &endpoint.response else {
            panic!("BUG: {path} must be a telemetry endpoint to answer on the clock");
        };
        match responder(&ctx(t_s)).data {
            ResponseData::Json(json) => json,
            ResponseData::Bytes { .. } => panic!("BUG: {path} answered with bytes, not JSON"),
        }
    }

    #[test]
    fn serves_every_path_the_miner_widgets_read() {
        let resource = Params::default().resource("miner", 20_300);
        let paths: Vec<_> = resource
            .endpoints
            .iter()
            .map(|endpoint| endpoint.path.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                "/api/v1/version",
                "/api/v1/auth/login",
                "/api/v1/miner/stats",
                "/api/v1/miner/hw/hashboards",
                "/api/v1/miner/details",
                "/api/v1/cooling/state",
                "/api/v1/network/",
                "/api/v1/configuration/constraints",
            ]
        );
    }

    #[test]
    fn a_hashboard_carries_both_temperatures_and_both_hashrates() {
        let boards = body(&Params::default(), "/api/v1/miner/hw/hashboards", 0.0);
        let board = &boards["hashboards"][0];
        // Temperature needs the pair, and the mining-mode ratio needs both rates;
        // either half missing leaves the reading unavailable widget-side.
        assert!(board["board_temp"]["degree_c"].is_number());
        assert!(board["highest_chip_temp"]["temperature"]["degree_c"].is_number());
        assert!(board["stats"]["real_hashrate"]["last_1m"]["gigahash_per_second"].is_number());
        assert!(board["stats"]["nominal_hashrate"]["gigahash_per_second"].is_number());
    }

    #[test]
    fn the_board_sensor_reads_below_its_hottest_chip() {
        let boards = body(&Params::default(), "/api/v1/miner/hw/hashboards", 0.0);
        let board = &boards["hashboards"][0];
        let board_c = board["board_temp"]["degree_c"]
            .as_f64()
            .expect("BUG: board temperature must be a number");
        let chip_c = board["highest_chip_temp"]["temperature"]["degree_c"]
            .as_f64()
            .expect("BUG: chip temperature must be a number");
        assert!(
            board_c < chip_c,
            "board {board_c} should sit under chip {chip_c}"
        );
    }

    #[test]
    fn an_empty_chip_type_reports_no_chip_identity() {
        let params = Params {
            chip_type: String::new(),
            ..Params::default()
        };
        let boards = body(&params, "/api/v1/miner/hw/hashboards", 0.0);
        let board = &boards["hashboards"][0];
        assert_eq!(board["chip_type"], Json::Null);
        assert_eq!(board["chips_count"], Json::Null);
        assert!(
            board["board_temp"]["degree_c"].is_number(),
            "only the chip identity drops, not the board's readings"
        );
    }

    #[test]
    fn cooling_reports_the_fan_duty_as_the_ratio_the_widget_reads() {
        let params = Params {
            fan_percent: NonNegative::from(72.0),
            ..Params::default()
        };
        let cooling = body(&params, "/api/v1/cooling/state", 0.0);
        assert_eq!(cooling["fans"][0]["target_speed_ratio"], json!(0.72));
    }

    #[test]
    fn a_tuner_target_carries_all_three_edges_under_its_unit_leaf() {
        let params = Params {
            hashrate_target: Some(TargetRange {
                min: 0.5,
                default: 1.0,
                max: 1.4,
            }),
            ..Params::default()
        };
        let constraints = body(&params, "/api/v1/configuration/constraints", 0.0);
        let target = &constraints["tuner_constraints"]["hashrate_target"];
        assert_eq!(target["min"]["terahash_per_second"], json!(0.5));
        assert_eq!(target["default"]["terahash_per_second"], json!(1.0));
        assert_eq!(target["max"]["terahash_per_second"], json!(1.4));
    }

    #[test]
    fn an_absent_tuner_target_reports_no_edges_at_all() {
        let params = Params {
            hashrate_target: None,
            ..Params::default()
        };
        let constraints = body(&params, "/api/v1/configuration/constraints", 0.0);
        assert_eq!(
            constraints["tuner_constraints"]["hashrate_target"],
            json!({}),
            "a partial range would strand the gauge with no sweep to anchor"
        );
    }

    #[test]
    fn telemetry_holds_up_until_fail_after_secs_then_takes_the_fault_status() {
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

        let steady = Params::default();
        assert_eq!(steady.status_at(&ctx(0.0)), HttpStatus::OK);
        assert_eq!(steady.status_at(&ctx(9_999.0)), HttpStatus::OK);
    }
}
