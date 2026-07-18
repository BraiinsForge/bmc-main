// Copyright (C) 2026  Braiins Systems s.r.o.

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

/// Tunables for a simulated Braiins Forge Miner x4 (Braiins OS Libre).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default)]
#[schemars(rename = "UbosParams")]
pub struct Params {
    /// Product name reported on `/api/info`.
    pub model_name: String,
    /// Current hashrate to hover around, in TH/s.
    pub hashrate_ths: f64,
    /// Power draw to hover around, in W.
    pub power_w: f64,
    /// Temperature to hover around, in °C.
    pub temp_c: f64,
    /// Reported uptime, in seconds.
    pub uptime_s: u64,
    /// HTTP status the telemetry endpoint returns (503 = present, unreadable).
    pub status: u16,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            model_name: "Braiins Forge Miner x4".to_owned(),
            hashrate_ths: 4.8,
            power_w: 76.0,
            temp_c: 65.0,
            uptime_s: 187_020,
            status: 200,
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: AnnounceSpec::Mdns {
                service_type: "_ubos._tcp".to_owned(),
                subtype: None,
                txt: BTreeMap::new(),
            },
            endpoints: vec![EndpointSpec {
                method: "GET".to_owned(),
                path: "/api/info".to_owned(),
                body: Body::Render(json!({
                    "name": self.model_name.as_str(),
                    "hashrate": leaf(drift(self.hashrate_ths * 1e12)),
                    "power_out_mw": leaf(drift(self.power_w * 1_000.0)),
                    "temperature": leaf(celsius(self.temp_c)),
                    "uptime": self.uptime_s,
                })),
                status: self.status,
            }],
            sampler: None,
        }
    }
}
