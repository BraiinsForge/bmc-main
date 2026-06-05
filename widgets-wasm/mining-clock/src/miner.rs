// Copyright (C) 2026  Braiins Systems s.r.o.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "gauge model is consumed by the render path wired in a later step"
    )
)]

use bmc_wasm_sdk::ufmt;
use mining::gauge::TargetRange;

pub(crate) trait JsonLookup {
    #[cfg_attr(
        test,
        expect(
            dead_code,
            reason = "kept for parity with the shared lookup trait; the auth path consumes it later"
        )
    )]
    fn str(&self, path: &str) -> Option<String>;
    #[cfg_attr(
        test,
        expect(
            dead_code,
            reason = "kept for parity with the shared lookup trait; integer fields are consumed later"
        )
    )]
    fn i64(&self, path: &str) -> Option<i64>;
    fn f64(&self, path: &str) -> Option<f64>;
}

pub(crate) fn ths_from_ghs(value: f64) -> f64 {
    value / 1_000.0
}

pub(crate) const STALE_AFTER_MS: u32 = 15_000;
const POWER_FULL_SCALE_W: f64 = 100.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MinerData {
    pub(crate) hashrate_ths: Option<f64>,
    pub(crate) power_w: Option<f64>,
    pub(crate) nominal_hashrate_ths: Option<f64>,
    pub(crate) constraints: Constraints,
}

// Tuner min/default/max targets that anchor the two gauge rings: `hashrate`
// drives the outer ring and the shared state, `power` the inner ring. Each is
// `Some` only when the endpoint reports all three of its leaves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Constraints {
    pub(crate) hashrate: Option<TargetRange>,
    pub(crate) power: Option<TargetRange>,
}

pub(crate) fn parse_stats(json: &impl JsonLookup, data: &mut MinerData) {
    if let Some(ghs) = json.f64("/miner_stats/real_hashrate/last_1m/gigahash_per_second") {
        data.hashrate_ths = Some(ths_from_ghs(ghs));
    }
    if let Some(power) = json.f64("/power_stats/approximated_consumption/watt") {
        data.power_w = Some(power);
    }
}

pub(crate) fn parse_hashboards(json: &impl JsonLookup, data: &mut MinerData) {
    let mut sum_ghs = 0.0;
    let mut idx = 0;
    while hashboard_present(json, idx) {
        if let Some(nominal) = json
            .f64(&bmc_wasm_sdk::fmt!(
                "/hashboards/{idx}/stats/nominal_hashrate/gigahash_per_second"
            ))
            .filter(|nominal| *nominal > 0.0)
        {
            sum_ghs += nominal;
        }
        idx += 1;
    }
    data.nominal_hashrate_ths = if sum_ghs > 0.0 {
        Some(ths_from_ghs(sum_ghs))
    } else {
        None
    };
}

fn hashboard_present(json: &impl JsonLookup, idx: usize) -> bool {
    [
        "stats/nominal_hashrate/gigahash_per_second",
        "stats/real_hashrate/last_1m/gigahash_per_second",
        "board_temp/degree_c",
        "highest_chip_temp/temperature/degree_c",
    ]
    .iter()
    .any(|path| {
        json.f64(&bmc_wasm_sdk::fmt!("/hashboards/{idx}/{path}"))
            .is_some()
    })
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

#[expect(
    clippy::cast_possible_truncation,
    reason = "gauge fraction is a clamped 0..1 ratio that loses no meaningful precision in f32"
)]
pub(crate) fn hashrate_fraction(hashrate: Option<f64>, nominal: Option<f64>) -> f32 {
    match (hashrate, nominal) {
        (Some(hashrate), Some(nominal)) if nominal > 0.0 => {
            (hashrate / nominal).clamp(0.0, 1.0) as f32
        }
        _ => 0.0,
    }
}

#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "power gauge fraction is a clamped 0..1 ratio that loses no meaningful precision in f32"
)]
pub(crate) fn power_fraction(power_w: Option<f64>) -> f32 {
    match power_w {
        Some(power_w) => (power_w / POWER_FULL_SCALE_W).clamp(0.0, 1.0) as f32,
        None => 0.0,
    }
}

// Mining current ratio: real hashrate as a percent of nominal. Matches
// mining-info's `real / nominal * 100`, the input to the shared gauge state.
// Unavailable when either input is missing or nominal is non-positive.
#[must_use]
pub(crate) fn mcr_percent(hashrate: Option<f64>, nominal: Option<f64>) -> Option<f64> {
    match (hashrate, nominal) {
        (Some(hashrate), Some(nominal)) if nominal > 0.0 => Some(hashrate / nominal * 100.0),
        _ => None,
    }
}

pub(crate) fn is_stale(age_ms: u32) -> bool {
    age_ms >= STALE_AFTER_MS
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

    fn assert_fraction_eq(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_stats_hashrate_and_power() {
        let mut json = MapJson::default();
        json.floats.insert(
            "/miner_stats/real_hashrate/last_1m/gigahash_per_second",
            122_480.0,
        );
        json.floats
            .insert("/power_stats/approximated_consumption/watt", 41.0);
        let mut data = MinerData::default();
        parse_stats(&json, &mut data);
        assert_eq!(data.hashrate_ths, Some(122.48));
        assert_eq!(data.power_w, Some(41.0));
    }

    #[test]
    fn parses_hashboards_by_summing_nominal_hashrate() {
        let mut json = MapJson::default();
        json.floats.insert(
            "/hashboards/0/stats/nominal_hashrate/gigahash_per_second",
            100_000.0,
        );
        json.floats.insert(
            "/hashboards/1/stats/nominal_hashrate/gigahash_per_second",
            25_000.0,
        );
        let mut data = MinerData::default();
        parse_hashboards(&json, &mut data);
        assert_eq!(data.nominal_hashrate_ths, Some(125.0));
    }

    #[test]
    fn parses_hashboards_past_missing_nominal_field() {
        let mut json = MapJson::default();
        json.floats.insert(
            "/hashboards/0/stats/nominal_hashrate/gigahash_per_second",
            100_000.0,
        );
        json.floats
            .insert("/hashboards/1/board_temp/degree_c", 44.0);
        json.floats.insert(
            "/hashboards/2/stats/nominal_hashrate/gigahash_per_second",
            25_000.0,
        );
        let mut data = MinerData::default();
        parse_hashboards(&json, &mut data);
        assert_eq!(data.nominal_hashrate_ths, Some(125.0));
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
        json.floats
            .remove("/tuner_constraints/power_target/max/watt");
        let mut data = MinerData::default();
        parse_constraints(&json, &mut data);
        assert!(data.constraints.hashrate.is_some());
        assert_eq!(data.constraints.power, None);
    }

    #[test]
    fn hashrate_fraction_requires_positive_nominal_and_clamps() {
        assert_fraction_eq(hashrate_fraction(Some(50.0), None), 0.0);
        assert_fraction_eq(hashrate_fraction(Some(50.0), Some(0.0)), 0.0);
        assert_fraction_eq(hashrate_fraction(Some(50.0), Some(100.0)), 0.5);
        assert_fraction_eq(hashrate_fraction(Some(125.0), Some(100.0)), 1.0);
    }

    #[test]
    fn power_fraction_reaches_full_scale_at_100_watts() {
        assert_fraction_eq(power_fraction(None), 0.0);
        assert_fraction_eq(power_fraction(Some(-1.0)), 0.0);
        assert_fraction_eq(power_fraction(Some(50.0)), 0.5);
        assert_fraction_eq(power_fraction(Some(100.0)), 1.0);
        assert_fraction_eq(power_fraction(Some(125.0)), 1.0);
    }

    #[test]
    fn mcr_percent_is_real_over_nominal_times_100() {
        let mcr = mcr_percent(Some(130.0), Some(100.0)).expect("BUG: both inputs available");
        assert!((mcr - 130.0).abs() < 1e-9, "got {mcr}");
        assert_eq!(mcr_percent(Some(50.0), None), None);
        assert_eq!(mcr_percent(Some(50.0), Some(0.0)), None);
        assert_eq!(mcr_percent(None, Some(100.0)), None);
    }

    #[test]
    fn staleness_threshold_is_exclusive_below_and_stale_at_threshold() {
        assert!(!is_stale(STALE_AFTER_MS - 1));
        assert!(is_stale(STALE_AFTER_MS));
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum AuthState {
    #[default]
    NoToken,
    LoggingIn,
    Authenticated(String),
    // A login attempt completed and was rejected. Distinct from `LoggingIn`; the
    // login poll keeps retrying underneath.
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
