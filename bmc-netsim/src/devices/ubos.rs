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

//! uBOS profile (Braiins Forge Miner x4). Announces the full
//! `_ubos._tcp` type and serves `GET /api/info` in uBOS units (raw H/s, mW).
//!
//! Shape follows the uBOS `/api/info` response as consumed
//! by the widget's `families/ubos.rs` adapter (snake_case: name,
//! hashrate, power_out_mw, temperature, uptime).
//!
//! uBOS exposes no nominal/nameplate hashrate: neither `/api/info`
//! nor the newer `/api/system/info` carries one, and the board descriptor
//! stores no rated figure — so this profile faithfully serves none.
//!
//! A nominal for the okay/not-okay rule would have to come from an external model catalog.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::blueprint::{AnnounceSpec, Body, EndpointSpec, ResourceSpec};
use crate::build::{celsius, drift, leaf};
use crate::http_status::HttpStatus;
use crate::quantity::{Celsius, NonNegative};

/// Tunables for a simulated Braiins Forge Miner x4 (Braiins OS Libre).
// Strict for the same reason as the BOS params: a mistyped key in a
// hand-authored blueprint must not silently drop the fault it meant to inject.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(rename = "UbosParams")]
pub struct Params {
    /// Product name reported on `/api/info`.
    pub model_name: String,
    /// Current hashrate to hover around, in TH/s.
    pub hashrate_ths: NonNegative,
    /// Power draw to hover around, in W.
    pub power_w: NonNegative,
    /// Temperature to hover around, in °C.
    pub temp_c: Celsius,
    /// Reported uptime, in seconds.
    pub uptime_s: u64,
    /// HTTP status the telemetry endpoint returns (503 = present, unreadable).
    pub status: HttpStatus,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            model_name: "Braiins Forge Miner x4".to_owned(),
            hashrate_ths: NonNegative::from(4.8),
            power_w: NonNegative::from(76.0),
            temp_c: Celsius::from(65.0),
            uptime_s: 187_020,
            status: HttpStatus::OK,
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: Some(AnnounceSpec::Mdns {
                service_type: "_ubos._tcp".to_owned(),
                subtype: None,
                txt: BTreeMap::new(),
            }),
            endpoints: vec![EndpointSpec {
                method: "GET".to_owned(),
                path: "/api/info".to_owned(),
                body: Body::Render(json!({
                    "name": self.model_name.as_str(),
                    "hashrate": leaf(drift(self.hashrate_ths.get() * 1e12)),
                    "power_out_mw": leaf(drift(self.power_w.get() * 1_000.0)),
                    "temperature": leaf(celsius(self.temp_c.get())),
                    "uptime": self.uptime_s,
                })),
                status: self.status,
            }],
            sampler: None,
        }
    }
}
