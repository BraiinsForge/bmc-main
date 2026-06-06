// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_sdk::ufmt;

use crate::model::{Availability, MinerData, TemperatureRange};
use mining::gauge::TargetRange;
use units::units::{DegreeCelsius, JoulePerTeraHash, Percent, Seconds, TeraHashPerSecond, Watt};

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
        data.uptime_s = Availability::Available(Seconds(uptime));
    }
}

pub(crate) fn parse_stats(json: &impl JsonLookup, data: &mut MinerData) {
    if let Some(ghs) = json.f64("/miner_stats/real_hashrate/last_1m/gigahash_per_second") {
        data.hashrate_ths = Availability::Available(TeraHashPerSecond(ths_from_ghs(ghs)));
    }
    if let Some(power) = json.f64("/power_stats/approximated_consumption/watt") {
        data.power_w = Availability::Available(Watt(power));
    }
    if let Some(efficiency) = json.f64("/power_stats/efficiency/joule_per_terahash") {
        data.efficiency_j_th = Availability::Available(JoulePerTeraHash(efficiency));
    }
}

pub(crate) fn parse_hashboards(json: &impl JsonLookup, data: &mut MinerData) {
    let board = json.f64("/hashboards/0/board_temp/degree_c");
    let chip = json.f64("/hashboards/0/highest_chip_temp/temperature/degree_c");
    if let (Some(board_c), Some(chip_c)) = (board, chip) {
        data.temperature = Availability::Available(TemperatureRange {
            board: DegreeCelsius(board_c),
            chip: DegreeCelsius(chip_c),
        });
    }
    let nominal = json.f64("/hashboards/0/stats/nominal_hashrate/gigahash_per_second");
    let real = json.f64("/hashboards/0/stats/real_hashrate/last_1m/gigahash_per_second");
    if let (Some(real), Some(nominal)) = (real, nominal)
        && nominal > 0.0
    {
        data.mcr_percent = Availability::Available(Percent(real / nominal * 100.0));
    }
}

pub(crate) fn parse_cooling(json: &impl JsonLookup, data: &mut MinerData) {
    if let Some(ratio) = json.f64("/fans/0/target_speed_ratio") {
        data.fan_percent = Availability::Available(Percent(ratio * 100.0));
    }
}

pub(crate) fn parse_network(json: &impl JsonLookup, data: &mut MinerData) {
    if let Some(ip) = json.str("/networks/0/address") {
        data.ip_address = Availability::Available(ip);
    }
}

pub(crate) fn parse_constraints(json: &impl JsonLookup, data: &mut MinerData) {
    data.constraints.hashrate = target_range(
        json,
        "/tuner_constraints/hashrate_target",
        "terahash_per_second",
    );
    data.constraints.power = target_range(json, "/tuner_constraints/power_target", "watt");
}

// Read a `{min,default,max}/<leaf>` target block, present only when all three
// edges are reported.
fn target_range(json: &impl JsonLookup, base: &str, leaf: &str) -> Option<TargetRange> {
    let edge = |name: &str| json.f64(&bmc_wasm_sdk::fmt!("{base}/{name}/{leaf}"));
    Some(TargetRange {
        min: edge("min")?,
        default: edge("default")?,
        max: edge("max")?,
    })
}

// A failed poll (network error or non-2xx) means the endpoint's fields are no
// longer trustworthy. Each `reset_*` clears exactly the fields its matching
// `parse_*` produces, so an unreachable miner shows unavailable instead of the
// last good reading, while a single flaky endpoint never wipes another's data.
pub(crate) fn reset_details(data: &mut MinerData) {
    data.uptime_s = Availability::Unavailable;
}

pub(crate) fn reset_stats(data: &mut MinerData) {
    data.hashrate_ths = Availability::Unavailable;
    data.power_w = Availability::Unavailable;
    data.efficiency_j_th = Availability::Unavailable;
}

pub(crate) fn reset_hashboards(data: &mut MinerData) {
    data.temperature = Availability::Unavailable;
    data.mcr_percent = Availability::Unavailable;
}

pub(crate) fn reset_cooling(data: &mut MinerData) {
    data.fan_percent = Availability::Unavailable;
}

pub(crate) fn reset_network(data: &mut MinerData) {
    data.ip_address = Availability::Unavailable;
}

pub(crate) fn reset_constraints(data: &mut MinerData) {
    data.constraints = crate::model::Constraints::default();
}

pub(crate) fn reset_all(data: &mut MinerData) {
    reset_details(data);
    reset_stats(data);
    reset_hashboards(data);
    reset_cooling(data);
    reset_network(data);
    reset_constraints(data);
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
    use units::units::Quantity;

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
        assert_eq!(data.uptime_s, Availability::Available(Seconds(187_020)));
    }

    #[test]
    fn parses_stats_efficiency_when_present() {
        let mut json = MapJson::default();
        json.floats
            .insert("/power_stats/efficiency/joule_per_terahash", 21.5);
        let mut data = MinerData::default();
        parse_stats(&json, &mut data);
        assert_eq!(
            data.efficiency_j_th,
            Availability::Available(JoulePerTeraHash(21.5))
        );
    }

    #[test]
    fn leaves_efficiency_unavailable_when_absent() {
        let json = MapJson::default();
        let mut data = MinerData::default();
        parse_stats(&json, &mut data);
        assert_eq!(data.efficiency_j_th, Availability::Unavailable);
    }

    #[test]
    fn reset_clears_only_its_own_fields() {
        let mut data = MinerData {
            hashrate_ths: Availability::Available(TeraHashPerSecond(4.0)),
            power_w: Availability::Available(Watt(120.0)),
            efficiency_j_th: Availability::Available(JoulePerTeraHash(21.5)),
            mcr_percent: Availability::Available(Percent(90.0)),
            fan_percent: Availability::Available(Percent(72.0)),
            ..MinerData::default()
        };
        reset_stats(&mut data);
        assert_eq!(data.hashrate_ths, Availability::Unavailable);
        assert_eq!(data.power_w, Availability::Unavailable);
        assert_eq!(data.efficiency_j_th, Availability::Unavailable);
        // Fields owned by other endpoints are untouched.
        assert_eq!(data.mcr_percent, Availability::Available(Percent(90.0)));
        assert_eq!(data.fan_percent, Availability::Available(Percent(72.0)));
    }

    #[test]
    fn reset_all_clears_stale_miner_values_after_auth_changes() {
        let mut data = MinerData {
            hashrate_ths: Availability::Available(TeraHashPerSecond(4.0)),
            temperature: Availability::Available(TemperatureRange {
                board: DegreeCelsius(61.0),
                chip: DegreeCelsius(74.0),
            }),
            power_w: Availability::Available(Watt(120.0)),
            efficiency_j_th: Availability::Available(JoulePerTeraHash(21.5)),
            mcr_percent: Availability::Available(Percent(90.0)),
            fan_percent: Availability::Available(Percent(72.0)),
            uptime_s: Availability::Available(Seconds(187_020)),
            ip_address: Availability::Available("192.168.1.42".to_owned()),
            constraints: crate::model::Constraints {
                hashrate: Some(TargetRange {
                    min: 50.0,
                    default: 100.0,
                    max: 120.0,
                }),
                power: None,
            },
        };
        reset_all(&mut data);
        assert_eq!(data, MinerData::default());
    }

    fn full_constraints_json() -> MapJson {
        let mut json = MapJson::default();
        json.floats.insert(
            "/tuner_constraints/hashrate_target/min/terahash_per_second",
            50.0,
        );
        json.floats.insert(
            "/tuner_constraints/hashrate_target/default/terahash_per_second",
            100.0,
        );
        json.floats.insert(
            "/tuner_constraints/hashrate_target/max/terahash_per_second",
            120.0,
        );
        json.floats
            .insert("/tuner_constraints/power_target/min/watt", 1_000.0);
        json.floats
            .insert("/tuner_constraints/power_target/default/watt", 3_000.0);
        json.floats
            .insert("/tuner_constraints/power_target/max/watt", 3_500.0);
        json
    }

    #[test]
    fn parses_tuner_constraints_for_hashrate_and_power() {
        let mut data = MinerData::default();
        parse_constraints(&full_constraints_json(), &mut data);
        assert_eq!(
            data.constraints.hashrate,
            Some(TargetRange {
                min: 50.0,
                default: 100.0,
                max: 120.0
            })
        );
        assert_eq!(
            data.constraints.power,
            Some(TargetRange {
                min: 1_000.0,
                default: 3_000.0,
                max: 3_500.0
            })
        );
    }

    #[test]
    fn constraint_target_is_absent_when_a_leaf_is_missing() {
        let mut json = full_constraints_json();
        // Drop one power leaf: power becomes None, hashrate stays whole.
        json.floats
            .remove("/tuner_constraints/power_target/max/watt");
        let mut data = MinerData::default();
        parse_constraints(&json, &mut data);
        assert!(data.constraints.hashrate.is_some());
        assert_eq!(data.constraints.power, None);
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
        assert!((percent.raw() - 72.0).abs() < 1e-9);
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
