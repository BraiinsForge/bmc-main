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

//! AxeOS profile (NerdQaxe++).
//! Announces `_http._tcp` with the `_axeos` subtype
//! and identifying TXT records, serving `GET /api/system/info`.
//!
//! Shape follows the ESP-Miner `/api/system/info` schema
//! (GPL-3.0; referenced, not vendored: <https://github.com/bitaxeorg/ESP-Miner>)
//! and the widget's `families/bitaxe.rs` adapter.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::blueprint::{AnnounceSpec, Body, EndpointSpec, ResourceSpec, Sampler, SeriesSpec};
use crate::build::{celsius, drift, leaf, mac, steady};
use crate::cache::Cache;
use crate::http_status::HttpStatus;
use crate::quantity::{Celsius, NonNegative};

/// AxeOS 10-minute history window: 300 samples at a 2 s cadence
/// (ESP-Miner `HISTORY_WINDOW_TEN_MINS`).
const WINDOW_SAMPLES: usize = 300;
const SAMPLE_PERIOD_S: f64 = 2.0;

/// Series names shared by the sampler and `statistics_body`; they match the
/// `/api/system/info` keys so seeds — and the newest sample — align with it.
const SERIES_HASHRATE: &str = "hashRate";
const SERIES_POWER: &str = "power";
const SERIES_TEMP: &str = "temp";

/// Shape the recorded history into ESP-Miner's `/api/system/statistics` matrix
/// (row per sample in `labels` order, ms time last, relative to the oldest).
fn statistics_body(cache: &Cache) -> Json {
    // rows[*].values are [hashrate, power, temp] in the requested order.
    let rows = cache.rows(&[SERIES_HASHRATE, SERIES_POWER, SERIES_TEMP]);
    let base_s = rows.first().map_or(0.0, |row| row.t_s);
    let statistics: Vec<_> = rows
        .iter()
        .map(|row| {
            json!([
                row.values[0],
                row.values[1],
                row.values[2],
                (row.t_s - base_s) * 1_000.0,
            ])
        })
        .collect();
    let current = rows.last().map_or(0.0, |row| (row.t_s - base_s) * 1_000.0);
    json!({
        "currentTimestamp": current,
        "labels": ["hashrate", "power", "asicTemp", "timestamp"],
        "statistics": statistics,
    })
}

/// Tunables for a simulated AxeOS miner.
// Strict for the same reason as the BOS params: a mistyped key in a
// hand-authored blueprint must not silently drop the fault it meant to inject.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(rename = "AxeosParams")]
pub struct Params {
    /// Model name reported as `deviceModel`.
    pub model_name: String,
    /// Current hashrate to hover around, in TH/s.
    pub hashrate_ths: NonNegative,
    /// Nameplate (expected) hashrate at current settings, in TH/s.
    pub nominal_ths: NonNegative,
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
            model_name: "NerdQAxe++".to_owned(),
            hashrate_ths: NonNegative::from(4.5),
            nominal_ths: NonNegative::from(4.5),
            power_w: NonNegative::from(76.0),
            temp_c: Celsius::from(62.0),
            uptime_s: 187_020,
            status: HttpStatus::OK,
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        let mut txt = BTreeMap::new();
        txt.insert("family".to_owned(), "NerdQAxe".to_owned());
        txt.insert("board".to_owned(), "++".to_owned());
        txt.insert("asic".to_owned(), "BM1370".to_owned());
        txt.insert("asic_count".to_owned(), "4".to_owned());
        // Shared by the live leaves and the sampled series; series are named
        // after the info keys so seeds — and the newest sample — align.
        let hashrate = drift(self.hashrate_ths.get() * 1_000.0);
        let power = drift(self.power_w.get());
        let temp = celsius(self.temp_c.get());
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: Some(AnnounceSpec::Mdns {
                service_type: "_http._tcp".to_owned(),
                subtype: Some("_axeos".to_owned()),
                txt,
            }),
            endpoints: vec![
                EndpointSpec {
                    method: "GET".to_owned(),
                    path: "/api/system/info".to_owned(),
                    body: Body::Render(json!({
                        "hashRate": leaf(hashrate),
                        "expectedHashrate": leaf(steady(self.nominal_ths.get() * 1_000.0)),
                        "power": leaf(power),
                        "temp": leaf(temp),
                        "uptimeSeconds": self.uptime_s,
                        "macAddr": mac(name),
                        "deviceModel": self.model_name.as_str(),
                        "ASICModel": "BM1370",
                        "asicCount": 4,
                    })),
                    status: self.status,
                },
                EndpointSpec {
                    method: "GET".to_owned(),
                    path: "/api/system/statistics".to_owned(),
                    body: Body::accumulate(statistics_body),
                    status: self.status,
                },
            ],
            sampler: Some(Sampler {
                period_s: SAMPLE_PERIOD_S,
                series: vec![
                    SeriesSpec {
                        name: SERIES_HASHRATE.to_owned(),
                        value: hashrate,
                        capacity: WINDOW_SAMPLES,
                    },
                    SeriesSpec {
                        name: SERIES_POWER.to_owned(),
                        value: power,
                        capacity: WINDOW_SAMPLES,
                    },
                    SeriesSpec {
                        name: SERIES_TEMP.to_owned(),
                        value: temp,
                        capacity: WINDOW_SAMPLES,
                    },
                ],
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Params, statistics_body};
    use crate::cache::Cache;
    use crate::sampler::backfill;

    #[test]
    fn statistics_body_shapes_the_full_recorded_window() {
        let resource = Params::default().resource("axeos-01", 20_000);
        let sampler = resource.sampler.expect("axeos opts into a sampler");
        let cache = Cache::new(sampler.series.iter().map(|s| (s.name.clone(), s.capacity)));
        backfill(&cache, &sampler, 0x1234);

        let body = statistics_body(&cache);
        assert_eq!(
            body["labels"],
            serde_json::json!(["hashrate", "power", "asicTemp", "timestamp"]),
        );
        let rows = body["statistics"]
            .as_array()
            .expect("statistics is an array");
        assert_eq!(
            rows.len(),
            300,
            "one row per sample in the 10-minute window"
        );
        for row in rows {
            let cells = row.as_array().expect("row is an array");
            assert_eq!(
                cells.len(),
                4,
                "row is [hashrate, power, asicTemp, timestamp]"
            );
        }
        // The newest row's timestamp is the reported current timestamp.
        let newest_ts = rows.last().expect("non-empty")[3].clone();
        assert_eq!(body["currentTimestamp"], newest_ts);
        // Hashrate hovers near the 4.5 TH/s nominal (~4500 GH/s).
        let first_hashrate = rows[0][0].as_f64().expect("hashrate is a number");
        assert!(
            (4_000.0..5_000.0).contains(&first_hashrate),
            "hashrate {first_hashrate} near nominal",
        );
        // Timestamps are positive and non-decreasing (relative to the oldest).
        let timestamps: Vec<f64> = rows
            .iter()
            .map(|row| row[3].as_f64().expect("timestamp is a number"))
            .collect();
        assert!(
            timestamps[0] >= 0.0,
            "oldest timestamp {} negative",
            timestamps[0]
        );
        assert!(
            timestamps.windows(2).all(|w| w[1] >= w[0]),
            "timestamps non-decreasing",
        );
    }
}
