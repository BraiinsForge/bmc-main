// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_sdk::ufmt;

use crate::model::{Availability, MinerData, TemperatureRange};

pub(crate) trait JsonLookup {
    fn str(&self, path: &str) -> Option<String>;
    fn i64(&self, path: &str) -> Option<i64>;
    fn f64(&self, path: &str) -> Option<f64>;
}

pub(crate) fn ths_from_ghs(value: f64) -> f64 {
    value / 1_000.0
}

pub(crate) fn parse_details(json: &impl JsonLookup, data: &mut MinerData) {
    if let Some(uptime) = json
        .i64("/bosminer_uptime_s")
        .and_then(|v| u64::try_from(v).ok())
    {
        data.uptime_s = Availability::Available(uptime);
    }
}

pub(crate) fn parse_stats(json: &impl JsonLookup, data: &mut MinerData) {
    if let Some(ghs) = json.f64("/miner_stats/real_hashrate/last_1m/gigahash_per_second") {
        data.hashrate_ths = Availability::Available(ths_from_ghs(ghs));
    }
    if let Some(power) = json.f64("/power_stats/approximated_consumption/watt") {
        data.power_w = Availability::Available(power);
    }
}

pub(crate) fn parse_hashboards(json: &impl JsonLookup, data: &mut MinerData) {
    let board = json.f64("/hashboards/0/board_temp/degree_c");
    let chip = json.f64("/hashboards/0/highest_chip_temp/temperature/degree_c");
    if let (Some(board_c), Some(chip_c)) = (board, chip) {
        data.temperature = Availability::Available(TemperatureRange { board_c, chip_c });
    }
    let nominal = json.f64("/hashboards/0/stats/nominal_hashrate/gigahash_per_second");
    let real = json.f64("/hashboards/0/stats/real_hashrate/last_1m/gigahash_per_second");
    if let (Some(real), Some(nominal)) = (real, nominal)
        && nominal > 0.0
    {
        data.mcr_percent = Availability::Available(real / nominal * 100.0);
    }
}

pub(crate) fn parse_cooling(json: &impl JsonLookup, data: &mut MinerData) {
    if let Some(ratio) = json.f64("/fans/0/target_speed_ratio") {
        data.fan_percent = Availability::Available(ratio * 100.0);
    }
}

pub(crate) fn parse_network(json: &impl JsonLookup, data: &mut MinerData) {
    if let Some(ip) = json.str("/networks/0/address") {
        data.ip_address = Availability::Available(ip);
    }
}

#[cfg(target_arch = "wasm32")]
impl JsonLookup for bmc_wasm_sdk::json::JsonDoc {
    fn str(&self, path: &str) -> Option<String> {
        self.str(path)
    }

    fn i64(&self, path: &str) -> Option<i64> {
        self.i64(path)
    }

    fn f64(&self, path: &str) -> Option<f64> {
        self.f64(path)
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::JsonLookup;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub(crate) struct MapJson {
        pub(crate) strings: BTreeMap<&'static str, &'static str>,
        pub(crate) ints: BTreeMap<&'static str, i64>,
        pub(crate) floats: BTreeMap<&'static str, f64>,
    }

    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.strings.get(path).map(|s| (*s).to_owned())
        }

        fn i64(&self, path: &str) -> Option<i64> {
            self.ints.get(path).copied()
        }

        fn f64(&self, path: &str) -> Option<f64> {
            self.floats.get(path).copied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::MapJson;
    use super::*;

    #[test]
    fn converts_hashrate_from_ghs_to_ths() {
        assert!((ths_from_ghs(122_480.0) - 122.48).abs() < 1e-9);
    }

    #[test]
    fn parses_miner_details_uptime() {
        let mut json = MapJson::default();
        json.ints.insert("/bosminer_uptime_s", 187_020);
        let mut data = MinerData::default();
        parse_details(&json, &mut data);
        assert_eq!(data.uptime_s, Availability::Available(187_020));
    }

    #[test]
    fn parses_cooling_ratio_as_percent() {
        let mut json = MapJson::default();
        json.floats.insert("/fans/0/target_speed_ratio", 0.72);
        let mut data = MinerData::default();
        parse_cooling(&json, &mut data);
        let Availability::Available(percent) = data.fan_percent else {
            panic!("BUG: fan percent should be available");
        };
        assert!((percent - 72.0).abs() < 1e-9);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum AuthState {
    #[default]
    NoToken,
    LoggingIn,
    Authenticated(String),
    // A login attempt completed and was rejected. Distinct from `LoggingIn` so the
    // render path can surface the failure; the login poll keeps retrying underneath.
    Failed,
}

impl AuthState {
    pub(crate) fn token(&self) -> Option<&str> {
        match self {
            Self::Authenticated(token) => Some(token),
            Self::NoToken | Self::LoggingIn | Self::Failed => None,
        }
    }

    pub(crate) fn auth_header(&self) -> Option<String> {
        self.token()
            .map(|token| bmc_wasm_sdk::fmt!("Authorization: {token}"))
    }
}

pub(crate) fn endpoint(base: &str, path: &str) -> String {
    bmc_wasm_sdk::fmt!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

#[cfg(test)]
mod auth_tests {
    use super::*;

    #[test]
    fn joins_base_url_and_path_once() {
        assert_eq!(
            endpoint("http://miner/api/v1/", "/miner/stats"),
            "http://miner/api/v1/miner/stats"
        );
    }

    #[test]
    fn builds_bos_auth_header() {
        let mut auth = AuthState::default();
        assert_eq!(auth, AuthState::NoToken);
        assert_eq!(auth.auth_header(), None);
        assert_eq!(AuthState::LoggingIn.auth_header(), None);
        assert_eq!(AuthState::Failed.auth_header(), None);
        auth = AuthState::Authenticated("abc".to_owned());
        assert_eq!(auth.auth_header(), Some("Authorization: abc".to_owned()));
        auth = AuthState::NoToken;
        assert_eq!(auth.token(), None);
    }
}
