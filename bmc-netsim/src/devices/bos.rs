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

use crate::blueprint::{AnnounceSpec, EndpointSpec, ResourceSpec, ResponseSpec};
use crate::build::{celsius, drift, leaf, mac, steady};
use crate::http_status::HttpStatus;
use crate::noise::{mix, mix_index, stable01};
use crate::quantity::{Celsius, NonNegative};

/// Peak per-board temperature spread (°C): each board sits within half this
/// of the miner's baseline.
const BOARD_TEMP_SPREAD_C: f64 = 12.0;

/// A stable per-board temperature offset (°C), keyed on device identity and
/// board index, so the fleet shows a real min/avg/max spread.
fn board_offset(base: u64, index: usize) -> f64 {
    (stable01(mix_index(base, index)) - 0.5) * BOARD_TEMP_SPREAD_C
}

/// Tunables for a simulated BOS+ miner.
// Strict, unlike persisted state: a blueprint is hand-authored, so a mistyped
// key is a fault that would silently never fire rather than a forward-compatible
// field to skip over.
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
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/api/v1/miner/stats".to_owned(),
                response: ResponseSpec::Render {
                    status: self.status,
                    template: json!({
                        "miner_stats": { "real_hashrate": { "last_1m": { "gigahash_per_second": ghs } } },
                        "power_stats": { "approximated_consumption": { "watt": watt } },
                    }),
                },
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/api/v1/miner/hw/hashboards".to_owned(),
                response: ResponseSpec::Render {
                    status: self.status,
                    template: json!({
                        "hashboards": [self.board(board_offset(base, 0)), self.board(board_offset(base, 1))],
                    }),
                },
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/api/v1/miner/details".to_owned(),
                response: ResponseSpec::Render {
                    status: self.status,
                    template: json!({
                        "bosminer_uptime_s": self.uptime_s,
                        "platform": 8,
                        "mac_address": mac(name),
                        "miner_identity": { "miner_model": self.model_name.as_str() },
                        "sticker_hashrate": { "gigahash_per_second": leaf(steady(self.nominal_ths.get() * 1_000.0)) },
                    }),
                },
            },
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

    /// A hashboard whose chip temperature drifts around the baseline plus `offset_c`.
    fn board(&self, offset_c: f64) -> Json {
        json!({
            "highest_chip_temp": { "temperature": { "degree_c": leaf(celsius(self.temp_c.get() + offset_c)) } },
            "chip_type": "BM1370",
            "chips_count": 76,
        })
    }
}
