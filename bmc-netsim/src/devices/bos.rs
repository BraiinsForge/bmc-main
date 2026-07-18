// Copyright (C) 2026  Braiins Systems s.r.o.

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

use crate::blueprint::{AnnounceSpec, Body, EndpointSpec, ResourceSpec};
use crate::build::{celsius, drift, leaf, mac, steady};
use crate::noise::{mix, mix_index, stable01};

/// Peak per-board temperature spread (°C): each board sits within half this
/// of the miner's baseline.
const BOARD_TEMP_SPREAD_C: f64 = 12.0;

/// A stable per-board temperature offset (°C), keyed on device identity and
/// board index, so the fleet shows a real min/avg/max spread.
fn board_offset(base: u64, index: usize) -> f64 {
    (stable01(mix_index(base, index)) - 0.5) * BOARD_TEMP_SPREAD_C
}

/// Tunables for a simulated BOS+ miner.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default)]
#[schemars(rename = "BosParams")]
pub struct Params {
    /// Model name reported on `/miner/details`.
    pub model_name: String,
    /// Current hashrate to hover around, in TH/s (low value = not-okay).
    pub hashrate_ths: f64,
    /// Nameplate (`sticker_hashrate`) hashrate, in TH/s.
    pub nominal_ths: f64,
    /// Power draw to hover around, in W.
    pub power_w: f64,
    /// Junction temperature to hover around, in °C.
    pub temp_c: f64,
    /// Reported uptime, in seconds.
    pub uptime_s: u64,
    /// HTTP status the telemetry endpoints return (503 = present, unreadable).
    pub status: u16,
    /// HTTP status the login endpoint returns; 401 = the miner needs credentials
    /// the widget doesn't have, so it never authenticates (the no-creds case).
    pub auth_status: u16,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            model_name: "Braiins Mini Miner BMM 101".to_owned(),
            hashrate_ths: 1.0,
            nominal_ths: 1.0,
            power_w: 32.0,
            temp_c: 65.0,
            uptime_s: 187_020,
            status: 200,
            auth_status: 200,
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        let ghs = leaf(drift(self.hashrate_ths * 1_000.0));
        let watt = leaf(drift(self.power_w));
        let base = mix(0, name);
        let endpoints = vec![
            // The widget fingerprints BOS over this unauthenticated endpoint
            // before crediting; always 200, independent of the miner's state.
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/api/v1/version".to_owned(),
                body: Body::Render(json!({ "major": 1, "minor": 6, "patch": 0 })),
                status: 200,
            },
            EndpointSpec {
                method: "POST".to_owned(),
                path: "/api/v1/auth/login".to_owned(),
                body: Body::Render(json!({ "token": "sim-token" })),
                status: self.auth_status,
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/api/v1/miner/stats".to_owned(),
                body: Body::Render(json!({
                    "miner_stats": { "real_hashrate": { "last_1m": { "gigahash_per_second": ghs } } },
                    "power_stats": { "approximated_consumption": { "watt": watt } },
                })),
                status: self.status,
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/api/v1/miner/hw/hashboards".to_owned(),
                body: Body::Render(json!({
                    "hashboards": [self.board(board_offset(base, 0)), self.board(board_offset(base, 1))],
                })),
                status: self.status,
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/api/v1/miner/details".to_owned(),
                body: Body::Render(json!({
                    "bosminer_uptime_s": self.uptime_s,
                    "platform": 8,
                    "mac_address": mac(name),
                    "miner_identity": { "miner_model": self.model_name.as_str() },
                    "sticker_hashrate": { "gigahash_per_second": leaf(steady(self.nominal_ths * 1_000.0)) },
                })),
                status: self.status,
            },
        ];
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: AnnounceSpec::Mdns {
                service_type: "_http._tcp".to_owned(),
                subtype: Some("_bos".to_owned()),
                txt: BTreeMap::new(),
            },
            endpoints,
            sampler: None,
        }
    }

    /// A hashboard whose chip temperature drifts around the baseline plus `offset_c`.
    fn board(&self, offset_c: f64) -> Json {
        json!({
            "highest_chip_temp": { "temperature": { "degree_c": leaf(celsius(self.temp_c + offset_c)) } },
            "chip_type": "BM1370",
            "chips_count": 76,
        })
    }
}
